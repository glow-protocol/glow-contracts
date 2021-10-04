use crate::error::ContractError;
use crate::querier::{query_exchange_rate, query_oracle};
use crate::state::{
    read_lottery_info, store_lottery_info, LotteryInfo, PrizeInfo, CONFIG, POOL, PRIZES, STATE,
    TICKETS,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, to_binary, CosmosMsg, DepsMut, Env, MessageInfo, Order, Response, StdResult, WasmMsg,
};
use cw0::Expiration;
use cw20::Cw20ExecuteMsg::Send as Cw20Send;
use cw_storage_plus::{Bound, U64Key};
use terraswap::querier::query_token_balance;

use crate::helpers::{compute_reward, count_seq_matches};
use crate::oracle::{calculate_lottery_rand_round, sequence_from_hash};
use moneymarket::market::Cw20HookMsg;
use std::ops::{Add, Sub};
use std::str;
use std::usize;

pub fn execute_lottery(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;

    // Compute global Glow rewards
    compute_reward(&mut state, &pool, env.block.height);

    // No sent funds allowed when executing the lottery
    if !info.funds.is_empty() {
        return Err(ContractError::InvalidLotteryExecution {});
    }

    if !state.next_lottery_time.is_expired(&env.block) {
        return Err(ContractError::LotteryInProgress {});
    }

    if state.total_tickets.is_zero() {
        return Err(ContractError::InvalidLotteryExecution {});
    }

    state.next_lottery_exec_time = Expiration::AtTime(env.block.time).add(config.block_time)?;

    let lottery_rand_round = calculate_lottery_rand_round(env.clone(), config.round_delta);
    let lottery_info = LotteryInfo {
        rand_round: lottery_rand_round,
        sequence: "".to_string(),
        awarded: false,
        total_prizes: Decimal256::zero(),
        number_winners: [0; 6],
        page: "".to_string(),
    };

    store_lottery_info(deps.storage, state.current_lottery, &lottery_info)?;

    // Get pooled lottery deposits in Anchor
    let aust_balance = query_token_balance(
        &deps.querier,
        deps.api.addr_validate(config.a_terra_contract.as_str())?,
        env.contract.address,
    )?;

    let aust_lottery_balance = Uint256::from(aust_balance).multiply_ratio(
        (pool.lottery_shares + pool.sponsor_shares) * Uint256::one(),
        (pool.deposit_shares + pool.lottery_shares + pool.sponsor_shares) * Uint256::one(),
    );
    let rate =
        query_exchange_rate(deps.as_ref(), config.anchor_contract.to_string())?.exchange_rate;

    let pooled_lottery_deposits = Decimal256::from_uint256(aust_lottery_balance) * rate;

    let mut msgs: Vec<CosmosMsg> = vec![];
    // Redeem funds if lottery related shares are greater than outstanding lottery deposits
    let mut aust_to_redeem = Decimal256::zero();
    if (pool.lottery_deposits + pool.total_sponsor_amount) >= pooled_lottery_deposits {
        if state.award_available.is_zero() {
            return Err(ContractError::InsufficientLotteryFunds {});
        }
    } else {
        let amount_to_redeem =
            pooled_lottery_deposits - pool.lottery_deposits - pool.total_sponsor_amount;
        aust_to_redeem = amount_to_redeem / rate;
        state.award_available += amount_to_redeem;

        // Message for redeem amount operation of aUST
        let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.a_terra_contract.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20Send {
                contract: config.anchor_contract.to_string(),
                amount: (aust_to_redeem * Uint256::one()).into(),
                msg: to_binary(&Cw20HookMsg::RedeemStable {})?,
            })?,
        });
        msgs.push(redeem_msg);
    }

    // Store state
    STATE.save(deps.storage, &state)?;

    let res = Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "execute_lottery"),
        attr(
            "redeemed_amount",
            (aust_to_redeem * Uint256::one()).to_string(),
        ),
    ]);
    Ok(res)
}

fn calc_limit(request: Option<u32>) -> usize {
    request.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize
}

const MAX_LIMIT: u32 = 120;
const DEFAULT_LIMIT: u32 = 50;

pub fn execute_prize(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    limit: Option<u32>,
) -> Result<Response, ContractError> {
    let mut state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;

    // Compute global Glow rewards
    compute_reward(&mut state, &pool, env.block.height);

    // No sent funds allowed when executing the lottery
    if !info.funds.is_empty() {
        return Err(ContractError::InvalidLotteryExecution {});
    }

    // Execute lottery must be called before execute_prize
    let mut lottery_info = read_lottery_info(deps.storage, state.current_lottery);
    let current_lottery = state.current_lottery;

    if lottery_info.rand_round == 0 || !state.next_lottery_exec_time.is_expired(&env.block) {
        return Err(ContractError::InvalidLotteryPrizeExecution {});
    }

    // If first time called in current lottery, get winning sequence
    if lottery_info.sequence.is_empty() {
        let oracle_response = query_oracle(
            deps.as_ref(),
            config.oracle_contract.into_string(),
            lottery_info.rand_round,
        )?;
        let random_hash = hex::encode(oracle_response.random.as_slice());
        lottery_info.sequence = sequence_from_hash(random_hash);
    }

    // Calculate pagination bounds
    let limit = calc_limit(limit);

    let min_bound: &str = if lottery_info.page.is_empty() {
        &lottery_info.sequence[..2]
    } else {
        &lottery_info.page
    };

    // Get max bounds
    let max_bound_number = min_bound[..2].parse::<i32>().unwrap() + 1;
    let max_bound: String = if max_bound_number < 10 {
        format!("{}{}", 0, max_bound_number)
    } else if max_bound_number == 100 {
        format!("{}", max_bound_number - 1)
    } else {
        format!("{}", max_bound_number)
    };

    // Get winning tickets
    let winning_tickets: Vec<_> = TICKETS
        .range(
            deps.storage,
            Some(Bound::Inclusive(Vec::from(min_bound))),
            Some(Bound::Exclusive(Vec::from(max_bound.as_str()))),
            Order::Ascending,
        )
        .take(limit)
        .collect::<StdResult<Vec<_>>>()
        .unwrap();

    if !winning_tickets.is_empty() {
        // Update pagination for next iterations, if necessary
        if let Some(next) = TICKETS
            .range(
                deps.storage,
                Some(Bound::Exclusive(winning_tickets.last().unwrap().clone().0)),
                Some(Bound::Exclusive(Vec::from(max_bound))),
                Order::Ascending,
            )
            .next()
        {
            lottery_info.page = String::from_utf8(next.unwrap().0).unwrap();
        } else {
            lottery_info.awarded = true;
        }

        // Update holders prizes and lottery info number of winners
        winning_tickets.iter().for_each(|sequence| {
            let matches = count_seq_matches(
                &lottery_info.sequence.clone(),
                str::from_utf8(&*sequence.0).unwrap(),
            );
            lottery_info.number_winners[matches as usize] += sequence.1.len() as u32;
            sequence.1.iter().for_each(|winner| {
                let lottery_id: U64Key = state.current_lottery.into();
                PRIZES
                    .update(deps.storage, (winner, lottery_id), |hits| -> StdResult<_> {
                        let result = match hits {
                            Some(mut prize) => {
                                prize.matches[matches as usize] += 1;
                                prize
                            }
                            None => {
                                let mut winnings = [0; 6];
                                winnings[matches as usize] = 1;
                                PrizeInfo {
                                    claimed: false,
                                    matches: winnings,
                                }
                            }
                        };
                        Ok(result)
                    })
                    .unwrap();
            });
        });
    } else {
        lottery_info.awarded = true;
    }

    // If all winners have been accounted, update lottery info and jump to next round
    let mut total_awarded_prize = Decimal256::zero();
    if lottery_info.awarded {
        for (index, rank) in lottery_info.number_winners.iter().enumerate() {
            if *rank != 0 {
                total_awarded_prize += state.award_available * config.prize_distribution[index];
            }
        }
        lottery_info.total_prizes = total_awarded_prize;
        state.current_lottery += 1;

        state.next_lottery_time =
            Expiration::AtTime(env.block.time).add(config.lottery_interval)?;
        state.award_available = state.award_available.sub(total_awarded_prize);
        STATE.save(deps.storage, &state)?;
    }
    store_lottery_info(deps.storage, current_lottery, &lottery_info)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "execute_prize"),
        attr("total_awarded_prize", total_awarded_prize.to_string()),
    ]))
}

use crate::error::ContractError;
use crate::querier::query_exchange_rate;
use crate::state::{
    read_depositor_info, read_lottery_info, store_depositor_info, store_lottery_info, LotteryInfo,
    PrizeInfo, CONFIG, PRIZES, STATE, TICKETS, POOL
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, to_binary, Addr, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Order, Response, StdResult,
    Storage, Uint128, WasmMsg,
};
use cw0::Expiration;
use cw20::Cw20ExecuteMsg::Send as Cw20Send;
use cw_storage_plus::{Bound, U64Key};
use terraswap::querier::query_token_balance;

use crate::helpers::compute_reward;
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

    // TODO: Get random sequence here
    let winning_sequence = String::from("00000");

    let lottery_info = LotteryInfo {
        sequence: winning_sequence,
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
        env.clone().contract.address,
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

    // TODO: add msg to drand worker fee

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
    compute_reward(&mut state, &pool,env.block.height);

    // No sent funds allowed when executing the lottery
    if !info.funds.is_empty() {
        return Err(ContractError::InvalidLotteryExecution {});
    }

    // Execute lottery must be called before execute_prize
    let mut lottery_info = read_lottery_info(deps.storage, state.current_lottery);
    let current_lottery = state.current_lottery;
    if lottery_info.sequence.is_empty() {
        return Err(ContractError::InvalidLotteryPrizeExecution {});
    }

    // Calculate pagination bounds
    let limit = calc_limit(limit);

    let mut min_bound = "";
    if lottery_info.page.is_empty() {
        min_bound = &lottery_info.sequence[..2];
    } else {
        min_bound = &lottery_info.page;
    }

    // Get max bounds
    let max_bound_number = min_bound.parse::<i32>().unwrap() + 1;
    let mut max_bound = String::new();
    if max_bound_number < 10 {
        max_bound = format!("{}{}", 0, max_bound_number);
    } else if max_bound_number == 100 {
        format!("{}", max_bound_number - 1);
    } else {
        max_bound = format!("{}", max_bound_number);
    }

    // Get winning tickets
    let winning_tickets: Vec<_> = TICKETS
        .range(
            deps.storage,
            Some(Bound::Inclusive(Vec::from(min_bound))),
            Some(Bound::Exclusive(Vec::from(max_bound.as_str()))),
            Order::Ascending,
        )
        .take(limit)
        .collect::<StdResult<(Vec<_>)>>()
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
        winning_tickets.iter().for_each(|sequence| -> () {
            let matches = count_seq_matches(
                &lottery_info.sequence.clone(),
                str::from_utf8(&*sequence.0).unwrap(),
            );
            lottery_info.number_winners[matches as usize] += sequence.1.len() as u32;
            sequence.1.iter().for_each(|winner| {
                let lottery_id: U64Key = state.current_lottery.into();
                // TODO: revisit to avoid multiple state r-w
                PRIZES.update(deps.storage, (winner, lottery_id), |hits| -> StdResult<_> {
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
                });
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

fn apply_reserve_factor(awarded_amount: Decimal256, reserve_factor: Decimal256) -> Decimal256 {
    awarded_amount * reserve_factor
}

// distribution is a vector e.g. [0, 0, 0.025, 0.15, 0.3, 0.5]
fn assign_prize(
    awardable_prize: Decimal256,
    matches: u8,
    winners: u64,
    distribution: &[Decimal256],
) -> Decimal256 {
    let number_winners = Uint256::from(winners as u64);

    awardable_prize * distribution[matches as usize] / Decimal256::from_uint256(number_winners)
}

pub fn is_valid_sequence(sequence: &str, len: u8) -> bool {
    sequence.len() == (len as usize) && sequence.chars().all(|c| ('0'..='9').contains(&c))
}

pub fn count_seq_matches(a: &str, b: &str) -> u8 {
    let mut count = 0;
    for (i, c) in a.chars().enumerate() {
        if c == b.chars().nth(i).unwrap() {
            count += 1;
        } else {
            break;
        }
    }
    count
}

pub fn assert_holder(
    storage: &dyn Storage,
    combination: &String,
    holder: Addr,
    max_holders: u8,
) -> Result<(), ContractError> {
    if let Some(holders) = TICKETS.may_load(storage, combination.as_bytes()).unwrap() {
        if holders.contains(&holder) {
            return Err(ContractError::InvalidHolderSequence {});
        }

        if holders.len() > max_holders as usize {
            return Err(ContractError::InvalidHolderSequence {});
        }
    }

    Ok(())
}

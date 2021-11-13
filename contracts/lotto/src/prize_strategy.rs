use crate::error::ContractError;
use crate::querier::{query_exchange_rate, query_oracle};
use crate::state::{
    read_lottery_info, store_lottery_info, LotteryInfo, PrizeInfo, CONFIG, POOL, PRIZES, STATE,
    TICKETS,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, coin, to_binary, CosmosMsg, DepsMut, Env, MessageInfo, Order, Response, StdResult,
    Timestamp, WasmMsg,
};
use cw0::{Duration, Expiration};
use cw20::Cw20ExecuteMsg::Send as Cw20Send;
use cw_storage_plus::{Bound, U64Key};
use terraswap::querier::query_token_balance;

use crate::helpers::{calculate_max_bound, compute_reward, count_seq_matches};
use crate::oracle::{calculate_lottery_rand_round, sequence_from_hash};
use glow_protocol::querier::deduct_tax;
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

    // Verify that the next_lottery_time is expired
    if !state.next_lottery_time.is_expired(&env.block) {
        return Err(ContractError::LotteryInProgress {});
    }

    // Verify that there is at least one registered ticket
    if state.total_tickets.is_zero() {
        return Err(ContractError::InvalidLotteryExecution {});
    }

    // Set the next_lottery_exec_time to one block after the current block
    // What is the point of this? Why do you need to wait a block before executing the next lottery?
    state.next_lottery_exec_time = Expiration::AtTime(env.block.time).add(config.block_time)?;

    // check execute_lottery has not been called already
    let mut lottery_info = read_lottery_info(deps.storage, state.current_lottery);
    if lottery_info.rand_round != 0 {
        return Err(ContractError::InvalidLotteryExecution {});
    }

    let lottery_rand_round = calculate_lottery_rand_round(env.clone(), config.round_delta);
    lottery_info = LotteryInfo {
        rand_round: lottery_rand_round,
        sequence: "".to_string(),
        awarded: false,
        timestamp: env.block.height,
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
    let rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;

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

        //Discount tx taxes Anchor -> Glow
        let net_amount = deduct_tax(
            deps.as_ref(),
            coin(
                (amount_to_redeem * Uint256::one()).into(),
                config.clone().stable_denom,
            ),
        )?
        .amount;

        state.award_available += Decimal256::from_uint256(Uint256::from(net_amount));

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
        let random_hash = hex::encode(oracle_response.randomness.as_slice());
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
    let max_bound = calculate_max_bound(min_bound);

    // Get winning tickets
    let winning_tickets: Vec<_> = TICKETS
        .range(
            deps.storage,
            Some(Bound::Inclusive(Vec::from(min_bound))),
            Some(Bound::Inclusive(Vec::from(max_bound.clone()))),
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
                Some(Bound::Inclusive(Vec::from(max_bound))),
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

        // set next_lottery_time to the current lottery time plus the lottery interval
        // want next_lottery_time to be a time in the future
        // so next_lottery_time = next_lottery_time + x * lottery_interval
        // but next_lottery_time + x * lottery_interval > env.block_time

        // get the amount of time between now and the time at which the lottery
        // became runnable
        let time_since_last_lottery = match state.next_lottery_time {
            Expiration::AtHeight(height) => env.block.time.minus_seconds(height),
            _ => Timestamp::from_seconds(0),
        };

        // get the lottery interval in seconds
        let lottery_interval_seconds = match config.lottery_interval {
            Duration::Time(time) => time,
            _ => 0,
        };

        // get the number of lottery intervals that have passed
        // since the lottery became runnable
        // this should be 0 everytime
        // unless somebody forgot to run the lottery for a week for example
        let lottery_intervals_since_last_lottery =
            time_since_last_lottery.seconds() / lottery_interval_seconds;

        // set the next_lottery_time to the closest time in the future that is
        // the current value of next_lottery_time plus a multiple of lottery_interval
        // normally this multiple will be 1 everytime
        // but if somebody forgot to run the lottery for a week, it will be 2 for example
        state.next_lottery_time = state
            .next_lottery_time
            .add(config.lottery_interval * (1 + lottery_intervals_since_last_lottery))?;

        state.next_lottery_exec_time = Expiration::Never {};
        state.award_available = state.award_available.sub(total_awarded_prize);
        STATE.save(deps.storage, &state)?;
    }
    store_lottery_info(deps.storage, current_lottery, &lottery_info)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "execute_prize"),
        attr("total_awarded_prize", total_awarded_prize.to_string()),
    ]))
}

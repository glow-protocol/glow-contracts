use crate::error::ContractError;
use crate::querier::{query_exchange_rate, query_oracle};

use crate::state::{
    read_lottery_info, store_lottery_info, LotteryInfo, PrizeInfo, CONFIG, POOL, PRIZES, STATE,
    TICKETS,
};
use cosmwasm_bignumber::Uint256;
use cosmwasm_std::{
    attr, coin, to_binary, CosmosMsg, DepsMut, Env, MessageInfo, Order, Response, StdResult,
    WasmMsg,
};
use cw0::{Duration, Expiration};
use cw20::Cw20ExecuteMsg::Send as Cw20Send;
use cw_storage_plus::{Bound, U64Key};
use glow_protocol::lotto::NUM_PRIZE_BUCKETS;
use terraswap::querier::query_token_balance;

use crate::helpers::{calculate_max_bound, compute_reward, count_seq_matches};
use crate::oracle::{calculate_lottery_rand_round, sequence_from_hash};
use glow_protocol::querier::deduct_tax;
use moneymarket::market::Cw20HookMsg;
use std::ops::Add;
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

    // Get the contract's aust balance
    let contract_a_balance = query_token_balance(
        &deps.querier,
        deps.api.addr_validate(config.a_terra_contract.as_str())?,
        env.clone().contract.address,
    )?;

    // Get the aust exchange rate
    let rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;

    // Validate that no funds are sent when executing the lottery
    if !info.funds.is_empty() {
        return Err(ContractError::InvalidLotteryExecutionFunds {});
    }

    // Validate that the next_lottery_time has passed
    if !state.next_lottery_time.is_expired(&env.block) {
        return Err(ContractError::LotteryNotReady {
            next_lottery_time: state.next_lottery_time,
        });
    }

    // Validate that there are a non zero number of tickets
    if state.total_tickets.is_zero() {
        return Err(ContractError::InvalidLotteryExecutionTickets {});
    }

    // Validate that the value of the contract's lottery aust is always at least the
    // sum of the value of the user savings aust and lottery deposits.
    // This check should never fail but is in place as an extra safety measure.
    let lottery_pool_value =
        (Uint256::from(contract_a_balance) - pool.total_user_savings_aust) * rate;

    if lottery_pool_value < (pool.total_user_lottery_deposits + pool.total_sponsor_lottery_deposits)
    {
        return Err(ContractError::InsufficientPoolFunds {
            pool_value: lottery_pool_value,
            total_lottery_deposits: pool.total_user_lottery_deposits
                + pool.total_sponsor_lottery_deposits,
        });
    }

    // Compute global Glow rewards
    compute_reward(&mut state, &pool, env.block.height);

    // Set the next_lottery_exec_time to the current block time plus `config.block_time`
    // This is so that `execute_prize` can't be run until the randomness oracle is ready
    // with the rand_round calculated below
    state.next_lottery_exec_time = Expiration::AtTime(env.block.time).add(config.block_time)?;

    // Validate that the lottery hasn't already started
    let mut lottery_info = read_lottery_info(deps.storage, state.current_lottery);
    if lottery_info.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    // Get the lottery_rand_round
    let lottery_rand_round = calculate_lottery_rand_round(env.clone(), config.round_delta);

    // Populate lottery_info
    lottery_info = LotteryInfo {
        rand_round: lottery_rand_round,
        sequence: "".to_string(),
        awarded: false,
        timestamp: env.block.height,
        prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
        number_winners: [0; NUM_PRIZE_BUCKETS],
        page: "".to_string(),
    };
    store_lottery_info(deps.storage, state.current_lottery, &lottery_info)?;

    // Lottery balance equals aust_balance - total_user_savings_aust
    let aust_lottery_balance = Uint256::from(contract_a_balance) - pool.total_user_savings_aust;

    // Get the ust value of the aust going towards the lottery
    let aust_lottery_balance_value = aust_lottery_balance * rate;

    let mut msgs: Vec<CosmosMsg> = vec![];

    // total_user_lottery_deposits plus total_sponsor_lottery_deposits gives the total ust value deposited into the lottery pool according to the calculations from the deposit function.
    // aust_lottery_balance_value gives the total ust value of the aust portion of the contract's balance

    // The value to redeem is the difference between the value of the appreciated lottery aust
    // and the total ust amount that has been deposited towards the lottery.
    let amount_to_redeem = aust_lottery_balance_value
        - pool.total_user_lottery_deposits
        - pool.total_sponsor_lottery_deposits;

    // Divide by the rate to get the number of aust to redeem
    let aust_to_redeem = amount_to_redeem / rate;

    // Get the value of the aust that will be redeemed
    let aust_to_redeem_value = aust_to_redeem * rate;

    // Get the amount of ust that will be received after accounting for taxes
    let net_amount = Uint256::from(
        deduct_tax(
            deps.as_ref(),
            coin(aust_to_redeem_value.into(), config.clone().stable_denom),
        )?
        .amount,
    );

    if net_amount.is_zero() {
        // If aust_to_redeem and award_available are zero, return error
        return Err(ContractError::InsufficientLotteryFunds {});
    }

    for (index, fraction_of_prize) in config.prize_distribution.iter().enumerate() {
        // Add the proportional amount of the net redeemed amount to the relevant award bucket.
        state.prize_buckets[index] += net_amount * *fraction_of_prize
    }

    // Message to redeem "aust_to_redeem" of aust from the Anchor contract
    let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.a_terra_contract.to_string(),
        funds: vec![],
        msg: to_binary(&Cw20Send {
            contract: config.anchor_contract.to_string(),
            amount: aust_to_redeem.into(),
            msg: to_binary(&Cw20HookMsg::RedeemStable {})?,
        })?,
    });
    msgs.push(redeem_msg);

    // Store the state
    STATE.save(deps.storage, &state)?;

    let res = Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "execute_lottery"),
        attr("redeemed_amount", aust_to_redeem.to_string()),
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

    let mut lottery_info = read_lottery_info(deps.storage, state.current_lottery);
    let current_lottery = state.current_lottery;

    // Compute global Glow rewards
    compute_reward(&mut state, &pool, env.block.height);

    // Validate that no funds are sent when executing the prize distribution
    if !info.funds.is_empty() {
        return Err(ContractError::InvalidLotteryPrizeExecutionFunds {});
    }

    // Validate that rand_round has been assigned
    if lottery_info.rand_round == 0 {
        return Err(ContractError::InvalidLotteryPrizeExecution {});
    }

    // Validate that the next_lottery_exec_time has passed
    if !state.next_lottery_exec_time.is_expired(&env.block) {
        return Err(ContractError::InvalidLotteryPrizeExecutionExpired {});
    }

    // If first time called in current lottery, generate the random winning sequence
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

    // Min bound is either the string of the first two characters of the winning sequence
    // or the page specified by lottery_info
    let min_bound: &str = if lottery_info.page.is_empty() {
        &lottery_info.sequence[..2]
    } else {
        &lottery_info.page
    };

    // Get max bounds
    let max_bound = calculate_max_bound(min_bound);

    // Get winning tickets
    let winning_tickets: Vec<_> = TICKETS
        // Get tickets inclusive from the min_bound to the max_bound with a limit
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
            // Set the page to the next value after the last winning_ticket from the previous limited query
            lottery_info.page = String::from_utf8(next.unwrap().0).unwrap();
        } else {
            lottery_info.awarded = true;
        }

        // Update holders prizes and lottery info number of winners
        winning_tickets.iter().for_each(|sequence| {
            // Get the number of matches between this winning ticket and the perfect winning ticket.
            let matches = count_seq_matches(
                &lottery_info.sequence.clone(),
                str::from_utf8(&*sequence.0).unwrap(),
            );
            // Increment the number of winners corresponding the number of matches of this ticket
            // by the number of people who hold this ticket.
            lottery_info.number_winners[matches as usize] += sequence.1.len() as u32;

            sequence.1.iter().for_each(|winner| {
                // Get the lottery_id
                let lottery_id: U64Key = state.current_lottery.into();

                // Update the prizes to show that the winner has a winning ticket
                PRIZES
                    .update(deps.storage, (winner, lottery_id), |hits| -> StdResult<_> {
                        let result = match hits {
                            Some(mut prize) => {
                                prize.matches[matches as usize] += 1;
                                prize
                            }
                            None => {
                                let mut winnings = [0; NUM_PRIZE_BUCKETS];
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
        // If there are no more winning tickets, then set awarded to true
        lottery_info.awarded = true;
    }

    // If all winners have been accounted, update lottery info and jump to next round
    let mut total_awarded_prize = Uint256::zero();
    if lottery_info.awarded {
        // Update the lottery prize buckets based on whether or not there is a winner in the corresponding bucket
        for (index, rank) in lottery_info.number_winners.iter().enumerate() {
            if *rank != 0 {
                // Increase total_awarded_prize
                total_awarded_prize += state.prize_buckets[index];

                // Update the corresponding lottery prize bucket
                lottery_info.prize_buckets[index] = state.prize_buckets[index];

                // Set the corresponding award bucket to 0
                state.prize_buckets[index] = Uint256::zero();
            }
        }

        // Increment the current_lottery_number
        state.current_lottery += 1;

        // Set next_lottery_time to the current lottery time plus the lottery interval
        // We want next_lottery_time to be a time in the future so pick the smallest x such that
        // next_lottery_time = next_lottery_time + x * lottery_interval
        // but next_lottery_time + x * lottery_interval > env.block_time

        // Get the amount of time between now and the time at which the lottery
        // became runnable
        let time_since_next_lottery_time =
            if let Expiration::AtTime(next_lottery_time) = state.next_lottery_time {
                env.block.time.minus_seconds(next_lottery_time.seconds())
            } else {
                return Err(ContractError::InvalidLotteryNextTime {});
            };

        // Get the lottery interval in seconds
        let lottery_interval_seconds = if let Duration::Time(time) = config.lottery_interval {
            time
        } else {
            return Err(ContractError::InvalidLotteryInterval {});
        };

        // Get the number of lottery intervals that have passed
        // since the lottery became runnable
        // this should be 0 everytime
        // unless somebody forgot to run the lottery for a week for example
        let lottery_intervals_since_last_lottery =
            time_since_next_lottery_time.seconds() / lottery_interval_seconds;

        // Set the next_lottery_time to the closest time in the future that is
        // the current value of next_lottery_time plus a multiple of lottery_interval
        // normally this multiple will be 1 everytime
        // but if somebody forgot to run the lottery for a week, it will be 2 for example
        state.next_lottery_time = state
            .next_lottery_time
            .add(config.lottery_interval * (1 + lottery_intervals_since_last_lottery))?;

        // Set next_lottery_exec_time to never
        state.next_lottery_exec_time = Expiration::Never {};

        // Save the state
        STATE.save(deps.storage, &state)?;
    }

    // Save the lottery_info
    store_lottery_info(deps.storage, current_lottery, &lottery_info)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "execute_prize"),
        attr("total_awarded_prize", total_awarded_prize.to_string()),
    ]))
}

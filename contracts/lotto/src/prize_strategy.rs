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
use cw0::Expiration;
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

    // Validate that no funds are sent when executing the lottery
    if !info.funds.is_empty() {
        return Err(ContractError::InvalidLotteryExecutionFunds {});
    }

    // Validate that the next_lottery_time has passed
    if !state.next_lottery_time.is_expired(&env.block) {
        return Err(ContractError::LotteryNotReady {});
    }

    // Validate that there are a non zero number of tickets
    if state.total_tickets.is_zero() {
        return Err(ContractError::InvalidLotteryExecutionTickets {});
    }

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
        total_prizes: Uint256::zero(),
        number_winners: [0; 6],
        page: "".to_string(),
    };
    store_lottery_info(deps.storage, state.current_lottery, &lottery_info)?;

    // Get this contracts aust balance
    let aust_balance = query_token_balance(
        &deps.querier,
        deps.api.addr_validate(config.a_terra_contract.as_str())?,
        env.clone().contract.address,
    )?;

    // Get the number of shares that are dedicated to the lottery
    // by multiplying the total number of shares by the fraction of shares dedicated to the lottery
    let aust_lottery_balance = Uint256::from(aust_balance).multiply_ratio(
        pool.lottery_shares + pool.sponsor_shares,
        pool.deposit_shares + pool.lottery_shares + pool.sponsor_shares,
    );

    // Get the aust exchange rate
    let rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;

    // Get the ust value of the aust going towards the lottery
    let pooled_lottery_deposits = aust_lottery_balance * rate;

    let mut msgs: Vec<CosmosMsg> = vec![];

    let mut aust_to_redeem = Uint256::zero();

    // Lottery deposits plus sponsor amount gives the total ust value deposited into the lottery pool according to the calculations from the deposit function.
    // pooled_lottery_deposits gives the total ust value of the lottery pool according to the fraction of the aust owned by the contract.

    // pooled_lottery_deposits should always be greater than or equal to the pool.lottery_deposits + pool.total_sponsor_amount so this is more of a double check
    if (pool.lottery_deposits + pool.total_sponsor_amount) >= pooled_lottery_deposits {
        if state.award_available.is_zero() {
            // If lottery related shares have a smaller value than the amount of lottery deposits and award_available is zero
            // Return InsufficientLotteryFunds
            return Err(ContractError::InsufficientLotteryFunds {});
        }
    } else {
        // The value to redeem is the difference between the value of the appreciated lottery aust shares
        // and the total ust amount that has been deposited towards the lottery.
        let amount_to_redeem =
            pooled_lottery_deposits - pool.lottery_deposits - pool.total_sponsor_amount;

        // Divide by the rate to get the number of shares to redeem
        aust_to_redeem = amount_to_redeem / rate;

        // Get the value of the aust that will be redeemed
        let aust_to_redeem_value = aust_to_redeem * rate;

        // Get the amount of ust that will be received after accounting for taxes
        let net_amount = deduct_tax(
            deps.as_ref(),
            coin(aust_to_redeem_value.into(), config.clone().stable_denom),
        )?
        .amount;

        if aust_to_redeem.is_zero() {
            if state.award_available.is_zero() {
                // If aust_to_redeem and award_available are zero, return InsufficientLotteryFunds
                return Err(ContractError::InsufficientLotteryFunds {});
            }
        } else {
            // Add the net redeemed amount to the award available.
            state.award_available += Uint256::from(net_amount);

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
        }
    }

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
        // If there are no more winning tickets, then set awarded to true
        lottery_info.awarded = true;
    }

    // If all winners have been accounted, update lottery info and jump to next round
    let mut total_awarded_prize = Uint256::zero();
    if lottery_info.awarded {
        // Calculate the total_awarded_prize from the number_winners array
        for (index, rank) in lottery_info.number_winners.iter().enumerate() {
            if *rank != 0 {
                // increase total_awarded_prize
                total_awarded_prize += state.award_available * config.prize_distribution[index];
            }
        }
        // Save the total_prizes
        lottery_info.total_prizes = total_awarded_prize;

        // Increment the current_lottery_number
        state.current_lottery += 1;

        // Set the next_lottery_time and next_lottery_exec_time
        state.next_lottery_time =
            Expiration::AtTime(env.block.time).add(config.lottery_interval)?;
        state.next_lottery_exec_time = Expiration::Never {};

        // Subtract the awarded prize from the award_available to get the remaining award_available
        state.award_available = state.award_available.sub(total_awarded_prize);

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

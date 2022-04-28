use std::convert::TryInto;
use std::ops::Add;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, BlockInfo, Deps, DepsMut, Env, StdError, StdResult, Uint128};
use glow_protocol::lotto::{
    AmountRedeemableForPrizesInfo, DepositorInfo, RewardEmissionsIndex, TICKET_LENGTH,
};
use sha3::{Digest, Keccak256};

use crate::error::ContractError;

use crate::querier::query_prize_distribution_pending;
use crate::state::{
    read_operator_info, store_operator_info, Config, OldDepositorInfo, OldPool, OldState,
    OperatorInfo, Pool, SponsorInfo, State, TICKETS,
};

/// Compute distributed reward and update global reward index for operators
pub fn compute_global_operator_reward(state: &mut State, pool: &Pool, block_height: u64) {
    compute_global_reward(
        &mut state.operator_reward_emission_index,
        pool.total_operator_shares,
        block_height,
    );
}

/// Compute distributed reward and update global reward index for sponsors
pub fn compute_global_sponsor_reward(state: &mut State, pool: &Pool, block_height: u64) {
    compute_global_reward(
        &mut state.sponsor_reward_emission_index,
        pool.total_sponsor_lottery_deposits,
        block_height,
    );
}

/// Compute distributed reward and update global reward index
pub fn compute_global_reward(
    reward_emission_index: &mut RewardEmissionsIndex,
    spread: Uint256,
    block_height: u64,
) {
    if reward_emission_index.last_reward_updated >= block_height {
        return;
    }

    // Get the reward accrued since the last call to compute_global_reward for this reward index
    let passed_blocks =
        Decimal256::from_uint256(block_height - reward_emission_index.last_reward_updated);
    let reward_accrued = passed_blocks * reward_emission_index.glow_emission_rate;

    if !reward_accrued.is_zero() && !spread.is_zero() {
        reward_emission_index.global_reward_index +=
            reward_accrued / Decimal256::from_uint256(spread);
    }

    reward_emission_index.last_reward_updated = block_height;
}

/// Compute reward amount an operator/referrer received
pub fn compute_operator_reward(state: &State, operator: &mut OperatorInfo) {
    operator.pending_rewards += Decimal256::from_uint256(operator.shares)
        * (state.operator_reward_emission_index.global_reward_index - operator.reward_index);
    operator.reward_index = state.operator_reward_emission_index.global_reward_index;
}

/// Compute reward amount a sponsor received
pub fn compute_sponsor_reward(state: &State, sponsor: &mut SponsorInfo) {
    sponsor.pending_rewards += Decimal256::from_uint256(sponsor.lottery_deposit)
        * (state.sponsor_reward_emission_index.global_reward_index - sponsor.reward_index);
    sponsor.reward_index = state.sponsor_reward_emission_index.global_reward_index;
}

/// Compute distributed reward and update global reward index
pub fn old_compute_reward(state: &mut OldState, pool: &OldPool, block_height: u64) {
    if state.last_reward_updated >= block_height {
        return;
    }

    let passed_blocks = Decimal256::from_uint256(block_height - state.last_reward_updated);
    let reward_accrued = passed_blocks * state.glow_emission_rate;

    let total_deposited = pool.total_user_lottery_deposits + pool.total_sponsor_lottery_deposits;
    if !reward_accrued.is_zero() && !total_deposited.is_zero() {
        state.global_reward_index += reward_accrued / Decimal256::from_uint256(total_deposited);
    }

    state.last_reward_updated = block_height;
}

/// Compute reward amount a depositor received
pub fn old_compute_depositor_reward(
    global_reward_index: Decimal256,
    depositor: &mut OldDepositorInfo,
) {
    depositor.pending_rewards += Decimal256::from_uint256(depositor.lottery_deposit)
        * (global_reward_index - depositor.reward_index);
    depositor.reward_index = global_reward_index;
}

/// Takes info relating to a deposit and does the following:
///
/// - Validates the tickets being minted
/// - Generates pseudo random tickets for the depositor when they are eligible (up to 100)
/// - Saves the newly minted tickets to the `depositor_info` and `TICKETS`
///
/// Then it returns the number of tickets which were minted
#[allow(clippy::too_many_arguments)]
pub fn handle_depositor_ticket_updates(
    deps: DepsMut,
    env: &Env,
    config: &Config,
    pool: &Pool,
    depositor: &Addr,
    depositor_info: &mut DepositorInfo,
    encoded_tickets: String,
    aust_exchange_rate: Decimal256,
    minted_shares: Uint256,
    minted_aust: Uint256,
) -> Result<u64, ContractError> {
    // Get combinations from encoded tickets
    let combinations = base64_encoded_tickets_to_vec_string_tickets(encoded_tickets)?;

    // Validate that all sequence combinations are valid
    for combination in combinations.clone() {
        if !is_valid_sequence(&combination, TICKET_LENGTH) {
            return Err(ContractError::InvalidSequence(combination));
        }
    }

    let post_transaction_depositor_shares = depositor_info.shares + minted_shares;

    let post_transaction_depositor_balance = (pool.total_user_aust + minted_aust)
        * decimal_from_ratio_or_one(
            post_transaction_depositor_shares,
            pool.total_user_shares + minted_shares,
        )
        * aust_exchange_rate;

    let post_transaction_max_depositor_tickets = Uint128::from(
        post_transaction_depositor_balance
            / Decimal256::from_uint256(
                config.ticket_price
            // Subtract 10^-5 in order to offset rounding problems
            // relies on ticket price being at least 10^-5 UST
                - Uint256::from(10u128),
            ),
    )
    .u128() as u64;

    // Get the amount of requested tickets
    let mut number_of_new_tickets = combinations.len() as u64;

    // Get the number of tickets the user would have post transaction (without accounting for round up)
    let mut post_transaction_num_depositor_tickets =
        (depositor_info.tickets.len() + number_of_new_tickets as usize) as u64;

    // Check if we need to round up the number of combinations based on the depositor's mixed_tax_post_transaction_lottery_deposit
    let mut new_combinations = combinations;
    for _ in 0..100 {
        if post_transaction_max_depositor_tickets <= post_transaction_num_depositor_tickets {
            break;
        }

        let current_time = env.block.time.nanos();
        let sequence = pseudo_random_seq(
            depositor.clone().into_string(),
            post_transaction_num_depositor_tickets,
            current_time,
        );

        // Add the randomly generated sequence to new_combinations
        new_combinations.push(sequence);
        // Increment number_of_new_tickets and post_transaction_num_depositor_tickets
        number_of_new_tickets += 1;
        post_transaction_num_depositor_tickets += 1;
    }

    // Validate that the post_transaction_max_depositor_tickets is less than or equal to the post_transaction_num_depositor_tickets
    if post_transaction_num_depositor_tickets > post_transaction_max_depositor_tickets {
        return Err(ContractError::InsufficientPostTransactionDepositorBalance {
            post_transaction_depositor_balance,
            post_transaction_num_depositor_tickets,
            post_transaction_max_depositor_tickets,
        });
    }

    // Validate that the depositor won't go over max_tickets_per_depositor
    if post_transaction_num_depositor_tickets > config.max_tickets_per_depositor {
        return Err(ContractError::MaxTicketsPerDepositorExceeded {
            max_tickets_per_depositor: config.max_tickets_per_depositor,
            post_transaction_num_depositor_tickets,
        });
    }

    for combination in new_combinations {
        // check that the number of holders for any given ticket isn't too high
        if let Some(holders) = TICKETS
            .may_load(deps.storage, combination.as_bytes())
            .unwrap()
        {
            if holders.len() >= config.max_holders as usize {
                return Err(ContractError::InvalidHolderSequence(combination));
            }
        }

        // update the TICKETS storage
        let add_ticket = |a: Option<Vec<Addr>>| -> StdResult<Vec<Addr>> {
            let mut b = a.unwrap_or_default();
            b.push(depositor.clone());
            Ok(b)
        };
        TICKETS
            .update(deps.storage, combination.as_bytes(), add_ticket)
            .unwrap();

        // add the combination to the depositor_info
        depositor_info.tickets.push(combination);
    }

    Ok(number_of_new_tickets)
}

/// Handles all changes to operator's following a deposit
/// Modifies state and depositor_info, but doesn't save them to storage.
/// Call this function before modifying depositor_stats following a deposit.
/// Call `compute_global_operator` before calling this function.
pub fn handle_depositor_operator_updates(
    deps: DepsMut,
    state: &mut State,
    pool: &mut Pool,
    depositor: &Addr,
    depositor_info: &mut DepositorInfo,
    minted_shares: Uint256,
    new_operator_addr: Option<String>,
) -> StdResult<()> {
    // If an operator is already registered, add to its deposits. If not, handle relevant updates
    if depositor_info.operator_registered() {
        // Read existing operator info
        let mut operator = read_operator_info(deps.storage, &depositor_info.operator_addr);
        // Update reward index for the operator
        compute_operator_reward(state, &mut operator);
        // Then add the new deposit on the operator
        operator.shares = operator.shares.add(minted_shares);
        // Save updated operator_info
        store_operator_info(deps.storage, &depositor_info.operator_addr, operator)?;
        // Update pool
        pool.total_operator_shares = pool.total_operator_shares.add(minted_shares);
    } else if let Some(new_operator_addr) = new_operator_addr {
        // If there is no operator registered and a new operator address is provided
        let new_operator_addr = deps.api.addr_validate(&new_operator_addr)?;

        // Validate that a user cannot set itself as its own operator
        if &new_operator_addr == depositor {
            return Err(StdError::generic_err(
                "You cannot assign yourself as your own operator",
            ));
        }

        // Set the operator_addr of the depositor_info
        depositor_info.operator_addr = new_operator_addr;

        // Read the new operator in question
        let mut new_operator = read_operator_info(deps.storage, &depositor_info.operator_addr);

        // Update the reward index for the new operator
        compute_operator_reward(state, &mut new_operator);

        // Update new operator info deposits
        let post_transaction_depositor_shares = depositor_info.shares + minted_shares;

        new_operator.shares = new_operator.shares.add(post_transaction_depositor_shares);

        // Save changes to operator info
        store_operator_info(deps.storage, &depositor_info.operator_addr, new_operator)?;

        // Update pool
        pool.total_operator_shares = pool.total_operator_shares.add(minted_shares);
    }

    Ok(())
}

/// This iterates over all mature claims for the address, and removes them, up to an optional cap.
/// it removes the finished claims and returns the total amount of tokens to be released.
pub fn claim_unbonded_withdrawals(
    depositor: &mut DepositorInfo,
    block: &BlockInfo,
    cap: Option<Uint128>,
) -> StdResult<Uint128> {
    let mut to_send = Uint128::zero();

    if depositor.unbonding_info.is_empty() {
        return Ok(to_send);
    }

    let (_send, waiting): (Vec<_>, _) = depositor.unbonding_info.iter().cloned().partition(|c| {
        // if mature and we can pay fully, then include in _send
        if c.release_at.is_expired(block) {
            let new_amount = c.amount;
            if let Some(limit) = cap {
                if to_send + Uint128::from(new_amount) > limit {
                    return false;
                }
            }
            to_send += Uint128::from(new_amount);
            true
        } else {
            //nothing to send, leave all claims in waiting status
            false
        }
    });
    depositor.unbonding_info = waiting;
    Ok(to_send)
}

pub fn pseudo_random_seq(sender_addr: String, tickets: u64, time: u64) -> String {
    let mut input = sender_addr;
    input.push_str(&time.to_string());
    input.push_str(&tickets.to_string());
    let mut hasher = Keccak256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let pseudo_random_hash = &hex::encode(result)[2..TICKET_LENGTH + 2];
    pseudo_random_hash.to_string()
}

pub fn is_valid_sequence(sequence: &str, len: usize) -> bool {
    sequence.len() == len
        && sequence
            .chars()
            .all(|c| c.is_digit(10) || ('a'..='f').contains(&c))
}

#[allow(dead_code)]
pub fn uint256_times_decimal256_ceil(a: Uint256, b: Decimal256) -> Uint256 {
    // Check for rounding error
    let rounded_output = a * b;
    let decimal_output = Decimal256::from_uint256(a) * b;

    if decimal_output != Decimal256::from_uint256(rounded_output) {
        rounded_output + Uint256::one()
    } else {
        rounded_output
    }
}

pub fn calculate_value_of_aust_to_be_redeemed_for_lottery(
    state: &State,
    pool: &Pool,
    config: &Config,
    contract_a_balance: Uint256,
    aust_exchange_rate: Decimal256,
) -> AmountRedeemableForPrizesInfo {
    // Get the aust_user_balance
    let total_user_aust = pool.total_user_aust;

    // Get the amount to take from the users
    // Split factor percent of the appreciation since the last lottery
    let value_of_user_aust_to_be_redeemed_for_lottery = total_user_aust
        * (aust_exchange_rate - state.last_lottery_execution_aust_exchange_rate)
        * config.split_factor;

    // Get the user_aust_to_redeem
    let user_aust_to_redeem = value_of_user_aust_to_be_redeemed_for_lottery / aust_exchange_rate;

    // Sponsor balance equals aust_balance - total_user_aust
    let total_sponsor_aust = contract_a_balance - pool.total_user_aust;

    // This should always equal total_sponsor_aust * (aust_exchange_rate - state.last_lottery_exchange_rate)
    let value_of_sponsor_aust_to_be_redeemed_for_lottery =
        total_sponsor_aust * aust_exchange_rate - pool.total_sponsor_lottery_deposits;

    // Get the sponsor_aust_to_redeem
    let sponsor_aust_to_redeem =
        value_of_sponsor_aust_to_be_redeemed_for_lottery / aust_exchange_rate;

    // Get the aust_to_redeem and aust_to_redeem_value
    let aust_to_redeem = user_aust_to_redeem + sponsor_aust_to_redeem;
    let aust_to_redeem_value = aust_to_redeem * aust_exchange_rate;

    AmountRedeemableForPrizesInfo {
        value_of_user_aust_to_be_redeemed_for_lottery,
        user_aust_to_redeem,
        value_of_sponsor_aust_to_be_redeemed_for_lottery,
        sponsor_aust_to_redeem,
        aust_to_redeem,
        aust_to_redeem_value,
    }
}

#[allow(dead_code)]
pub fn calculate_depositor_balance(
    pool: &Pool,
    depositor_info: &DepositorInfo,
    aust_exchange_rate: Decimal256,
) -> Uint256 {
    // Calculate the depositor's balance from their aust balance
    pool.total_user_aust
        * decimal_from_ratio_or_one(depositor_info.shares, pool.total_user_shares)
        * aust_exchange_rate
}

pub fn base64_encoded_tickets_to_vec_string_tickets(
    encoded_tickets: String,
) -> StdResult<Vec<String>> {
    // Encoded_tickets to binary
    let decoded_binary_tickets = match base64::decode(encoded_tickets) {
        Ok(decoded_binary_tickets) => decoded_binary_tickets,
        Err(_) => {
            return Err(StdError::generic_err(
                "Couldn't base64 decode the encoded tickets.".to_string(),
            ));
        }
    };

    // Validate that the decoded value is the right length
    if decoded_binary_tickets.len() % 3 != 0 {
        return Err(StdError::generic_err("Decoded tickets wrong length."));
    };

    // Will always return a Vec of 6 character hex strings
    Ok(decoded_binary_tickets
        .chunks(3)
        .map(hex::encode)
        .collect::<Vec<String>>())
}

pub fn vec_string_tickets_to_vec_binary_tickets(
    vec_string_tickets: Vec<String>,
) -> StdResult<Vec<[u8; 3]>> {
    vec_string_tickets
        .iter()
        .map(|s| {
            let vec_ticket = match hex::decode(s) {
                Ok(b) => b,
                Err(_) => return Err(StdError::generic_err("Couldn't hex decode string ticket")),
            };

            match vec_ticket.try_into() {
                Ok(b) => Ok(b),
                Err(_) => Err(StdError::generic_err(
                    "Couldn't convert vec ticket to [u8, 3]",
                )),
            }
        })
        .collect::<StdResult<Vec<[u8; 3]>>>()
}

pub fn vec_binary_tickets_to_vec_string_tickets(vec_binary_tickets: Vec<[u8; 3]>) -> Vec<String> {
    vec_binary_tickets
        .iter()
        .map(hex::encode)
        .collect::<Vec<String>>()
}

pub fn decimal_from_ratio_or_one(a: Uint256, b: Uint256) -> Decimal256 {
    if a == Uint256::zero() && b == Uint256::zero() {
        return Decimal256::one();
    }

    Decimal256::from_ratio(a, b)
}

pub fn assert_prize_distribution_not_pending(
    deps: Deps,
    prize_distributor_contract: &Addr,
) -> StdResult<()> {
    if query_prize_distribution_pending(deps, prize_distributor_contract)?
        .prize_distribution_pending
        == true
    {
        return Err(StdError::generic_err("Prize distribution pending"));
    }

    Ok(())
}

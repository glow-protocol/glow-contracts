use std::convert::TryInto;
use std::ops::Add;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, BlockInfo, DepsMut, QuerierWrapper, StdError, StdResult, Uint128};
use glow_protocol::lotto::{BoostConfig, NUM_PRIZE_BUCKETS, TICKET_LENGTH};
use sha3::{Digest, Keccak256};

use crate::querier::{
    query_address_voting_balance_at_timestamp, query_total_voting_balance_at_timestamp,
};

use crate::state::{
    read_operator_info, store_operator_info, Config, DepositorInfo, DepositorStatsInfo,
    LotteryInfo, OperatorInfo, Pool, PrizeInfo, SponsorInfo, State,
};

/// Compute distributed reward and update global reward index
pub fn compute_global_operator_reward(state: &mut State, pool: &Pool, block_height: u64) {
    if state.last_operator_reward_updated >= block_height {
        return;
    }

    // Get the reward accrued since the last call to compute_global_operator_reward
    let passed_blocks = Decimal256::from_uint256(block_height - state.last_operator_reward_updated);
    let reward_accrued = passed_blocks * state.glow_operator_emission_rate;

    if !reward_accrued.is_zero() && !pool.total_operator_shares.is_zero() {
        state.global_operator_reward_index +=
            reward_accrued / Decimal256::from_uint256(pool.total_operator_shares);
    }

    state.last_operator_reward_updated = block_height;
}

/// Compute reward amount an operator/referrer received
pub fn compute_operator_reward(state: &State, operator: &mut OperatorInfo) {
    operator.pending_rewards += Decimal256::from_uint256(operator.shares)
        * (state.global_operator_reward_index - operator.reward_index);
    operator.reward_index = state.global_operator_reward_index;
}

/// Compute reward amount a sponsor received
pub fn compute_sponsor_reward(state: &State, sponsor: &mut SponsorInfo) {
    sponsor.pending_rewards += Decimal256::from_uint256(sponsor.lottery_deposit)
        * (state.global_operator_reward_index - sponsor.reward_index);
    sponsor.reward_index = state.global_operator_reward_index;
}

/// Handles all changes to operator's following a deposit
/// Modifies state and depositor_info, but doesn't save them to storage.
/// Call this function before modifying depositor_stats following a deposit.
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
        // store operator info
        store_operator_info(deps.storage, &depositor_info.operator_addr, operator)?;
        // update pool
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

        // Set the new depositor_info operator_addr
        depositor_info.operator_addr = new_operator_addr;

        // Read the new operator in question
        let mut new_operator = read_operator_info(deps.storage, &depositor_info.operator_addr);

        // Update the reward index for the new operator
        compute_operator_reward(state, &mut new_operator);

        // Update new operator info deposits
        let post_transaction_depositor_shares = depositor_info.shares + minted_shares;

        new_operator.shares = new_operator.shares.add(post_transaction_depositor_shares);

        // Store new operator info
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

pub fn calculate_winner_prize(
    querier: &QuerierWrapper,
    config: &Config,
    prize_info: &PrizeInfo,
    lottery_info: &LotteryInfo,
    snapshotted_depositor_stats: &DepositorStatsInfo,
    winner_address: &Addr,
) -> StdResult<(Uint128, Uint128)> {
    let LotteryInfo {
        prize_buckets,
        number_winners,
        glow_prize_buckets,
        block_height,
        total_user_shares: snapshotted_total_user_shares,
        ..
    } = lottery_info;

    let PrizeInfo {
        matches: winner_matches,
        ..
    } = prize_info;

    let mut ust_to_send: Uint128 = Uint128::zero();
    let mut glow_to_send: Uint128 = Uint128::zero();

    // Get the values needed for boost calculation

    // User lottery deposit

    let snapshotted_user_shares = snapshotted_depositor_stats.shares;

    // User voting balance

    let snapshotted_user_voting_balance = query_address_voting_balance_at_timestamp(
        querier,
        &config.ve_contract,
        *block_height,
        winner_address,
    )?;

    // Total voting balance

    let snapshotted_total_voting_balance =
        query_total_voting_balance_at_timestamp(querier, &config.ve_contract, *block_height)?;

    for i in 0..NUM_PRIZE_BUCKETS {
        if number_winners[i] == 0 {
            continue;
        }

        // Handle ust calculations
        let prize_available: Uint256 = prize_buckets[i];

        let amount: Uint128 = prize_available
            .multiply_ratio(winner_matches[i], number_winners[i])
            .into();

        ust_to_send += amount;

        // Handle glow calculations
        let glow_prize_available = glow_prize_buckets[i];

        // Get the raw awarded glow
        let glow_raw_amount =
            glow_prize_available.multiply_ratio(winner_matches[i], number_winners[i]);

        // Get the glow boost multiplier
        let glow_boost_multiplier = calculate_boost_multiplier(
            config.lotto_winner_boost_config.clone(),
            snapshotted_user_shares,
            *snapshotted_total_user_shares,
            snapshotted_user_voting_balance,
            snapshotted_total_voting_balance,
        );

        // Get the GLOW to send
        glow_to_send += Uint128::from(glow_raw_amount * glow_boost_multiplier);
    }

    Ok((ust_to_send, glow_to_send))
}

pub fn calculate_boost_multiplier(
    boost_config: BoostConfig,
    snapshotted_user_shares: Uint256,
    snapshotted_total_user_shares: Uint256,
    snapshotted_user_voting_balance: Uint128,
    snapshotted_total_voting_balance: Uint128,
) -> Decimal256 {
    // Boost formula:
    // min(
    //  max_multiplier,
    //  min_multiplier +
    //    (TotalDeposited / UserDeposited)
    //    * (UserVotingBalance / (max_proportional_ratio * TotalVotingBalance))
    //    * (max_multiplier - min_multiplier)
    //  )

    // Get the multiplier for users with no voting power
    let glow_base_multiplier = boost_config.base_multiplier;
    let glow_max_multiplier = boost_config.max_multiplier;

    // Calculate the additional multiplier for users with voting power
    let glow_voting_boost = if snapshotted_total_voting_balance > Uint128::zero()
        && snapshotted_user_shares > Uint256::zero()
    {
        let inverted_user_lottery_deposit_proportion =
            Decimal256::from_ratio(snapshotted_total_user_shares, snapshotted_user_shares);

        let user_voting_balance_proportion = Decimal256::from_ratio(
            Uint256::from(snapshotted_user_voting_balance),
            boost_config.total_voting_power_weight
                * Uint256::from(snapshotted_total_voting_balance),
        );

        let slope = glow_max_multiplier - glow_base_multiplier;

        inverted_user_lottery_deposit_proportion * user_voting_balance_proportion * slope
    } else {
        // If total_voting_balance is zero, then set glow_voting_boost to zero
        Decimal256::zero()
    };

    // Sum glow_base_multiplier and glow_voting_boost and set it to 1 if greater than 1
    let mut glow_multiplier = glow_base_multiplier + glow_voting_boost;
    if glow_multiplier > glow_max_multiplier {
        glow_multiplier = glow_max_multiplier;
    }
    glow_multiplier
}

// Get max bounds
pub fn calculate_max_bound(min_bound: &str, minimum_matches_for_winning_ticket: usize) -> String {
    format!(
        "{:f<length$}",
        min_bound[..minimum_matches_for_winning_ticket].to_string(),
        length = TICKET_LENGTH
    )
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

pub fn get_minimum_matches_for_winning_ticket(
    prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS],
) -> StdResult<usize> {
    for (index, fraction_of_prize) in prize_distribution.iter().enumerate() {
        if *fraction_of_prize != Decimal256::zero() {
            return Ok(index);
        }
    }

    // Should never happen because one of the prize distribution values should be greater than 0,
    // throw an error
    Err(StdError::generic_err(
        "The minimum matches for a winning ticket could not be calculated due to a malforming of the prize distribution"
    ))
}

pub struct ExecuteLotteryRedeemedAustInfo {
    pub value_of_user_aust_to_be_redeemed_for_lottery: Uint256,
    pub user_aust_to_redeem: Uint256,
    pub value_of_sponsor_aust_to_be_redeemed_for_lottery: Uint256,
    pub sponsor_aust_to_redeem: Uint256,
    pub aust_to_redeem: Uint256,
    pub aust_to_redeem_value: Uint256,
}

pub fn calculate_value_of_aust_to_be_redeemed_for_lottery(
    state: &State,
    pool: &Pool,
    config: &Config,
    contract_a_balance: Uint256,
    aust_exchange_rate: Decimal256,
) -> ExecuteLotteryRedeemedAustInfo {
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
    let total_sponsor_aust = Uint256::from(contract_a_balance) - pool.total_user_aust;

    // This should equal aust_sponsor_balance * (rate - state.last_lottery_exchange_rate) * config.split_factor;
    let value_of_sponsor_aust_to_be_redeemed_for_lottery =
        total_sponsor_aust * aust_exchange_rate - pool.total_sponsor_lottery_deposits;

    // Get the sponsor_aust_to_redeem
    let sponsor_aust_to_redeem =
        value_of_sponsor_aust_to_be_redeemed_for_lottery / aust_exchange_rate;

    // Get the aust_to_redeem and aust_to_redeem_value
    let aust_to_redeem = user_aust_to_redeem + sponsor_aust_to_redeem;
    let aust_to_redeem_value = aust_to_redeem * aust_exchange_rate;

    ExecuteLotteryRedeemedAustInfo {
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
    let depositor_balance = pool.total_user_aust
        * Decimal256::from_ratio(depositor_info.shares, pool.total_user_shares)
        * aust_exchange_rate;

    depositor_balance
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

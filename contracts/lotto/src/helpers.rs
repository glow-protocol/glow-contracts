use std::convert::TryInto;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, BlockInfo, QuerierWrapper, StdError, StdResult, Uint128};
use glow_protocol::lotto::{BoostConfig, NUM_PRIZE_BUCKETS, TICKET_LENGTH};
use sha3::{Digest, Keccak256};

use crate::querier::{
    query_address_voting_balance_at_height, query_total_voting_balance_at_height,
};

use crate::state::{
    Config, DepositorInfo, DepositorStatsInfo, LotteryInfo, Pool, PrizeInfo, SponsorInfo, State,
};

/// Compute distributed reward and update global reward index
pub fn compute_reward(state: &mut State, pool: &Pool, block_height: u64) {
    if state.last_reward_updated >= block_height {
        return;
    }

    let passed_blocks = Decimal256::from_uint256(block_height - state.last_reward_updated);
    let reward_accrued = passed_blocks * state.glow_emission_rate;

    let total_sponsor_lottery_deposits = pool.total_sponsor_lottery_deposits;
    if !reward_accrued.is_zero() && !total_sponsor_lottery_deposits.is_zero() {
        state.global_reward_index +=
            reward_accrued / Decimal256::from_uint256(total_sponsor_lottery_deposits);
    }

    state.last_reward_updated = block_height;
}

/// Compute reward amount a sponsor received
pub fn compute_sponsor_reward(state: &State, sponsor: &mut SponsorInfo) {
    sponsor.pending_rewards += Decimal256::from_uint256(sponsor.lottery_deposit)
        * (state.global_reward_index - sponsor.reward_index);
    sponsor.reward_index = state.global_reward_index;
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
        total_user_lottery_deposits: snapshotted_total_user_lottery_deposits,
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

    let snapshotted_user_lottery_deposit = snapshotted_depositor_stats.lottery_deposit;

    // User voting balance

    let snapshotted_user_voting_balance = if let Ok(response) =
        query_address_voting_balance_at_height(
            querier,
            &config.gov_contract,
            *block_height,
            winner_address,
        ) {
        response.balance
    } else {
        Uint128::zero()
    };

    // Total voting balance

    let snapshotted_total_voting_balance = if let Ok(response) =
        query_total_voting_balance_at_height(querier, &config.gov_contract, *block_height)
    {
        response.total_supply
    } else {
        Uint128::zero()
    };

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
            snapshotted_user_lottery_deposit,
            *snapshotted_total_user_lottery_deposits,
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
    snapshotted_user_lottery_deposit: Uint256,
    snapshotted_total_user_lottery_deposits: Uint256,
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
        && snapshotted_user_lottery_deposit > Uint256::zero()
    {
        let inverted_user_lottery_deposit_proportion = Decimal256::from_ratio(
            snapshotted_total_user_lottery_deposits,
            snapshotted_user_lottery_deposit,
        );

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

pub fn calculate_lottery_balance(
    state: &State,
    pool: &Pool,
    contract_a_balance: Uint256,
    rate: Decimal256,
) -> StdResult<Uint256> {
    // Validate that the value of the contract's lottery aust is always at least the
    // sum of the value of the user savings aust and lottery deposits.
    // This check should never fail but is in place as an extra safety measure.
    let lottery_pool_value = (contract_a_balance - pool.total_user_savings_aust) * rate;
    if lottery_pool_value < (pool.total_user_lottery_deposits + pool.total_sponsor_lottery_deposits)
    {
        return Err(StdError::generic_err(
            format!("Value of lottery pool must be greater than the value of lottery deposits. Pool value: {}. Lottery deposits: {}", lottery_pool_value,pool.total_user_lottery_deposits + pool.total_sponsor_lottery_deposits)
        ));
    }

    let carry_over_value = state
        .prize_buckets
        .iter()
        .fold(Uint256::zero(), |sum, val| sum + *val);

    // Lottery balance equals aust_balance - total_user_savings_aust
    let aust_lottery_balance = contract_a_balance - pool.total_user_savings_aust;

    // Get the ust value of the aust going towards the lottery
    let aust_lottery_balance_value = aust_lottery_balance * rate;

    let amount_to_redeem = aust_lottery_balance_value
        - pool.total_user_lottery_deposits
        - pool.total_sponsor_lottery_deposits;

    Ok(carry_over_value + amount_to_redeem)
}

#[allow(dead_code)]
pub fn calculate_depositor_balance(depositor: DepositorInfo, rate: Decimal256) -> Uint256 {
    // Get the amount of aust equivalent to the depositor's lottery deposit
    let depositor_lottery_aust = depositor.lottery_deposit / rate;

    // Calculate the depositor's aust balance
    let depositor_aust_balance = depositor.savings_aust + depositor_lottery_aust;

    // Calculate the depositor's balance from their aust balance
    depositor_aust_balance * rate
}

pub fn base64_encoded_tickets_to_vec_string_tickets(
    encoded_tickets: String,
) -> StdResult<Vec<String>> {
    // Encoded_tickets to binary
    let decoded_binary_tickets = match base64::decode(encoded_tickets) {
        Ok(decoded_binary_tickets) => decoded_binary_tickets,
        Err(e) => {
            return Err(StdError::generic_err(format!(
                "Couldn't base64 decode the encoded tickets. Error: {}",
                e
            )));
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

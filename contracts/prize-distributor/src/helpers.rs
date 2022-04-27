use std::convert::TryInto;
use std::ops::Add;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, BlockInfo, DepsMut, Env, QuerierWrapper, StdError, StdResult, Uint128};
use glow_protocol::lotto::{DepositorInfo, DepositorStatsInfo};
use glow_protocol::prize_distributor::{
    BoostConfig, RewardEmissionsIndex, NUM_PRIZE_BUCKETS, TICKET_LENGTH,
};
use sha3::{Digest, Keccak256};

use crate::error::ContractError;
use crate::querier::{
    query_address_voting_balance_at_timestamp, query_total_voting_balance_at_timestamp,
};

use crate::state::{Config, LotteryInfo, Pool, PrizeInfo, State};

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

// Think about moving this to ve_token
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
            decimal_from_ratio_or_one(snapshotted_total_user_shares, snapshotted_user_shares);

        let user_voting_balance_proportion = decimal_from_ratio_or_one(
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

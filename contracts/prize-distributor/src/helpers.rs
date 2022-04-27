use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, QuerierWrapper, StdError, StdResult, Uint128};
use glow_protocol::lotto::{DepositorInfo, DepositorStatsInfo};
use glow_protocol::prize_distributor::{BoostConfig, NUM_PRIZE_BUCKETS, TICKET_LENGTH};

use crate::querier::{
    query_address_voting_balance_at_timestamp, query_total_voting_balance_at_timestamp,
};

use crate::state::{Config, LotteryInfo, PrizeInfo, State};

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

pub fn decimal_from_ratio_or_one(a: Uint256, b: Uint256) -> Decimal256 {
    if a == Uint256::zero() && b == Uint256::zero() {
        return Decimal256::one();
    }

    Decimal256::from_ratio(a, b)
}

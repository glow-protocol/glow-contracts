use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, BlockInfo, StdError, StdResult, Storage, Uint128};
use glow_protocol::lotto::{NUM_PRIZE_BUCKETS, TICKET_LENGTH};
use sha3::{Digest, Keccak256};

use crate::state::{
    read_depositor_info, store_depositor_info, DepositorInfo, Pool, SponsorInfo, State,
};

/// Compute distributed reward and update global reward index
pub fn compute_reward(state: &mut State, pool: &Pool, block_height: u64) {
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
pub fn compute_depositor_reward(state: &State, depositor: &mut DepositorInfo) {
    depositor.pending_rewards += Decimal256::from_uint256(depositor.lottery_deposit)
        * (state.global_reward_index - depositor.reward_index);
    depositor.reward_index = state.global_reward_index;
}

/// Compute reward amount a sponsor received
pub fn compute_sponsor_reward(state: &State, sponsor: &mut SponsorInfo) {
    sponsor.pending_rewards += Decimal256::from_uint256(sponsor.lottery_deposit)
        * (state.global_reward_index - sponsor.reward_index);
    sponsor.reward_index = state.global_reward_index;
}

/// This iterates over all mature claims for the address, and removes them, up to an optional cap.
/// it removes the finished claims and returns the total amount of tokens to be released.
pub fn claim_deposits(
    storage: &mut dyn Storage,
    addr: &Addr,
    block: &BlockInfo,
    cap: Option<Uint128>,
) -> StdResult<(Uint128, DepositorInfo)> {
    let mut to_send = Uint128::zero();
    let mut depositor = read_depositor_info(storage, addr);

    if depositor.unbonding_info.is_empty() {
        return Ok((to_send, depositor));
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
    store_depositor_info(storage, addr, &depositor)?;
    Ok((to_send, depositor))
}

pub fn calculate_winner_prize(
    lottery_prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    address_rank: [u32; NUM_PRIZE_BUCKETS],
    lottery_winners: [u32; NUM_PRIZE_BUCKETS],
) -> Uint128 {
    let mut to_send: Uint128 = Uint128::zero();
    for i in 0..NUM_PRIZE_BUCKETS {
        if lottery_winners[i] == 0 {
            continue;
        }
        let prize_available: Uint256 = lottery_prize_buckets[i];

        let amount: Uint128 = prize_available
            .multiply_ratio(address_rank[i], lottery_winners[i])
            .into();

        to_send += amount;
    }
    to_send
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

pub fn encoded_tickets_to_combinations(encoded_tickets: String) -> StdResult<Vec<String>> {
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

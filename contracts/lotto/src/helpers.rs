use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, BlockInfo, StdResult, Storage, Uint128};
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

    let total_deposited = pool.total_deposits + pool.total_sponsor_amount;
    if !reward_accrued.is_zero() && !total_deposited.is_zero() {
        state.global_reward_index += reward_accrued / total_deposited;
    }

    state.last_reward_updated = block_height;
}

/// Compute reward amount a depositor received
pub fn compute_depositor_reward(state: &State, depositor: &mut DepositorInfo) {
    depositor.pending_rewards +=
        depositor.deposit_amount * (state.global_reward_index - depositor.reward_index);
    depositor.reward_index = state.global_reward_index;
}

/// Compute reward amount a sponsor received
pub fn compute_sponsor_reward(state: &State, sponsor: &mut SponsorInfo) {
    sponsor.pending_rewards += sponsor.amount * (state.global_reward_index - sponsor.reward_index);
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
            let new_amount = c.amount * Uint256::one();
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
    total_awarded: Decimal256,
    address_rank: [u32; 6],
    lottery_winners: [u32; 6],
    prize_dis: [Decimal256; 6],
) -> Uint128 {
    let mut to_send: Uint128 = Uint128::zero();
    for i in 2..6 {
        if lottery_winners[i] == 0 {
            continue;
        }
        let ranked_price: Uint256 = (total_awarded * prize_dis[i]) * Uint256::one();

        let amount: Uint128 = ranked_price
            .multiply_ratio(address_rank[i], lottery_winners[i])
            .into();

        to_send += amount;
    }
    to_send
}

pub fn calculate_max_bound(min_bound: &str) -> String {
    // Get max bounds
    let max_bound: Vec<char> = min_bound[..2].chars().collect();
    let first_char = max_bound[0].to_digit(16).unwrap();
    let second_char = max_bound[1].to_digit(16).unwrap();

    if second_char == 15 {
        if first_char == 15 {
            "fffff".to_string()
        } else {
            format!("{:x}0000", first_char + 1)
        }
    } else {
        format!("{}{:x}000", max_bound[0], second_char + 1)
    }
}

pub fn pseudo_random_seq(sender_addr: String, tickets: u64, time: u64) -> String {
    let mut input = sender_addr;
    input.push_str(&time.to_string());
    input.push_str(&tickets.to_string());
    let mut hasher = Keccak256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let pseudo_random_hash = &hex::encode(result)[2..7];
    pseudo_random_hash.to_string()
}

pub fn is_valid_sequence(sequence: &str, len: u8) -> bool {
    sequence.len() == (len as usize)
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

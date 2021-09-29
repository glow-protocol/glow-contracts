use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, BlockInfo, DepsMut, StdResult, Storage, Uint128};

use crate::state::{read_depositor_info, store_depositor_info, DepositorInfo, Pool, State};
use cw0::Expiration;
use glow_protocol::lotto::Claim;

/// Compute distributed reward and update global reward index
pub fn compute_reward(state: &mut State, pool: &Pool, block_height: u64) {
    if state.last_reward_updated >= block_height {
        return;
    }

    let passed_blocks = Decimal256::from_uint256(block_height - state.last_reward_updated);
    let reward_accrued = passed_blocks * state.glow_emission_rate;

    if !reward_accrued.is_zero() && !pool.total_deposits.is_zero() {
        state.global_reward_index += reward_accrued / pool.total_deposits;
    }

    state.last_reward_updated = block_height;
}

/// Compute reward amount a borrower received
pub fn compute_depositor_reward(state: &State, depositor: &mut DepositorInfo) {
    depositor.pending_rewards +=
        depositor.deposit_amount * (state.global_reward_index - depositor.reward_index);
    depositor.reward_index = state.global_reward_index;
}

/// This iterates over all mature claims for the address, and removes them, up to an optional cap.
/// it removes the finished claims and returns the total amount of tokens to be released.
pub fn claim_deposits(
    storage: &mut dyn Storage,
    addr: &Addr,
    block: &BlockInfo,
    cap: Option<Uint128>,
) -> StdResult<Uint128> {
    let mut to_send = Uint128::zero();
    let mut depositor = read_depositor_info(storage, addr);

    if depositor.unbonding_info.is_empty() {
        return Ok(to_send);
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
    Ok(to_send)
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

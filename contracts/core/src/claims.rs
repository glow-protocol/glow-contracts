use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{BlockInfo, CanonicalAddr, DepsMut, StdResult, Storage, Uint128};

use crate::state::{read_depositor_info, store_depositor_info};
use cw0::Expiration;
use glow_protocol::core::Claim;

// TODO: helper functions should be methods, check cw_controllers
/// This creates a claim, such that the given address can claim an amount of tokens after
/// the release date. Fn not used at the moment
#[allow(dead_code)]
pub fn create_claim(
    deps: DepsMut,
    addr: &CanonicalAddr,
    amount: Decimal256,
    release_at: Expiration,
) -> StdResult<()> {
    let mut depositor = read_depositor_info(deps.as_ref().storage, addr);
    depositor.unbonding_info.push(Claim { amount, release_at });
    store_depositor_info(deps.storage, addr, &depositor)?;
    Ok(())
}

/// This iterates over all mature claims for the address, and removes them, up to an optional cap.
/// it removes the finished claims and returns the total amount of tokens to be released.
pub fn claim_deposits(
    storage: &mut dyn Storage,
    addr: &CanonicalAddr,
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

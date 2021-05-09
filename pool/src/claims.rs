use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    Api, BlockInfo, CanonicalAddr, Extern, HumanAddr, Order, Querier, StdError, StdResult, Storage,
    Uint128,
};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton,
};

use crate::state::{read_depositor_info, store_depositor_info};
use cw0::Expiration;

// TODO: helper functions should be methods, check cw_controllers
/// This creates a claim, such that the given address can claim an amount of tokens after
/// the release date.
pub fn create_claim<S: Storage>(
    storage: &mut S,
    addr: &CanonicalAddr,
    amount: Decimal256,
    release_at: Expiration,
) -> StdResult<()> {
    let mut depositor = read_depositor_info(storage, addr);
    depositor
        .unbonding_info
        .push(Claim { amount, release_at });
    store_depositor_info(storage, addr, &depositor)?;
    Ok(())
}

/// This iterates over all mature claims for the address, and removes them, up to an optional cap.
/// it removes the finished claims and returns the total amount of tokens to be released.
pub fn claim_deposits<S:Storage>(
    storage: &mut S,
    addr: &CanonicalAddr,
    block: &BlockInfo,
    cap: Option<Uint128>,
) -> StdResult<Uint128> {
    let mut to_send = Uint128(0);
    let mut depositor = read_depositor_info(storage, addr);

    if depositor.unbonding_info.len() == 0 {
        return Err(StdError::generic_err(
            "Depositor does not have any outstanding claims",
        ))
    }

    let (_send, waiting): (Vec<_>, _) = depositor.unbonding_info.iter().cloned().partition(|c| {
        // if mature and we can pay fully, then include in _send
        if c.release_at.is_expired(block) {
            let new_amount = c.amount * Uint256::one();
            if let Some(limit) = cap {
                if to_send + new_amount.into() > limit {
                    return false;
                }
            }
            to_send += new_amount.into();
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Claim {
    pub amount: Decimal256,
    pub release_at: Expiration,
}

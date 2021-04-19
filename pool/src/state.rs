use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256};
use cosmwasm_std::{Api, CanonicalAddr, Extern, HumanAddr, Querier, StdResult, Storage};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, ReadonlyBucket, ReadonlySingleton, Singleton,
};

//use crate::msg::DepositorInfoResponse;

const KEY_CONFIG: &[u8] = b"config";
const KEY_STATE: &[u8] = b"state";

const PREFIX_DEPOSIT: &[u8] = b"deposit";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub contract_addr: CanonicalAddr,
    pub owner: CanonicalAddr,
    pub b_terra_contract: CanonicalAddr,
    pub stable_denom: String,
    pub anchor_contract: CanonicalAddr,
    pub period_prize: u64,
    pub ticket_exchange_rate: Decimal256, //pub mock_anchor_rate // to simulate interest rate accrued
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
    pub total_tickets: Decimal256,
    pub total_reserves: Decimal256,
    pub last_interest: Decimal256,
    pub total_accrued_interest: Decimal256,
    pub award_available: Decimal256,
    pub total_assets: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorInfo {
    pub deposit_amount: Decimal256, //to-do not needed
    pub tickets: Decimal256,
    pub accrued_interest: Decimal256,
}

pub fn store_config<S: Storage>(storage: &mut S, data: &Config) -> StdResult<()> {
    Singleton::new(storage, KEY_CONFIG).save(data)
}

pub fn read_config<S: Storage>(storage: &S) -> StdResult<Config> {
    ReadonlySingleton::new(storage, KEY_CONFIG).load()
}

pub fn store_state<S: Storage>(storage: &mut S, data: &State) -> StdResult<()> {
    Singleton::new(storage, KEY_STATE).save(data)
}

pub fn read_state<S: Storage>(storage: &S) -> StdResult<State> {
    ReadonlySingleton::new(storage, KEY_STATE).load()
}
//TODO: think if we need to keep track of the coins here or in the sUST contract
/*
pub fn store_depositor_info<S: Storage>(
    storage: &mut S,
    depositor: &CanonicalAddr,
    deposit: &DepositorInfo,
) -> StdResult<()> {
    bucket(PREFIX_DEPOSIT, storage).save(depositor.as_slice(), deposit)
}

pub fn read_depositor_info<S: Storage>(
    storage: &mut S,
    depositor: &CanonicalAddr,
) -> DepositorInfo {
    match bucket_read(PREFIX_DEPOSIT, storage).load(depositor.as_slice()) {
        Ok(v) => v,
        _ => DepositorInfo {
            deposit_amount: Decimal256::zero(),
            tickets: Decimal256::zero(),
            accrued_interest: Decimal256::zero(),
        },
    }
}

// settings for pagination
const MAX_LIMIT: u32 = 30;
const DEFAULT_LIMIT: u32 = 10;

pub fn read_depositor_infos<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    start_after: Option<CanonicalAddr>,
    limit: Option<u32>,
) -> StdResult<Vec<DepositorInfoResponse>> {
    let liability_bucket: ReadonlyBucket<S, DepositorInfo> =
        bucket_read(PREFIX_LIABILITY, &deps.storage);

    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = calc_range_start(start_after);

    liability_bucket
        .range(start.as_deref(), None, Order::Ascending)
        .take(limit)
        .map(|elem| {
            let (k, v) = elem?;
            let depositor: HumanAddr = deps.api.human_address(&CanonicalAddr::from(k))?;
            Ok(DepositorInfoResponse {
                depositor,
                deposit_amount: v.deposit_amount,
                tickets: v.tickets,
                accrued_interest: v.accrued_interest,
            })
        })
        .collect()
}

// this will set the first key after the provided key, by appending a 1 byte
fn calc_range_start(start_after: Option<CanonicalAddr>) -> Option<Vec<u8>> {
    start_after.map(|addr| {
        let mut v = addr.as_slice().to_vec();
        v.push(1);
        v
    })
}
*/


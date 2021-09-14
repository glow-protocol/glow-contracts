use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{CanonicalAddr, Deps, Order, StdResult, Storage, Uint128};
use cosmwasm_storage::{bucket, bucket_read, Bucket, ReadonlyBucket, ReadonlySingleton, Singleton};
use cw0::{Duration, Expiration};
use glow_protocol::lotto::{Claim, DepositorInfoResponse};

use crate::prize_strategy::count_seq_matches;

const KEY_CONFIG: &[u8] = b"config";
const KEY_STATE: &[u8] = b"state";

const PREFIX_SEQUENCE: &[u8] = b"sequence";
const PREFIX_LOTTERY: &[u8] = b"lottery";
const PREFIX_DEPOSIT: &[u8] = b"depositor";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: CanonicalAddr,
    pub a_terra_contract: CanonicalAddr,
    pub gov_contract: CanonicalAddr,
    pub distributor_contract: CanonicalAddr,
    pub anchor_contract: CanonicalAddr,
    pub stable_denom: String,
    pub lottery_interval: Duration, // number of blocks (or time) between lotteries
    pub block_time: Duration, // number of blocks (or time) lottery is blocked while is executed
    pub ticket_price: Decimal256, // prize of a ticket in stable_denom
    pub prize_distribution: Vec<Decimal256>, // [0, 0, 0.05, 0.15, 0.3, 0.5]
    pub target_award: Decimal256,
    pub reserve_factor: Decimal256, // % of the prize that goes to the reserve fund
    pub split_factor: Decimal256,   // what % of interest goes to saving and which one lotto pool
    pub instant_withdrawal_fee: Decimal256, // % to be deducted as a fee for instant withdrawals
    pub unbonding_period: Duration,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
    pub total_tickets: Uint256,
    pub total_reserve: Decimal256,
    pub total_deposits: Decimal256,
    pub lottery_deposits: Decimal256,
    pub shares_supply: Decimal256,
    pub deposit_shares: Decimal256,
    pub award_available: Decimal256,
    pub current_lottery: u64,
    pub next_lottery_time: Expiration,
    pub last_reward_updated: u64,
    pub global_reward_index: Decimal256,
    pub glow_emission_rate: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorInfo {
    pub deposit_amount: Decimal256,
    pub shares: Decimal256,
    pub redeemable_amount: Uint128,
    pub reward_index: Decimal256,
    pub pending_rewards: Decimal256,
    pub tickets: Vec<String>,
    pub unbonding_info: Vec<Claim>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryInfo {
    pub sequence: String,
    pub awarded: bool,
    pub total_prizes: Decimal256,
    pub winners: Vec<(u8, Vec<CanonicalAddr>)>, // [(number_hits, [hitters])]
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Sequence {
    pub holders: Vec<CanonicalAddr>,
}

pub fn store_config(storage: &mut dyn Storage, data: &Config) -> StdResult<()> {
    Singleton::new(storage, KEY_CONFIG).save(data)
}

pub fn read_config(storage: &dyn Storage) -> StdResult<Config> {
    ReadonlySingleton::new(storage, KEY_CONFIG).load()
}

pub fn store_state(storage: &mut dyn Storage, data: &State) -> StdResult<()> {
    Singleton::new(storage, KEY_STATE).save(data)
}

pub fn read_state(storage: &dyn Storage) -> StdResult<State> {
    ReadonlySingleton::new(storage, KEY_STATE).load()
}

pub fn store_lottery_info(
    storage: &mut dyn Storage,
    lottery_id: u64,
    lottery_info: &LotteryInfo,
) -> StdResult<()> {
    bucket(storage, PREFIX_LOTTERY).save(&lottery_id.to_be_bytes(), lottery_info)
}

pub fn read_lottery_info(storage: &dyn Storage, lottery_id: u64) -> LotteryInfo {
    match bucket_read(storage, PREFIX_LOTTERY).load(&lottery_id.to_be_bytes()) {
        Ok(v) => v,
        _ => LotteryInfo {
            sequence: "".to_string(),
            awarded: false,
            total_prizes: Decimal256::zero(),
            winners: vec![],
        },
    }
}

pub fn sequence_bucket(storage: &mut dyn Storage) -> Bucket<Vec<CanonicalAddr>> {
    bucket(storage, PREFIX_SEQUENCE)
}

pub fn store_sequence_info(
    storage: &mut dyn Storage,
    depositor: CanonicalAddr,
    sequence: &str,
) -> StdResult<()> {
    let mut holders: Vec<CanonicalAddr> = read_sequence_info(storage, sequence);
    holders.push(depositor);
    sequence_bucket(storage).save(sequence.as_bytes(), &holders)
}

pub fn read_sequence_info(storage: &dyn Storage, sequence: &str) -> Vec<CanonicalAddr> {
    match bucket_read(storage, PREFIX_SEQUENCE).load(sequence.as_bytes()) {
        Ok(v) => v,
        _ => vec![],
    }
}

// settings for pagination
const MAX_LIMIT: u32 = 30;
const DEFAULT_LIMIT: u32 = 10;

// pub fn read_all_sequences<S: Storage, A: Api, Q: Querier>(
//     deps: Extern<S, A, Q>,
//     start_after: Option<CanonicalAddr>,
//     limit: Option<u32>,
// ) -> StdResult<Vec<(String, Vec<CanonicalAddr>)>> {
//     let sequence_bucket: ReadonlyBucket<S, Vec<CanonicalAddr>> =
//         bucket_read(PREFIX_SEQUENCE, &deps.storage);

//     let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
//     let start = calc_range_start(start_after);

//     sequence_bucket
//         .range(start.as_deref(), None, Order::Ascending)
//         .take(limit)
//         .map(|elem| {
//             let (k, v) = elem?;
//             let sequence = String::from_utf8(k).ok().unwrap();
//             Ok((sequence, v))
//         })
//         .collect()
// }

pub fn read_all_sequences(deps: Deps) -> Vec<(String, Vec<CanonicalAddr>)> {
    let sequence_bucket: ReadonlyBucket<Vec<CanonicalAddr>> =
        bucket_read(deps.storage, PREFIX_SEQUENCE);

    // TODO: review limit value for optimization
    let limit = 1000;
    let mut start = None;

    let mut all_sequences = vec![];

    loop {
        let mut sequences: Vec<(String, Vec<CanonicalAddr>)> = sequence_bucket
            .range(start.as_deref(), None, Order::Ascending)
            .take(limit)
            .map(|elem| {
                let (k, v) = elem.unwrap();
                let sequence = String::from_utf8(k).ok().unwrap();
                (sequence, v)
            })
            .collect();

        if sequences.is_empty() {
            break;
        }

        start = calc_sequence_range_start(Some(sequences.last().unwrap().0.as_str()));

        let last_loop = sequences.len() < limit;

        all_sequences.append(&mut sequences);

        if last_loop {
            break;
        }
    }

    all_sequences
}

// pub fn read_matching_sequences<S: Storage, A: Api, Q: Querier>(
//     deps: &Extern<S, A, Q>,
//     start_after: Option<CanonicalAddr>,
//     limit: Option<u32>,
//     win_sequence: &str,
// ) -> Vec<(u8, Vec<CanonicalAddr>)> {
//     let sequence_bucket: ReadonlyBucket<S, Vec<CanonicalAddr>> =
//         bucket_read(PREFIX_SEQUENCE, &deps.storage);

//     let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
//     let start = calc_range_start(start_after);

//     sequence_bucket
//         .range(start.as_deref(), None, Order::Ascending)
//         .take(limit)
//         .filter_map(|elem| {
//             let (k, v) = elem.ok()?;
//             let sequence = String::from_utf8(k).ok()?;
//             let number_matches = count_seq_matches(win_sequence, &sequence);
//             if number_matches < 2 {
//                 None
//             } else {
//                 Some((number_matches, v))
//             }
//         })
//         .collect()
// }

pub fn read_matching_sequences(deps: Deps, win_sequence: &str) -> Vec<(u8, Vec<CanonicalAddr>)> {
    let all_sequences = read_all_sequences(deps);

    all_sequences
        .iter()
        .filter_map(|elem| {
            let (s, v) = elem;
            let sequence = String::from(s);
            let number_matches = count_seq_matches(win_sequence, &sequence);
            if number_matches < 2 {
                None
            } else {
                Some((number_matches, v.clone()))
            }
        })
        .collect()
}

pub fn store_depositor_info(
    storage: &mut dyn Storage,
    depositor: &CanonicalAddr,
    depositor_info: &DepositorInfo,
) -> StdResult<()> {
    bucket(storage, PREFIX_DEPOSIT).save(depositor.as_slice(), depositor_info)
}

pub fn read_depositor_info(storage: &dyn Storage, depositor: &CanonicalAddr) -> DepositorInfo {
    match bucket_read(storage, PREFIX_DEPOSIT).load(depositor.as_slice()) {
        Ok(v) => v,
        _ => DepositorInfo {
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![],
        },
    }
}

pub fn read_depositors(
    deps: Deps,
    start_after: Option<CanonicalAddr>,
    limit: Option<u32>,
) -> StdResult<Vec<DepositorInfoResponse>> {
    let liability_bucket: ReadonlyBucket<DepositorInfo> = bucket_read(deps.storage, PREFIX_DEPOSIT);

    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = calc_range_start(start_after);

    liability_bucket
        .range(start.as_deref(), None, Order::Ascending)
        .take(limit)
        .map(|elem| {
            let (k, v) = elem?;
            let depositor = deps.api.addr_humanize(&CanonicalAddr::from(k))?.to_string();
            Ok(DepositorInfoResponse {
                depositor,
                deposit_amount: v.deposit_amount,
                shares: v.shares,
                redeemable_amount: v.redeemable_amount,
                reward_index: v.reward_index,
                pending_rewards: v.pending_rewards,
                tickets: v.tickets,
                unbonding_info: v.unbonding_info,
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

// this will set the first key after the provided key, by appending a 1 byte
fn calc_sequence_range_start(start_after: Option<&str>) -> Option<Vec<u8>> {
    start_after.map(|sequence| {
        let mut v = sequence.as_bytes().to_vec();
        v.push(1);
        v
    })
}

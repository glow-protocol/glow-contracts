use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Deps, Order, StdResult, Storage};
use cosmwasm_storage::{bucket, bucket_read, ReadonlyBucket};
use cw0::{Duration, Expiration};
use cw_storage_plus::{Item, Map, U64Key};
use glow_protocol::lotto::{Claim, DepositorInfoResponse};

const PREFIX_LOTTERY: &[u8] = b"lottery";
const PREFIX_DEPOSIT: &[u8] = b"depositor";
const PREFIX_SPONSOR: &[u8] = b"sponsor";

pub const CONFIG: Item<Config> = Item::new("config");
pub const STATE: Item<State> = Item::new("state");
pub const POOL: Item<Pool> = Item::new("pool");
pub const TICKETS: Map<&[u8], Vec<Addr>> = Map::new("tickets");
pub const PRIZES: Map<(&Addr, U64Key), PrizeInfo> = Map::new("prizes");

// settings for pagination
const MAX_LIMIT: u32 = 100;
const DEFAULT_LIMIT: u32 = 10;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    pub a_terra_contract: Addr,
    pub gov_contract: Addr,
    pub distributor_contract: Addr,
    pub anchor_contract: Addr,
    pub oracle_contract: Addr,
    pub stable_denom: String,
    pub lottery_interval: Duration,
    pub block_time: Duration,
    pub round_delta: u64,
    pub ticket_price: Decimal256,
    pub max_holders: u8,
    pub prize_distribution: [Decimal256; 6],
    pub target_award: Decimal256,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub instant_withdrawal_fee: Decimal256,
    pub unbonding_period: Duration,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
    pub total_tickets: Uint256,
    pub total_reserve: Decimal256,
    pub award_available: Decimal256,
    pub current_lottery: u64,
    pub next_lottery_time: Expiration,
    pub next_lottery_exec_time: Expiration,
    pub next_epoch: Expiration,
    pub last_reward_updated: u64,
    pub global_reward_index: Decimal256,
    pub glow_emission_rate: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Pool {
    pub total_deposits: Decimal256,
    pub total_sponsor_amount: Decimal256,
    pub lottery_deposits: Decimal256,
    pub lottery_shares: Decimal256,
    pub deposit_shares: Decimal256,
    pub sponsor_shares: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorInfo {
    pub deposit_amount: Decimal256,
    pub shares: Decimal256,
    pub reward_index: Decimal256,
    pub pending_rewards: Decimal256,
    pub tickets: Vec<String>,
    pub unbonding_info: Vec<Claim>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct SponsorInfo {
    pub amount: Decimal256,
    pub shares: Decimal256,
    pub pending_rewards: Decimal256,
    pub reward_index: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryInfo {
    pub rand_round: u64,
    pub sequence: String,
    pub awarded: bool,
    pub total_prizes: Decimal256,
    pub number_winners: [u32; 6],
    pub page: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PrizeInfo {
    pub claimed: bool,
    pub matches: [u32; 6],
}
impl Default for PrizeInfo {
    fn default() -> Self {
        PrizeInfo {
            claimed: false,
            matches: [0; 6],
        }
    }
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
            rand_round: 0,
            sequence: "".to_string(),
            awarded: false,
            total_prizes: Decimal256::zero(),
            number_winners: [0; 6],
            page: "".to_string(),
        },
    }
}

pub fn store_depositor_info(
    storage: &mut dyn Storage,
    depositor: &Addr,
    depositor_info: &DepositorInfo,
) -> StdResult<()> {
    bucket(storage, PREFIX_DEPOSIT).save(depositor.as_bytes(), depositor_info)
}

pub fn read_depositor_info(storage: &dyn Storage, depositor: &Addr) -> DepositorInfo {
    match bucket_read(storage, PREFIX_DEPOSIT).load(depositor.as_bytes()) {
        Ok(v) => v,
        _ => DepositorInfo {
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![],
        },
    }
}

pub fn store_sponsor_info(
    storage: &mut dyn Storage,
    sponsor: &Addr,
    sponsor_info: &SponsorInfo,
) -> StdResult<()> {
    bucket(storage, PREFIX_SPONSOR).save(sponsor.as_bytes(), sponsor_info)
}

pub fn read_sponsor_info(storage: &dyn Storage, sponsor: &Addr) -> SponsorInfo {
    match bucket_read(storage, PREFIX_SPONSOR).load(sponsor.as_bytes()) {
        Ok(v) => v,
        _ => SponsorInfo {
            amount: Decimal256::zero(),
            shares: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            reward_index: Decimal256::zero(),
        },
    }
}

pub fn read_depositors(
    deps: Deps,
    start_after: Option<Addr>,
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
            let depositor = String::from_utf8(k).unwrap();
            Ok(DepositorInfoResponse {
                depositor,
                deposit_amount: v.deposit_amount,
                shares: v.shares,
                reward_index: v.reward_index,
                pending_rewards: v.pending_rewards,
                tickets: v.tickets,
                unbonding_info: v.unbonding_info,
            })
        })
        .collect()
}

fn calc_range_start(start_after: Option<Addr>) -> Option<Vec<u8>> {
    start_after.map(|addr| {
        let mut v = addr.as_bytes().to_vec();
        v.push(1);
        v
    })
}

pub fn query_prizes(deps: Deps, address: &Addr, lottery_id: u64) -> StdResult<PrizeInfo> {
    let lottery_key = U64Key::from(lottery_id);
    PRIZES.load(deps.storage, (address, lottery_key))
}

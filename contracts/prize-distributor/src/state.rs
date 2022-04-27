use std::convert::TryInto;
use std::str::from_utf8;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Deps, Order, StdError, StdResult, Storage, Timestamp};

use cw0::{Duration, Expiration};
use cw_storage_plus::{Bound, Item, Map, U64Key};
use glow_protocol::prize_distributor::BoostConfig;

use glow_protocol::prize_distributor::NUM_PRIZE_BUCKETS;

pub const PREFIX_OPERATOR: &[u8] = b"operator";

pub const CONFIG: Item<Config> = Item::new("config");
pub const STATE: Item<State> = Item::new("state");
pub const PRIZES: Map<(U64Key, &Addr), PrizeInfo> = Map::new("prizes_v2");
pub const LOTTERIES: Map<U64Key, LotteryInfo> = Map::new("lo_v2");

// settings for pagination
const DEFAULT_LIMIT: u32 = 10;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    pub a_terra_contract: Addr,
    pub ve_contract: Addr,
    pub gov_contract: Addr,
    pub community_contract: Addr,
    pub distributor_contract: Addr,
    pub anchor_contract: Addr,
    pub oracle_contract: Addr,
    pub stable_denom: String,
    pub lottery_interval: u64,
    pub epoch_interval: Duration,
    pub block_time: Duration,
    pub round_delta: u64,
    pub prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS],
    pub reserve_factor: Decimal256,
    pub glow_prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    pub lotto_winner_boost_config: BoostConfig,
}

impl Config {
    pub fn contracts_registered(&self) -> bool {
        self.gov_contract != Addr::unchecked("")
            && self.community_contract != Addr::unchecked("")
            && self.distributor_contract != Addr::unchecked("")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
    pub total_reserve: Uint256,
    pub prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    pub current_lottery: u64,
    pub next_lottery_time: Timestamp,
    pub next_lottery_exec_time: Expiration,
    pub next_epoch: Expiration,
    pub last_lottery_execution_aust_exchange_rate: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryInfo {
    pub rand_round: u64,
    pub sequence: String,
    pub awarded: bool,
    pub timestamp: Timestamp,
    pub block_height: u64,
    pub prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    pub number_winners: [u32; NUM_PRIZE_BUCKETS],
    pub page: String,
    pub glow_prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    pub total_user_shares: Uint256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct PrizeInfo {
    pub claimed: bool,
    pub matches: [u32; NUM_PRIZE_BUCKETS],
}

pub fn store_lottery_info(
    storage: &mut dyn Storage,
    lottery_id: u64,
    lottery_info: &LotteryInfo,
) -> StdResult<()> {
    LOTTERIES.save(storage, U64Key::from(lottery_id), lottery_info)
}

pub fn read_lottery_info(storage: &dyn Storage, lottery_id: u64) -> LotteryInfo {
    match LOTTERIES.load(storage, U64Key::from(lottery_id)) {
        Ok(v) => v,
        _ => LotteryInfo {
            rand_round: 0,
            sequence: "".to_string(),
            awarded: false,
            timestamp: Timestamp::from_seconds(0),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            number_winners: [0; NUM_PRIZE_BUCKETS],
            page: "".to_string(),
            glow_prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            block_height: 0,
            total_user_shares: Uint256::zero(),
        },
    }
}

pub fn read_prize(deps: Deps, address: &Addr, lottery_id: u64) -> StdResult<PrizeInfo> {
    let lottery_key = U64Key::from(lottery_id);
    PRIZES.load(deps.storage, (lottery_key, address))
}

pub fn read_lottery_prizes(
    deps: Deps,
    lottery_id: u64,
    start_after: Option<Addr>,
    limit: Option<u32>,
) -> StdResult<Vec<(Addr, PrizeInfo)>> {
    let lottery_key = U64Key::from(lottery_id);

    let start = start_after.map(|a| Bound::Exclusive(a.as_bytes().to_vec()));
    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;

    PRIZES
        .prefix(lottery_key)
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (k, v) = item?;

            let addr = Addr::unchecked(from_utf8(&k)?);

            Ok((addr, v))
        })
        .collect::<StdResult<Vec<_>>>()
}

// helper to deserialize the length
pub fn parse_length(value: &[u8]) -> StdResult<usize> {
    Ok(u16::from_be_bytes(
        value
            .try_into()
            .map_err(|_| StdError::generic_err("Could not read 2 byte length"))?,
    )
    .into())
}

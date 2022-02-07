use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Deps, Order, StdResult, Storage, Timestamp};
use cosmwasm_storage::{bucket, bucket_read, ReadonlyBucket};
use cw0::{Duration, Expiration};
use cw_storage_plus::{Item, Map, U64Key};
use glow_protocol::lotto::{BoostConfig, Claim, DepositorInfoResponse};

use glow_protocol::lotto::NUM_PRIZE_BUCKETS;

const OLD_PREFIX_LOTTERY: &[u8] = b"lottery";
const PREFIX_DEPOSIT: &[u8] = b"depositor";
const PREFIX_SPONSOR: &[u8] = b"sponsor";

pub const CONFIG: Item<Config> = Item::new("config");
pub const OLDCONFIG: Item<OldConfig> = Item::new("config");
pub const STATE: Item<State> = Item::new("state");
pub const POOL: Item<Pool> = Item::new("pool");
pub const TICKETS: Map<&[u8], Vec<Addr>> = Map::new("tickets");
pub const PRIZES: Map<(&Addr, U64Key), PrizeInfo> = Map::new("prizes");

pub const LOTTERIES: Map<U64Key, LotteryInfo> = Map::new("lo_v2");

// settings for pagination
const DEFAULT_LIMIT: u32 = 10;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    pub a_terra_contract: Addr,
    pub gov_contract: Addr,
    pub community_contract: Addr,
    pub distributor_contract: Addr,
    pub anchor_contract: Addr,
    pub oracle_contract: Addr,
    pub stable_denom: String,
    pub lottery_interval: Duration,
    pub epoch_interval: Duration,
    pub block_time: Duration,
    pub round_delta: u64,
    pub ticket_price: Uint256,
    pub max_holders: u8,
    pub prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS],
    pub target_award: Uint256,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub instant_withdrawal_fee: Decimal256,
    pub unbonding_period: Duration,
    pub max_tickets_per_depositor: u64,
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
pub struct OldConfig {
    pub owner: Addr,
    pub a_terra_contract: Addr,
    pub gov_contract: Addr,
    pub distributor_contract: Addr,
    pub anchor_contract: Addr,
    pub oracle_contract: Addr,
    pub stable_denom: String,
    pub lottery_interval: Duration,
    pub epoch_interval: Duration,
    pub block_time: Duration,
    pub round_delta: u64,
    pub ticket_price: Uint256,
    pub max_holders: u8,
    pub prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS],
    pub target_award: Uint256,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub instant_withdrawal_fee: Decimal256,
    pub unbonding_period: Duration,
}

impl OldConfig {
    pub fn contracts_registered(&self) -> bool {
        self.gov_contract != Addr::unchecked("") && self.distributor_contract != Addr::unchecked("")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
    pub total_tickets: Uint256,
    pub total_reserve: Uint256,
    pub prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    pub current_lottery: u64,
    pub next_lottery_time: Expiration,
    pub next_lottery_exec_time: Expiration,
    pub next_epoch: Expiration,
    pub last_reward_updated: u64,
    pub global_reward_index: Decimal256,
    pub glow_emission_rate: Decimal256,
}

// Note: total_user_lottery_deposits and total_sponsor_lottery_deposits
// could be merged into total_lottery_deposits without changing the functionality of the code
// but keeping them separate allows for a better understanding of the deposit to sponsor distribution
// as well as makes the code more flexible for future changes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Pool {
    // Sum of all user lottery deposits
    // This is used for
    // - checking for pool solvency
    // - calculating the global reward index
    // - calculating the amount to redeem when executing a lottery
    pub total_user_lottery_deposits: Uint256,
    // Sum of all user savings aust
    // This is used for:
    // - checking for pool solvency
    // - tracking the amount of aust reserved for savings
    pub total_user_savings_aust: Uint256,
    // Sum of all sponsor lottery deposits
    // which equals the sum of sponsor deposits
    // because all sponsor deposits go entirely towards the lottery
    // This is used for:
    // - checking for pool solvency
    // - calculating the global reward index
    // - calculating the amount to redeem when executing a lottery
    pub total_sponsor_lottery_deposits: Uint256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorInfo {
    // Cumulative value of the depositor's lottery deposits
    // The sums of all depositor deposit amounts equals total_user_lottery_deposits
    // This is used for:
    // - calculating how many tickets the user should have access to
    // - computing the depositor's deposit reward
    // - calculating the depositor's balance (how much they can withdraw)
    pub lottery_deposit: Uint256,
    // Amount of aust in the users savings account
    // This is used for:
    // - calculating the depositor's balance (how much they can withdraw)
    pub savings_aust: Uint256,
    // Reward index is used for tracking and calculating the depositor's rewards
    pub reward_index: Decimal256,
    // Stores the amount rewards that are available for the user to claim.
    pub pending_rewards: Decimal256,
    // The number of tickets the user owns.
    pub tickets: Vec<String>,
    // Stores information on the user's unbonding claims.
    pub unbonding_info: Vec<Claim>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct SponsorInfo {
    // Cumulative value of the sponsor's deposits.
    // The sums of all sponsor amounts equals total_sponsor_deposits
    // This is used for:
    // - calculating the sponsor's balance (how much they can withdraw)
    pub lottery_deposit: Uint256,
    // Reward index is used for tracking and calculating the sponsor's rewards
    pub pending_rewards: Decimal256,
    // Stores the amount rewards that are available for the sponsor to claim.
    pub reward_index: Decimal256,
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
    pub total_user_lottery_deposits: Uint256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct OldLotteryInfo {
    pub rand_round: u64,
    pub sequence: String,
    pub awarded: bool,
    pub timestamp: u64,
    pub prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    pub number_winners: [u32; NUM_PRIZE_BUCKETS],
    pub page: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct PrizeInfo {
    pub claimed: bool,
    pub matches: [u32; NUM_PRIZE_BUCKETS],
    pub lottery_deposit: Uint256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct OldPrizeInfo {
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
            total_user_lottery_deposits: Uint256::zero(),
        },
    }
}

pub fn old_read_lottery_info(storage: &dyn Storage, lottery_id: u64) -> OldLotteryInfo {
    match bucket_read(storage, OLD_PREFIX_LOTTERY).load(&lottery_id.to_be_bytes()) {
        Ok(v) => v,
        _ => OldLotteryInfo {
            rand_round: 0,
            sequence: "".to_string(),
            awarded: false,
            timestamp: 0,
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            number_winners: [0; NUM_PRIZE_BUCKETS],
            page: "".to_string(),
        },
    }
}

pub fn old_remove_lottery_info(storage: &mut dyn Storage, lottery_id: u64) {
    bucket::<OldLotteryInfo>(storage, OLD_PREFIX_LOTTERY).remove(&lottery_id.to_be_bytes())
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
            lottery_deposit: Uint256::zero(),
            savings_aust: Uint256::zero(),
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
            lottery_deposit: Uint256::zero(),
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

    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;
    let start = calc_range_start(start_after);

    liability_bucket
        .range(start.as_deref(), None, Order::Ascending)
        .take(limit)
        .map(|elem| {
            let (k, v) = elem?;
            let depositor = String::from_utf8(k).unwrap();
            Ok(DepositorInfoResponse {
                depositor,
                lottery_deposit: v.lottery_deposit,
                savings_aust: v.savings_aust,
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

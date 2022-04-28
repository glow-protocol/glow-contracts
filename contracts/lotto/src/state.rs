use std::convert::TryInto;
use std::str::from_utf8;

use glow_protocol::prize_distributor::PrizeInfo;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Deps, Order, StdError, StdResult, Storage, Timestamp};
use cosmwasm_storage::{bucket, bucket_read, ReadonlyBucket};
use cw0::{Duration, Expiration};
use cw_storage_plus::{Bound, Item, Map, PrimaryKey, SnapshotMap, U64Key};
use glow_protocol::lotto::{
    Claim, DepositorData, DepositorInfo, DepositorInfoResponse, DepositorStatsInfo,
    DepositorStatsResponse, RewardEmissionsIndex,
};

use glow_protocol::lotto::NUM_PRIZE_BUCKETS;

pub const PREFIX_SPONSOR: &[u8] = b"sponsor";
pub const PREFIX_OPERATOR: &[u8] = b"operator";

pub const CONFIG: Item<Config> = Item::new("config");
pub const OLDCONFIG: Item<OldConfig> = Item::new("config");
pub const STATE: Item<State> = Item::new("state");
pub const OLDSTATE: Item<OldState> = Item::new("state");
pub const POOL: Item<Pool> = Item::new("pool");
pub const OLDPOOL: Item<OldPool> = Item::new("pool");

pub const TICKETS: Map<&[u8], Vec<Addr>> = Map::new("tickets");

// Old stuff, migrated to prize distributor contract
pub const OLD_PRIZES: Map<(&Addr, U64Key), PrizeInfo> = Map::new("prizes");
pub const OLD_PREFIX_LOTTERY: &[u8] = b"lottery";

pub const OLD_PREFIX_DEPOSIT: &[u8] = b"depositor";
pub const DEPOSITOR_DATA: Map<&Addr, DepositorData> = Map::new("depositor_data");
pub const DEPOSITOR_STATS: SnapshotMap<&Addr, DepositorStatsInfo> = SnapshotMap::new(
    "depositor_stats",
    "depositor_stats__checkpoint",
    "depositor_stats__changelog",
    cw_storage_plus::Strategy::EveryBlock,
);

use crate::helpers::{
    vec_binary_tickets_to_vec_string_tickets, vec_string_tickets_to_vec_binary_tickets,
};

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
    pub ticket_price: Uint256,
    pub max_holders: u8,
    pub split_factor: Decimal256,
    pub instant_withdrawal_fee: Decimal256,
    pub unbonding_period: Duration,
    pub max_tickets_per_depositor: u64,
    pub paused: bool,
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
    pub operator_reward_emission_index: RewardEmissionsIndex,
    pub sponsor_reward_emission_index: RewardEmissionsIndex,
    pub last_lottery_execution_aust_exchange_rate: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct OldState {
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Pool {
    // This is the amount of aust which belongs to users
    // It is equal to the cumulative amount of aust deposited by all users
    // minus the cumulative amount of aust withdrawn by all users
    // minut user aust redeemed when executing the lottery.
    pub total_user_aust: Uint256,
    // This is the sum of shares across all depositors.
    // It starts out as equal to total_user_aust,
    // but total_user_aust smaller during each lottery execution
    // while total_user_shares stays the same
    pub total_user_shares: Uint256,
    // Sum of all sponsor lottery deposits
    // which equals the sum of sponsor long term deposits
    // because all sponsor long term deposits go entirely towards the lottery
    // This is used for:
    // - calculating the global sponsor reward index
    // - calculating the amount sponsored aust to redeem when executing a lottery
    pub total_sponsor_lottery_deposits: Uint256,
    // Sum of all user lottery shares that have operators
    // This is used for
    // - calculating the global operator reward index
    pub total_operator_shares: Uint256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct OldPool {
    pub total_user_lottery_deposits: Uint256,
    pub total_user_savings_aust: Uint256,
    pub total_sponsor_lottery_deposits: Uint256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct OldDepositorInfo {
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
    // Stores the amount rewards that are available for the sponsor to claim.
    pub pending_rewards: Decimal256,
    // Reward index is used for tracking and calculating the sponsor's rewards
    pub reward_index: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct OperatorInfo {
    // Cumulative value of the operators' shares.
    // The sums of all operator share amounts equals total_operator_shares
    // This is used for:
    // - calculating the operator's reward
    pub shares: Uint256,
    // Stores the amount rewards that are available for the operator to claim.
    pub pending_rewards: Decimal256,
    // Reward index is used for tracking and calculating the operator's rewards
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
    pub total_user_shares: Uint256,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct OldLotteryInfo {
    pub rand_round: u64,
    pub sequence: String,
    pub awarded: bool,
    pub timestamp: u64,
    pub prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    pub number_winners: [u32; NUM_PRIZE_BUCKETS],
    pub page: String,
}

pub fn old_read_lottery_info(
    storage: &dyn Storage,
    lottery_id: u64,
) -> StdResult<Option<OldLotteryInfo>> {
    bucket_read(storage, OLD_PREFIX_LOTTERY).may_load(&lottery_id.to_be_bytes())
}

pub fn old_remove_lottery_info(storage: &mut dyn Storage, lottery_id: u64) {
    bucket::<OldLotteryInfo>(storage, OLD_PREFIX_LOTTERY).remove(&lottery_id.to_be_bytes())
}

pub fn store_depositor_info(
    storage: &mut dyn Storage,
    depositor: &Addr,
    depositor_info: DepositorInfo,
    height: u64,
) -> StdResult<()> {
    // Get the number of tickets
    let num_tickets = depositor_info.tickets.len();

    // Get the tickets in binary form
    let vec_binary_tickets = vec_string_tickets_to_vec_binary_tickets(depositor_info.tickets)?;

    let depositor_data = DepositorData {
        vec_binary_tickets,
        unbonding_info: depositor_info.unbonding_info,
    };

    let depositor_stats_info = DepositorStatsInfo {
        shares: depositor_info.shares,
        num_tickets,
        operator_addr: depositor_info.operator_addr,
    };

    DEPOSITOR_DATA.save(storage, depositor, &depositor_data)?;

    DEPOSITOR_STATS.save(storage, depositor, &depositor_stats_info, height)?;

    Ok(())
}

pub fn old_remove_depositor_info(storage: &mut dyn Storage, depositor: &Addr) {
    bucket::<OldDepositorInfo>(storage, OLD_PREFIX_DEPOSIT).remove(depositor.as_bytes())
}

/// Store depositor stats
/// Does *not* store changes to num_tickets
/// in order to ensure that num_tickets always stays in sync with DepositorData
pub fn store_depositor_stats(
    storage: &mut dyn Storage,
    depositor: &Addr,
    depositor_stats: DepositorStatsInfo,
    height: u64,
) -> StdResult<()> {
    let update_stats = |maybe_stats: Option<DepositorStatsInfo>| -> StdResult<DepositorStatsInfo> {
        let stats = maybe_stats.unwrap_or(DepositorStatsInfo {
            shares: Uint256::zero(),
            num_tickets: 0,
            operator_addr: Addr::unchecked(""),
        });
        if stats.num_tickets != depositor_stats.num_tickets {
            return Err(StdError::generic_err(
                "Can't change num tickets and save depositor stats",
            ));
        }
        Ok(depositor_stats)
    };

    DEPOSITOR_STATS.update(storage, depositor, height, update_stats)?;

    Ok(())
}

pub fn old_read_depositor_info(storage: &dyn Storage, depositor: &Addr) -> OldDepositorInfo {
    match bucket_read(storage, OLD_PREFIX_DEPOSIT).load(depositor.as_bytes()) {
        Ok(v) => v,
        _ => OldDepositorInfo {
            lottery_deposit: Uint256::zero(),
            savings_aust: Uint256::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![],
        },
    }
}

pub fn read_depositor_info(storage: &dyn Storage, depositor: &Addr) -> DepositorInfo {
    let depositor_data = match DEPOSITOR_DATA.load(storage, depositor) {
        Ok(v) => v,
        _ => DepositorData {
            vec_binary_tickets: vec![],
            unbonding_info: vec![],
        },
    };

    let depositor_stats_info = match DEPOSITOR_STATS.load(storage, depositor) {
        Ok(v) => v,
        _ => DepositorStatsInfo {
            shares: Uint256::zero(),
            num_tickets: 0,
            operator_addr: Addr::unchecked(""),
        },
    };

    let vec_string_tickets =
        vec_binary_tickets_to_vec_string_tickets(depositor_data.vec_binary_tickets);

    DepositorInfo {
        // DepositorData
        tickets: vec_string_tickets,
        unbonding_info: depositor_data.unbonding_info,

        // DepositorStats
        shares: depositor_stats_info.shares,
        operator_addr: depositor_stats_info.operator_addr,
    }
}

pub fn read_depositor_stats(storage: &dyn Storage, depositor: &Addr) -> DepositorStatsInfo {
    match DEPOSITOR_STATS.load(storage, depositor) {
        Ok(v) => v,
        _ => DepositorStatsInfo {
            shares: Uint256::zero(),
            num_tickets: 0,
            operator_addr: Addr::unchecked(""),
        },
    }
}

pub fn read_depositor_stats_at_height(
    storage: &dyn Storage,
    depositor: &Addr,
    height: u64,
) -> DepositorStatsInfo {
    match DEPOSITOR_STATS.may_load_at_height(storage, depositor, height) {
        Ok(Some(v)) => v,
        _ => DepositorStatsInfo {
            shares: Uint256::zero(),
            num_tickets: 0,
            operator_addr: Addr::unchecked(""),
        },
    }
}

pub fn read_depositor_data(storage: &dyn Storage, depositor: &Addr) -> DepositorData {
    match DEPOSITOR_DATA.load(storage, depositor) {
        Ok(v) => v,
        _ => DepositorData {
            vec_binary_tickets: vec![],
            unbonding_info: vec![],
        },
    }
}

pub fn store_sponsor_info(
    storage: &mut dyn Storage,
    sponsor: &Addr,
    sponsor_info: SponsorInfo,
) -> StdResult<()> {
    bucket(storage, PREFIX_SPONSOR).save(sponsor.as_bytes(), &sponsor_info)
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

pub fn store_operator_info(
    storage: &mut dyn Storage,
    operator: &Addr,
    operator_info: OperatorInfo,
) -> StdResult<()> {
    bucket(storage, PREFIX_OPERATOR).save(operator.as_bytes(), &operator_info)
}

pub fn read_operator_info(storage: &dyn Storage, operator: &Addr) -> OperatorInfo {
    match bucket_read(storage, PREFIX_OPERATOR).load(operator.as_bytes()) {
        Ok(v) => v,
        _ => OperatorInfo {
            shares: Uint256::zero(),
            pending_rewards: Decimal256::zero(),
            reward_index: Decimal256::zero(),
        },
    }
}

pub fn read_depositors_info(
    deps: Deps,
    start_after: Option<Addr>,
    limit: Option<u32>,
) -> StdResult<Vec<DepositorInfoResponse>> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;
    let start = start_after.map(|v| Bound::Exclusive(v.as_bytes().to_vec()));

    DEPOSITOR_STATS
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|elem| {
            let (k, v) = elem?;
            let depositor = String::from_utf8(k).unwrap();
            let depositor_addr = Addr::unchecked(&depositor);
            let depositor_data = read_depositor_data(deps.storage, &depositor_addr);
            let vec_string_tickets =
                vec_binary_tickets_to_vec_string_tickets(depositor_data.vec_binary_tickets);
            Ok(DepositorInfoResponse {
                depositor,
                shares: v.shares,
                tickets: vec_string_tickets,
                unbonding_info: depositor_data.unbonding_info,
            })
        })
        .collect()
}

pub fn read_depositors_stats(
    deps: Deps,
    start_after: Option<Addr>,
    limit: Option<u32>,
) -> StdResult<Vec<DepositorStatsResponse>> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;
    let start = start_after.map(|v| Bound::Exclusive(v.as_bytes().to_vec()));

    DEPOSITOR_STATS
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|elem| {
            let (k, v) = elem?;
            let depositor = String::from_utf8(k).unwrap();
            Ok(DepositorStatsResponse {
                depositor,
                shares: v.shares,
                num_tickets: v.num_tickets,
            })
        })
        .collect()
}

pub fn old_read_depositors(
    deps: Deps,
    start_after: Option<Addr>,
    limit: Option<u32>,
) -> StdResult<Vec<(Addr, OldDepositorInfo)>> {
    let liability_bucket: ReadonlyBucket<OldDepositorInfo> =
        bucket_read(deps.storage, OLD_PREFIX_DEPOSIT);

    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;
    let start = old_calc_range_start(start_after);

    liability_bucket
        .range(start.as_deref(), None, Order::Ascending)
        .take(limit)
        .map(|elem| {
            let (k, v) = elem?;
            let depositor = String::from_utf8(k).unwrap();
            let depositor_addr = Addr::unchecked(&depositor);

            Ok((
                depositor_addr,
                OldDepositorInfo {
                    lottery_deposit: v.lottery_deposit,
                    savings_aust: v.savings_aust,
                    reward_index: v.reward_index,
                    pending_rewards: v.pending_rewards,
                    tickets: v.tickets,
                    unbonding_info: v.unbonding_info,
                },
            ))
        })
        .collect()
}

pub fn old_read_prize_infos(
    deps: Deps,
    start_after: Option<(String, u64)>,
    limit: Option<u32>,
) -> StdResult<Vec<(Addr, u64, PrizeInfo)>> {
    let min_bound = start_after.map(|(start_after_depositor, start_after_lottery_id)| {
        Bound::exclusive(
            (
                Addr::unchecked(start_after_depositor),
                U64Key::from(start_after_lottery_id),
            )
                .joined_key(),
        )
    });

    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;

    let old_prizes = OLD_PRIZES
        .range(deps.storage, min_bound, None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (mut k, v) = item?;

            // https://github.com/CosmWasm/cw-plus/issues/466

            // Gets the length prefix from the composite key
            let mut tu = k.split_off(2);

            // Calculate the size of the first key in the composite key
            // using the length prefix
            let t_len = parse_length(&k)?;

            // Split tu into the first and second key.
            // u is the second key, and tu is the first key
            let u = tu.split_off(t_len);

            // Extract address from the first key
            let addr = Addr::unchecked(from_utf8(&tu)?);

            let lottery_id = u64::from_be_bytes(u.try_into().unwrap());

            // Return
            Ok((addr, lottery_id, v))
        })
        .collect::<StdResult<Vec<_>>>()?;

    Ok(old_prizes)
}

fn old_calc_range_start(start_after: Option<Addr>) -> Option<Vec<u8>> {
    start_after.map(|addr| {
        let mut v = addr.as_bytes().to_vec();
        v.push(1);
        v
    })
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

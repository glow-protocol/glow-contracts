use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Timestamp, Uint128};
use cw0::{Duration, Expiration};

use crate::lotto::{OldLotteryInfo, NUM_PRIZE_BUCKETS};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct BoostConfig {
    pub base_multiplier: Decimal256,
    pub max_multiplier: Decimal256,
    pub total_voting_power_weight: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: String,
    pub stable_denom: String,                                // uusd
    pub anchor_contract: String,                             // anchor money market address
    pub aterra_contract: String,                             // aterra auusd contract address
    pub oracle_contract: String,                             // oracle address
    pub lottery_interval: u64,                               // time between lotteries
    pub epoch_interval: u64, // time between executing epoch operations
    pub block_time: u64,     // number of blocks (or time) lottery is blocked while is executed
    pub round_delta: u64,    // number of rounds of security to get oracle rand
    pub prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS], // distribution for awarding prizes to winning tickets
    pub reserve_factor: Decimal256, // % of the prize that goes to the reserve fund
    pub initial_lottery_execution: u64, // time in seconds for the first Lotto execution
    pub glow_prize_buckets: [Uint256; NUM_PRIZE_BUCKETS], // glow to be awarded as a bonus to lottery winners
    pub lotto_winner_boost_config: Option<BoostConfig>, // the boost config to apply to glow emissions for lotto winners
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Register Contracts contract address - restricted to owner
    RegisterContracts {
        /// Gov contract tracks ve balances
        gov_contract: String,
        /// Community treasury contract that accrues and manages protocol fees
        community_contract: String,
        /// Faucet contract to drip GLOW token to users and update Glow emission rate
        distributor_contract: String,
        /// veGLOW contract for calculating boost multipliers
        ve_contract: String,
        /// Savings contract for redeeming prize funds
        savings_contract: String,
    },
    InjectStartingState {
        prizes: Vec<(Addr, u64, PrizeInfo)>,
        lotteries: Vec<OldLotteryInfo>,
        prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    },
    /// Update contract configuration - restricted to owner
    UpdateConfig {
        owner: Option<String>,
        oracle_addr: Option<String>,
        reserve_factor: Option<Decimal256>,
        epoch_interval: Option<u64>,
        paused: Option<bool>,
        lotto_winner_boost_config: Option<BoostConfig>,
    },
    /// Update lottery configuration - restricted to owner
    UpdateLotteryConfig {
        lottery_interval: Option<u64>,
        block_time: Option<u64>,
        ticket_price: Option<Uint256>,
        prize_distribution: Option<[Decimal256; NUM_PRIZE_BUCKETS]>,
        round_delta: Option<u64>,
    },
    /// Claims pending lottery prizes for a given list of lottery ids
    ClaimLottery { lottery_ids: Vec<u64> },
    /// First step on the lottery execution. Sets oracle round number
    ExecuteLottery {},
    /// Second step (paginated) on the lottery execution. Sets winner sequence and
    /// stores winning sequences
    ExecutePrize { limit: Option<u32> },
    /// Updates rewards emission rate and transfer outstanding reserve to gov
    ExecuteEpochOps {},
}

/// Migration message
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Lotto contract configuration
    Config {},
    /// Current state
    State { block_height: Option<u64> },
    /// Lottery information by lottery id
    LotteryInfo { lottery_id: Option<u64> },
    /// Prizes for a given address on a given lottery id
    PrizeInfo { address: String, lottery_id: u64 },
    /// Prizes for a given lottery id
    LotteryPrizeInfos {
        lottery_id: u64,
        start_after: Option<String>,
        limit: Option<u32>,
    },
    /// Prize Distribution Pending
    PrizeDistributionPending {},
    /// Get the lottery balance. This is the amount that would be distributed in prizes if the lottery were run right
    /// now.
    LotteryBalance {},
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: String,
    pub stable_denom: String,
    pub a_terra_contract: String,
    pub anchor_contract: String,
    pub gov_contract: String,
    pub ve_contract: String,
    pub community_contract: String,
    pub distributor_contract: String,
    pub savings_contract: String,
    pub lottery_interval: u64,
    pub epoch_interval: Duration,
    pub block_time: Duration,
    pub round_delta: u64,
    pub prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS],
    pub reserve_factor: Decimal256,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StateResponse {
    pub total_reserve: Uint256,
    pub prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    pub current_lottery: u64,
    pub next_lottery_time: Timestamp,
    pub next_lottery_exec_time: Expiration,
    pub next_epoch: Expiration,
    pub last_lottery_execution_aust_exchange_rate: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryInfoResponse {
    pub lottery_id: u64,
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PrizeInfoResponse {
    pub holder: Addr,
    pub lottery_id: u64,
    pub claimed: bool,
    pub matches: [u32; NUM_PRIZE_BUCKETS],
    pub won_ust: Uint128,
    pub won_glow: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PrizeInfosResponse {
    pub prize_infos: Vec<PrizeInfoResponse>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryBalanceResponse {
    pub value_of_user_aust_to_be_redeemed_for_lottery: Uint256,
    pub user_aust_to_redeem: Uint256,
    pub value_of_sponsor_aust_to_be_redeemed_for_lottery: Uint256,
    pub sponsor_aust_to_redeem: Uint256,
    pub aust_to_redeem: Uint256,
    pub aust_to_redeem_value: Uint256,
    pub prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct PrizeInfo {
    pub claimed: bool,
    pub matches: [u32; NUM_PRIZE_BUCKETS],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct PrizeDistributionPendingResponse {
    pub prize_distribution_pending: bool,
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
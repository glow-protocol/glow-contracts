use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Timestamp, Uint128};
use cw0::{Duration, Expiration};

pub const TICKET_LENGTH: usize = 6;
pub const NUM_PRIZE_BUCKETS: usize = TICKET_LENGTH + 1;

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
    pub epoch_interval: u64,   // time between executing epoch operations
    pub block_time: u64,       // number of blocks (or time) lottery is blocked while is executed
    pub round_delta: u64,      // number of rounds of security to get oracle rand
    pub ticket_price: Uint256, // prize of a ticket in stable_denom
    pub max_holders: u8,       // Max number of holders per ticket
    pub prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS], // distribution for awarding prizes to winning tickets
    pub target_award: Uint256, // target award used in deposit rewards computation
    pub reserve_factor: Decimal256, // % of the prize that goes to the reserve fund
    pub split_factor: Decimal256, // what % of interest goes to saving and which one lotto pool
    pub instant_withdrawal_fee: Decimal256, // % to be deducted as a fee for instant withdrawals
    pub unbonding_period: u64, // unbonding period after regular withdrawals from pool
    pub initial_emission_rate: Decimal256, // initial GLOW emission rate for depositor rewards
    pub initial_lottery_execution: u64, // time in seconds for the first Lotto execution
    pub max_tickets_per_depositor: u64, // the maximum number of tickets that a depositor can hold
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
    },
    /// Update contract configuration - restricted to owner
    UpdateConfig {
        owner: Option<String>,
        oracle_addr: Option<String>,
        reserve_factor: Option<Decimal256>,
        instant_withdrawal_fee: Option<Decimal256>,
        unbonding_period: Option<u64>,
        epoch_interval: Option<u64>,
        max_holders: Option<u8>,
        max_tickets_per_depositor: Option<u64>,
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
    /// Deposit amount of stable into the pool
    Deposit { encoded_tickets: String },
    /// Deposit amount of stable into the pool in the name of the recipient
    Gift {
        encoded_tickets: String,
        recipient: String,
    },
    /// Sponsor the pool. If award is true, sponsor the award available directly
    Sponsor {
        award: Option<bool>,
        prize_distribution: Option<[Decimal256; NUM_PRIZE_BUCKETS]>,
    },
    /// Withdraws the sponsorship of the sender
    SponsorWithdraw {},
    /// Withdraws amount from the pool. If amount is None, it tries to withdraw all
    /// the pooled funds of the sender. If instant true, incurs on withdrawal fee.
    Withdraw {
        amount: Option<Uint128>,
        instant: Option<bool>,
    },
    /// Claim unbonded withdrawals
    Claim {},
    /// Claims pending lottery prizes for a given list of lottery ids
    ClaimLottery { lottery_ids: Vec<u64> },
    /// Claims pending depositor rewards
    ClaimRewards {},
    /// First step on the lottery execution. Sets oracle round number
    ExecuteLottery {},
    /// Second step (paginated) on the lottery execution. Sets winner sequence and
    /// stores winning sequences
    ExecutePrize { limit: Option<u32> },
    /// Updates rewards emission rate and transfer outstanding reserve to gov
    ExecuteEpochOps {},
    /// Handles the migrate loop
    MigrateOldDepositors { limit: Option<u32> },
}

/// Migration message
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {
    pub glow_prize_buckets: [Uint256; NUM_PRIZE_BUCKETS], // glow to be awarded as a bonus to lottery winners
    pub max_tickets_per_depositor: u64, // the maximum number of tickets that a depositor can hold
    pub community_contract: String,     // Glow community contract address
    pub lotto_winner_boost_config: Option<BoostConfig>, // The boost config to apply to glow emissions for lotto winners
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Lotto contract configuration
    Config {},
    /// Current state. If block_height is provided, return current depositor rewards
    State { block_height: Option<u64> },
    /// Lotto pool current state. Savings aust and lottery deposits.
    Pool {},
    /// Lottery information by lottery id
    LotteryInfo { lottery_id: Option<u64> },
    /// Ticket information by sequence. Returns a list of holders (addresses)
    TicketInfo { sequence: String },
    /// Prizes for a given address on a given lottery id
    PrizeInfo { address: String, lottery_id: u64 },
    /// Depositor information by address
    DepositorInfo { address: String },
    /// Depositor stats by address
    DepositorStats { address: String },
    /// List (paginated) of DepositorInfo
    DepositorsInfo {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    /// List (paginated) of DepositorStats
    DepositorsStats {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    /// Sponsor information by address
    Sponsor { address: String },
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
    pub community_contract: String,
    pub distributor_contract: String,
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
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StateResponse {
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

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PoolResponse {
    pub total_user_lottery_deposits: Uint256,
    pub total_user_savings_aust: Uint256,
    pub total_sponsor_lottery_deposits: Uint256,
}

// We define a custom struct for each query response
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
    pub total_user_lottery_deposits: Uint256,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorInfoResponse {
    pub depositor: String,
    pub lottery_deposit: Uint256,
    pub savings_aust: Uint256,
    pub reward_index: Decimal256,
    pub pending_rewards: Decimal256,
    pub tickets: Vec<String>,
    pub unbonding_info: Vec<Claim>,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorStatsResponse {
    pub depositor: String,
    pub lottery_deposit: Uint256,
    pub savings_aust: Uint256,
    pub reward_index: Decimal256,
    pub pending_rewards: Decimal256,
    pub num_tickets: usize,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct SponsorInfoResponse {
    pub sponsor: String,
    pub lottery_deposit: Uint256,
    pub reward_index: Decimal256,
    pub pending_rewards: Decimal256,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorsInfoResponse {
    pub depositors: Vec<DepositorInfoResponse>,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorsStatsResponse {
    pub depositors: Vec<DepositorStatsResponse>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Claim {
    pub amount: Uint256,
    pub release_at: Expiration,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct TicketInfoResponse {
    pub holders: Vec<Addr>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PrizeInfoResponse {
    pub holder: Addr,
    pub lottery_id: u64,
    pub claimed: bool,
    pub matches: [u32; NUM_PRIZE_BUCKETS],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryBalanceResponse {
    pub lottery_balance: Uint256,
}

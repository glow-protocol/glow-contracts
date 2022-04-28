use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Uint128};
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
pub struct RewardEmissionsIndex {
    pub last_reward_updated: u64,
    pub global_reward_index: Decimal256,
    pub glow_emission_rate: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: String,
    pub stable_denom: String,                            // uusd
    pub anchor_contract: String,                         // anchor money market address
    pub aterra_contract: String,                         // aterra auusd contract address
    pub oracle_contract: String,                         // oracle address
    pub lottery_interval: u64,                           // time between lotteries
    pub ticket_price: Uint256,                           // prize of a ticket in stable_denom
    pub max_holders: u8,                                 // Max number of holders per ticket
    pub split_factor: Decimal256, // what % of interest goes to saving and which one lotto pool
    pub instant_withdrawal_fee: Decimal256, // % to be deducted as a fee for instant withdrawals
    pub unbonding_period: u64,    // unbonding period after regular withdrawals from pool
    pub initial_operator_glow_emission_rate: Decimal256, // initial GLOW emission rate for operator rewards
    pub initial_sponsor_glow_emission_rate: Decimal256, // initial GLOW emission rate for sponsor rewards
    pub max_tickets_per_depositor: u64, // the maximum number of tickets that a depositor can hold
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
    },
    /// Update contract configuration - restricted to owner
    UpdateConfig {
        owner: Option<String>,
        oracle_addr: Option<String>,
        instant_withdrawal_fee: Option<Decimal256>,
        unbonding_period: Option<u64>,
        max_holders: Option<u8>,
        max_tickets_per_depositor: Option<u64>,
        paused: Option<bool>,
        operator_glow_emission_rate: Option<Decimal256>,
        sponsor_glow_emission_rate: Option<Decimal256>,
    },
    /// Update lottery configuration - restricted to owner
    UpdateLotteryConfig { ticket_price: Option<Uint256> },
    /// Deposit amount of stable into the pool
    Deposit {
        encoded_tickets: String,
        operator: Option<String>,
    },
    /// Claim tickets
    ClaimTickets { encoded_tickets: String },
    /// Deposit amount of stable into the pool in the name of the recipient
    Gift {
        encoded_tickets: String,
        recipient: String,
        operator: Option<String>,
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
    pub ve_contract: String,                            // Glow ve token contract address
    pub operator_glow_emission_rate: Decimal256,        // The emission rate to set for operators
    pub sponsor_glow_emission_rate: Decimal256,         // The emission rate to set for sponsors
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Lotto contract configuration
    Config {},
    /// Current state
    State { block_height: Option<u64> },
    /// Lotto pool current state. Savings aust and lottery deposits.
    Pool {},
    /// Lottery information by lottery id
    LotteryInfo { lottery_id: Option<u64> },
    /// Ticket information by sequence. Returns a list of holders (addresses)
    TicketInfo { sequence: String },
    /// Prizes for a given address on a given lottery id
    PrizeInfo { address: String, lottery_id: u64 },
    /// Prizes for a given lottery id
    LotteryPrizeInfos {
        lottery_id: u64,
        start_after: Option<String>,
        limit: Option<u32>,
    },
    /// Depositor information by address
    DepositorInfo { address: String },
    /// Depositor stats by address
    DepositorStatsInfo { address: String },
    /// List (paginated) of DepositorInfo
    DepositorInfos {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    /// List (paginated) of DepositorStats
    DepositorsStatsInfos {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    /// Sponsor information by address
    Sponsor { address: String },
    /// Sponsor information by address
    Operator { address: String },
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
    pub ticket_price: Uint256,
    pub max_holders: u8,
    pub split_factor: Decimal256,
    pub instant_withdrawal_fee: Decimal256,
    pub unbonding_period: Duration,
    pub max_tickets_per_depositor: u64,
    pub paused: bool,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StateResponse {
    pub total_tickets: Uint256,
    pub operator_reward_emission_index: RewardEmissionsIndex,
    pub sponsor_reward_emission_index: RewardEmissionsIndex,
    pub last_lottery_execution_aust_exchange_rate: Decimal256,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PoolResponse {
    pub total_user_aust: Uint256,
    pub total_user_shares: Uint256,
    pub total_sponsor_lottery_deposits: Uint256,
    pub total_operator_shares: Uint256,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct OldLotteryInfoResponse {
    pub lottery_id: u64,
    pub rand_round: u64,
    pub sequence: String,
    pub awarded: bool,
    pub prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    pub number_winners: [u32; NUM_PRIZE_BUCKETS],
    pub page: String,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorInfoResponse {
    pub depositor: String,
    pub shares: Uint256,
    pub tickets: Vec<String>,
    pub unbonding_info: Vec<Claim>,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorStatsResponse {
    pub depositor: String,
    pub shares: Uint256,
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
pub struct OperatorInfoResponse {
    pub operator: String,
    pub shares: Uint256,
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
    pub won_ust: Uint128,
    pub won_glow: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PrizeInfosResponse {
    pub prize_infos: Vec<PrizeInfoResponse>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct AmountRedeemableForPrizesResponse {
    pub value_of_user_aust_to_be_redeemed_for_lottery: Uint256,
    pub user_aust_to_redeem: Uint256,
    pub value_of_sponsor_aust_to_be_redeemed_for_lottery: Uint256,
    pub sponsor_aust_to_redeem: Uint256,
    pub aust_to_redeem: Uint256,
    pub aust_to_redeem_value: Uint256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorStatsInfo {
    // This is the amount of shares the depositor owns out of total_user_aust
    // shares * total_user_aust / total_user_shares gives the amount of aust
    // that a depositor owns and has available to withdraw.
    pub shares: Uint256,
    // The number of tickets owned by the depositor
    pub num_tickets: usize,
    // Stores the address of the operator / referrer used by depositor.
    pub operator_addr: Addr,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorData {
    // The number of tickets the user owns.
    pub vec_binary_tickets: Vec<[u8; 3]>,
    // Stores information on the user's unbonding claims.
    pub unbonding_info: Vec<Claim>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorInfo {
    // This is the amount of shares the depositor owns out of total_user_aust
    // shares * total_user_aust / total_user_shares gives the amount of aust
    // that a depositor owns and has available to withdraw.
    pub shares: Uint256,
    // The number of tickets the user owns.
    pub tickets: Vec<String>,
    // Stores information on the user's unbonding claims.
    pub unbonding_info: Vec<Claim>,
    // Stores the address of the operator / referrer used by depositor.
    pub operator_addr: Addr,
}

impl DepositorInfo {
    pub fn operator_registered(&self) -> bool {
        self.operator_addr != Addr::unchecked("")
    }
}

pub struct ExecuteLotteryRedeemedAustInfo {
    pub value_of_user_aust_to_be_redeemed_for_lottery: Uint256,
    pub user_aust_to_redeem: Uint256,
    pub value_of_sponsor_aust_to_be_redeemed_for_lottery: Uint256,
    pub sponsor_aust_to_redeem: Uint256,
    pub aust_to_redeem: Uint256,
    pub aust_to_redeem_value: Uint256,
}

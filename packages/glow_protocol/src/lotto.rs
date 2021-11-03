use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Uint128};
use cw0::{Duration, Expiration};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: String,
    pub stable_denom: String,                // uusd
    pub anchor_contract: String,             // anchor money market address
    pub aterra_contract: String,             // aterra auusd contract address
    pub oracle_contract: String,             // oracle address
    pub lottery_interval: u64,               // number of blocks (or time) between lotteries
    pub block_time: u64, // number of blocks (or time) lottery is blocked while is executed
    pub round_delta: u64, // number of rounds of security to get oracle rand
    pub ticket_price: Decimal256, // prize of a ticket in stable_denom
    pub max_holders: u8, // Max number of holders per ticket
    pub prize_distribution: [Decimal256; 6], // [0, 0, 0.05, 0.15, 0.3, 0.5]
    pub target_award: Decimal256, // target award used in deposit rewards computation
    pub reserve_factor: Decimal256, // % of the prize that goes to the reserve fund
    pub split_factor: Decimal256, // what % of interest goes to saving and which one lotto pool
    pub instant_withdrawal_fee: Decimal256, // % to be deducted as a fee for instant withdrawals
    pub unbonding_period: u64, // unbonding period after regular withdrawals from pool
    pub initial_emission_rate: Decimal256, // initial GLOW emission rate for depositor rewards
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Register Contracts contract address - restricted to owner
    RegisterContracts {
        /// Gov contract accrues protocol fees and distributes them to Glow stakers
        gov_contract: String,
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
    },
    /// Update lottery configuration - restricted to owner
    UpdateLotteryConfig {
        lottery_interval: Option<u64>,
        block_time: Option<u64>,
        ticket_price: Option<Decimal256>,
        prize_distribution: Option<[Decimal256; 6]>,
        round_delta: Option<u64>,
    },
    /// Deposit amount of stable into the pool
    Deposit { combinations: Vec<String> },
    /// Deposit amount of stable into the pool in the name of the recipient
    Gift {
        combinations: Vec<String>,
        recipient: String,
    },
    /// Sponsor the pool. If award is true, sponsor the award available directly
    Sponsor { award: Option<bool> },
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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Lotto contract configuration
    Config {},
    /// Current state. If block_height is provided, return current depositor rewards
    State { block_height: Option<u64> },
    /// Lotto pool current state. Shares and deposits.
    Pool {},
    /// Lottery information by lottery id
    LotteryInfo { lottery_id: Option<u64> },
    /// Ticket information by sequence. Returns a list of holders (addresses)
    TicketInfo { sequence: String },
    /// Prizes for a given address on a given lottery id
    PrizeInfo { address: String, lottery_id: u64 },
    /// Depositor information by address
    Depositor { address: String },
    /// Sponsor information by address
    Sponsor { address: String },
    /// List (paginated) of depositors information
    Depositors {
        start_after: Option<String>,
        limit: Option<u32>,
    },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: String,
    pub stable_denom: String,
    pub a_terra_contract: String,
    pub anchor_contract: String,
    pub gov_contract: String,
    pub distributor_contract: String,
    pub lottery_interval: Duration,
    pub block_time: Duration,
    pub ticket_price: Decimal256,
    pub max_holders: u8,
    pub prize_distribution: [Decimal256; 6],
    pub target_award: Decimal256,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub instant_withdrawal_fee: Decimal256,
    pub unbonding_period: Duration,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StateResponse {
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

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PoolResponse {
    pub total_deposits: Decimal256,
    pub lottery_deposits: Decimal256,
    pub total_sponsor_amount: Decimal256,
    pub deposit_shares: Decimal256,
    pub lottery_shares: Decimal256,
    pub sponsor_shares: Decimal256,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryInfoResponse {
    pub lottery_id: u64,
    pub rand_round: u64,
    pub sequence: String,
    pub awarded: bool,
    pub timestamp: u64,
    pub total_prizes: Decimal256,
    pub number_winners: [u32; 6],
    pub page: String,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorInfoResponse {
    pub depositor: String,
    pub deposit_amount: Decimal256,
    pub shares: Decimal256,
    pub reward_index: Decimal256,
    pub pending_rewards: Decimal256,
    pub tickets: Vec<String>,
    pub unbonding_info: Vec<Claim>,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct SponsorInfoResponse {
    pub sponsor: String,
    pub amount: Decimal256,
    pub shares: Decimal256,
    pub reward_index: Decimal256,
    pub pending_rewards: Decimal256,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorsInfoResponse {
    pub depositors: Vec<DepositorInfoResponse>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Claim {
    pub amount: Decimal256,
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
    pub matches: [u32; 6],
}

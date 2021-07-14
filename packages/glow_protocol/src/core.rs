use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{HumanAddr, Uint128};
use cw0::{Duration, Expiration};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub owner: HumanAddr,
    pub stable_denom: String,
    pub anchor_contract: HumanAddr,
    pub aterra_contract: HumanAddr,
    pub lottery_interval: u64,
    pub block_time: u64,
    pub ticket_prize: Decimal256,
    pub prize_distribution: Vec<Decimal256>,
    pub target_award: Decimal256,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub unbonding_period: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    /// Register Contracts contract address
    RegisterContracts {
        /// The contract has the logics for
        /// GLOW distribution to depositors
        /// distribution_model: HumanAddr,
        /// Collector contract to send the reserve fees
        collector_contract: HumanAddr,
        /// Faucet contract to drip GLOW token to users
        distributor_contract: HumanAddr,
    },
    /// Update contract configuration
    UpdateConfig {
        owner: Option<HumanAddr>,
        lottery_interval: Option<u64>,
        block_time: Option<u64>,
        ticket_prize: Option<Decimal256>,
        prize_distribution: Option<Vec<Decimal256>>,
        reserve_factor: Option<Decimal256>,
        split_factor: Option<Decimal256>,
        unbonding_period: Option<u64>,
    },
    SingleDeposit {
        combination: String,
    },
    Deposit {
        combinations: Vec<String>,
    },
    Gift {
        combinations: Vec<String>,
        recipient: HumanAddr,
    },
    Sponsor {
        award: Option<bool>,
    },
    Withdraw {
        instant: Option<bool>,
    },
    Claim {
        amount: Option<Uint128>,
    },
    ClaimRewards {},
    ExecuteLottery {},
    /// (internal) Called internally by ExecuteLottery
    _HandlePrize {},
    ExecuteEpochOps {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
    State {
        block_height: Option<u64>,
    },
    LotteryInfo {
        lottery_id: Option<u64>,
    },
    Depositor {
        address: HumanAddr,
    },
    Depositors {
        start_after: Option<HumanAddr>,
        limit: Option<u32>,
    },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: HumanAddr,
    pub stable_denom: String,
    pub anchor_contract: HumanAddr,
    pub gov_contract: HumanAddr,
    pub distributor_contract: HumanAddr,
    pub lottery_interval: Duration,
    pub block_time: Duration,
    pub ticket_prize: Decimal256,
    pub prize_distribution: Vec<Decimal256>,
    pub target_award: Decimal256,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub unbonding_period: Duration,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StateResponse {
    pub total_tickets: Uint256,
    pub total_reserve: Decimal256,
    pub lottery_deposits: Decimal256,
    pub shares_supply: Decimal256,
    pub deposit_shares: Decimal256,
    pub award_available: Decimal256,
    pub current_balance: Uint256,
    pub current_lottery: u64,
    pub next_lottery_time: Expiration,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryInfoResponse {
    pub lottery_id: u64,
    pub sequence: String,
    pub awarded: bool,
    pub total_prizes: Decimal256,
    pub winners: Vec<(u8, Vec<HumanAddr>)>, // [(number_hits, [lucky_holders])]
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DepositorInfoResponse {
    pub depositor: HumanAddr,
    pub deposit_amount: Decimal256,
    pub shares: Decimal256,
    pub redeemable_amount: Uint128,
    pub tickets: Vec<String>,
    pub unbonding_info: Vec<Claim>,
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

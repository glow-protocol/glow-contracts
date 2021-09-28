use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Uint128};
use cw0::{Duration, Expiration};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: String,
    pub stable_denom: String,
    pub anchor_contract: String,
    pub aterra_contract: String,
    pub lottery_interval: u64,
    pub block_time: u64,
    pub ticket_price: Decimal256,
    pub max_holders: u8,
    pub prize_distribution: [Decimal256; 6],
    pub target_award: Decimal256,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub instant_withdrawal_fee: Decimal256,
    pub unbonding_period: u64,
    pub initial_emission_rate: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Register Contracts contract address
    RegisterContracts {
        /// Gov contract accrues protocol fees and distributes them to Glow stakers
        gov_contract: String,
        /// Faucet contract to drip GLOW token to users and update Glow emission rate
        distributor_contract: String,
    },
    /// Update contract configuration
    UpdateConfig {
        owner: Option<String>,
        lottery_interval: Option<u64>,
        block_time: Option<u64>,
        ticket_price: Option<Decimal256>,
        prize_distribution: Option<[Decimal256; 6]>,
        reserve_factor: Option<Decimal256>,
        split_factor: Option<Decimal256>,
        unbonding_period: Option<u64>,
    },
    Deposit {
        combinations: Vec<String>,
    },
    Gift {
        combinations: Vec<String>,
        recipient: String,
    },
    Sponsor {
        award: Option<bool>,
    },
    Withdraw {
        amount: Option<Uint128>,
        instant: Option<bool>,
    },
    Claim {
        lottery: Option<u64>,
    },
    ClaimRewards {},
    ExecuteLottery {},
    ExecutePrize {
        limit: Option<u32>,
    },
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
    TicketInfo {
        sequence: String,
    },
    PrizeInfo {
        address: String,
        lottery_id: u64,
    },
    Depositor {
        address: String,
    },
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

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryInfoResponse {
    pub lottery_id: u64,
    pub sequence: String,
    pub awarded: bool,
    pub total_prizes: Decimal256,
    pub number_winners: [u32; 6], // numeber of winners per hits e.g. [0,0,3,2,0,0]
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

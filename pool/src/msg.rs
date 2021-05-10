use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::HumanAddr;

use cw0::{Duration, Expiration};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub owner: HumanAddr,
    pub stable_denom: String,
    pub anchor_contract: HumanAddr,
    pub aterra_contract: HumanAddr,
    pub lottery_interval: Duration,
    pub block_time: Duration,
    pub ticket_prize: Decimal256,
    pub prize_distribution: Vec<Decimal256>,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub unbonding_period: Duration,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    SingleDeposit {
        combination: String,
    },
    Withdraw {
        amount: u64,
    },
    ExecuteLottery {},
    _HandlePrize {},
    UpdateConfig {
        owner: Option<HumanAddr>,
        lottery_interval: Option<Duration>,
        block_time: Option<Duration>,
        ticket_prize: Option<Decimal256>,
        prize_distribution: Option<Vec<Decimal256>>,
        reserve_factor: Option<Decimal256>,
        split_factor: Option<Decimal256>,
        unbonding_period: Option<Duration>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    // GetCount returns the current count as a json-encoded number
    Config {},
    State { block_height: Option<u64> },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: HumanAddr,
    pub stable_denom: String,
    pub anchor_contract: HumanAddr,
    pub lottery_interval: Duration,
    pub block_time: Duration,
    pub ticket_prize: Decimal256,
    pub prize_distribution: Vec<Decimal256>,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
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
    pub award_available: Decimal256,
    pub spendable_balance: Decimal256,
    pub current_balance: Uint256,
    pub current_lottery: u64,
    pub next_lottery_time: Expiration,
}

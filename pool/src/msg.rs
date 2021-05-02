use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{HumanAddr, Uint128};

use cw0::Duration;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub owner: HumanAddr,
    pub stable_denom: String,
    pub anchor_contract: HumanAddr,
    pub lottery_interval: u64,
    pub block_time: u64,
    pub ticket_prize: u64,
    pub prize_distribution: Vec<Decimal256>,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub period_prize: u64, // not sure what am i doing with this one
    pub ticket_exchange_rate: Decimal256,
    pub unbonding_period: Duration,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    SingleDeposit {
        combination: String,
    },
    Withdraw {
        amount: Uint128,
    },
    ExecuteLottery {},
    _HandlePrize {},
    UpdateConfig {
        owner: Option<HumanAddr>,
        period_prize: Option<u64>,
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
    pub period_prize: u64,
    pub ticket_exchange_rate: Decimal256,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StateResponse {
    pub total_tickets: Uint256,
    pub total_reserves: Decimal256,
    pub last_interest: Decimal256,
    pub total_accrued_interest: Decimal256,
    pub award_available: Decimal256,
    pub total_assets: Decimal256,
}

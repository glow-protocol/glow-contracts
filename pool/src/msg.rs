use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::HumanAddr;

use cw20::Cw20ReceiveMsg;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub owner: HumanAddr,
    pub stable_denom: String,
    pub anchor_contract: HumanAddr,
    pub b_terra_code_id: u64,
    pub lottery_interval: u64,
    pub block_time: u64,
    pub ticket_prize: u64,
    pub prize_distribution: Vec<Decimal256>,
    pub reserve_factor: Decimal256,
    pub split_factor: Decimal256,
    pub period_prize: u64, // not sure what am i doing with this one
    pub ticket_exchange_rate: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    Receive(Cw20ReceiveMsg),
    DepositStable {},
    SingleDeposit {
        combination: String,
    },
    Withdraw {
        amount: Option<u64>,
    },
    ExecuteLottery {},
    _HandlePrize {},
    RegisterSTerra {},
    UpdateConfig {
        owner: Option<HumanAddr>,
        period_prize: Option<u64>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cw20HookMsg {
    // Return stablecoins to user and burn b_terra
    RedeemStable {},
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

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::Uint128;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: String,          // glow gov contract
    pub glow_token: String,     // glow token address
    pub whitelist: Vec<String>, // whitelisted contract addresses to spend distributor
    pub spend_limit: Uint128,   // spend limit per each `spend` request
    pub emission_cap: Decimal256,
    pub emission_floor: Decimal256,
    pub increment_multiplier: Decimal256,
    pub decrement_multiplier: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    UpdateConfig {
        owner: Option<String>,
        spend_limit: Option<Uint128>,
        emission_cap: Option<Decimal256>,
        emission_floor: Option<Decimal256>,
        increment_multiplier: Option<Decimal256>,
        decrement_multiplier: Option<Decimal256>,
    },
    Spend {
        recipient: String,
        amount: Uint128,
    },
    AddDistributor {
        distributor: String,
    },
    RemoveDistributor {
        distributor: String,
    },
}

/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
    GlowEmissionRate {
        current_award: Uint256,
        target_award: Uint256,
        current_emission_rate: Decimal256,
    },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: String,
    pub glow_token: String,
    pub whitelist: Vec<String>,
    pub spend_limit: Uint128,
    pub emission_cap: Decimal256,
    pub emission_floor: Decimal256,
    pub increment_multiplier: Decimal256,
    pub decrement_multiplier: Decimal256,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GlowEmissionRateResponse {
    pub emission_rate: Decimal256,
}

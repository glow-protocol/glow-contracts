use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::Decimal256;
use cosmwasm_std::{HumanAddr, Uint128};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub gov_contract: HumanAddr,   // glow gov contract
    pub glow_token: HumanAddr,     // glow token address
    pub whitelist: Vec<HumanAddr>, // whitelisted contract addresses to spend distributor
    pub spend_limit: Uint128,      // spend limit per each `spend` request
    pub emission_cap: Decimal256,
    pub emission_floor: Decimal256,
    pub increment_multiplier: Decimal256,
    pub decrement_multiplier: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    UpdateConfig {
        spend_limit: Option<Uint128>,
        emission_cap: Option<Decimal256>,
        emission_floor: Option<Decimal256>,
        increment_multiplier: Option<Decimal256>,
        decrement_multiplier: Option<Decimal256>,
    },
    Spend {
        recipient: HumanAddr,
        amount: Uint128,
    },
    AddDistributor {
        distributor: HumanAddr,
    },
    RemoveDistributor {
        distributor: HumanAddr,
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
        current_award: Decimal256,
        target_award: Decimal256,
        current_emission_rate: Decimal256,
    },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub gov_contract: HumanAddr,
    pub glow_token: HumanAddr,
    pub whitelist: Vec<HumanAddr>,
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

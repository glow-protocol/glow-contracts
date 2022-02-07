use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::Decimal256;
use cosmwasm_std::Uint128;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: String,        // owner contract, to be transferred to glow gov contract
    pub stable_denom: String, // stable denomination
    pub glow_token: String,   // glow token address
    pub lotto_contract: String, // lotto contract address
    pub gov_contract: String, // gov contract address
    pub terraswap_factory: String, // terraswap factory contract address
    pub spend_limit: Uint128, // spend limit per each `spend` request
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ExecuteMsg {
    UpdateConfig {
        spend_limit: Option<Uint128>,
        owner: Option<String>,
    },
    Spend {
        recipient: String,
        amount: Uint128,
    },
    TransferStable {
        amount: Uint128,
        recipient: String,
    },
    SponsorLotto {
        amount: Uint128,
        award: Option<bool>,
        prize_distribution: Option<[Decimal256; 7]>,
    },
    WithdrawSponsor {},
    Swap {
        amount: Uint128,
    },
    Burn {
        amount: Uint128,
    },
}

/// Migrations message
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {
    pub lotto_contract: String,
    pub gov_contract: String,
    pub terraswap_factory: String,
    pub stable_denom: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: String,
    pub stable_denom: String,
    pub glow_token: String,
    pub lotto_contract: String,
    pub gov_contract: String,
    pub terraswap_factory: String,
    pub spend_limit: Uint128,
}

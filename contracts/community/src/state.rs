use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{CanonicalAddr, StdResult, Storage, Uint128};
use cosmwasm_storage::{singleton, singleton_read};

static KEY_CONFIG: &[u8] = b"config";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: CanonicalAddr, // Owner address, to be transferred to Gov Contract
    pub stable_denom: String, // Stable coin denomination used e.g. "uusd"
    pub glow_token: CanonicalAddr, // glow token address
    pub lotto_contract: CanonicalAddr, // glow lotto address
    pub gov_contract: CanonicalAddr, // glow governance address
    pub spend_limit: Uint128, // spend limit per each `spend` request
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct OldConfig {
    pub owner: CanonicalAddr, // Owner address, to be transferred to Gov Contract
    pub glow_token: CanonicalAddr, // glow token address
    pub spend_limit: Uint128, // spend limit per each `spend` request
}

pub fn store_config(storage: &mut dyn Storage, config: &Config) -> StdResult<()> {
    singleton(storage, KEY_CONFIG).save(config)
}

pub fn read_config(storage: &dyn Storage) -> StdResult<Config> {
    singleton_read(storage, KEY_CONFIG).load()
}

pub fn read_old_config(storage: &dyn Storage) -> StdResult<OldConfig> {
    singleton_read(storage, KEY_CONFIG).load()
}

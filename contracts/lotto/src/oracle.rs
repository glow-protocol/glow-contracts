use cosmwasm_std::{Addr, Binary, Env};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const RAND_GENESIS: u64 = 1595431050;
const RAND_PERIOD: u64 = 30;

pub fn calculate_lottery_rand_round(env: Env, round_delta: u64) -> u64 {
    let from_genesis = env.block.time.seconds().checked_sub(RAND_GENESIS).unwrap();
    let current_round = from_genesis.checked_div(RAND_PERIOD).unwrap();
    current_round + round_delta //make round delta as config param
}

pub fn sequence_from_hash(hash: String) -> String {
    let seq = &hash[2..7];
    seq.to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    GetRandomness { round: u64 },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct OracleResponse {
    pub randomness: Binary,
    pub worker: Addr,
}

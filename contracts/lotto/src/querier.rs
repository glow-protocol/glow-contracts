use crate::oracle::{OracleResponse, QueryMsg as QueryOracle};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::Uint128;
use cosmwasm_std::{
    to_binary, Addr, BalanceResponse as BankBalanceResponse, BankQuery, Deps, QuerierWrapper,
    QueryRequest, StdResult, WasmQuery,
};
use glow_protocol::distributor::{GlowEmissionRateResponse, QueryMsg as DistributorQueryMsg};
use moneymarket::market::{EpochStateResponse, QueryMsg as AnchorMsg};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Balance { address: String },
    BalanceAt { address: String, height: u64 },
    TotalSupply {},
    TotalSupplyAt { height: u64 },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct BalanceResponse {
    pub balance: Uint128,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct TotalSupplyResponse {
    pub total_supply: Uint128,
}

pub fn query_exchange_rate(
    deps: Deps,
    money_market_addr: String,
    height: u64,
) -> StdResult<EpochStateResponse> {
    let epoch_state: EpochStateResponse =
        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: money_market_addr,
            msg: to_binary(&AnchorMsg::EpochState {
                block_height: Some(height),
                distributed_interest: None,
            })?,
        }))?;

    Ok(epoch_state)
}

pub fn query_balance(deps: Deps, account_addr: String, denom: String) -> StdResult<Uint256> {
    // load price form the oracle
    let balance: BankBalanceResponse =
        deps.querier.query(&QueryRequest::Bank(BankQuery::Balance {
            address: account_addr,
            denom,
        }))?;
    Ok(balance.amount.amount.into())
}

pub fn query_glow_emission_rate(
    querier: &QuerierWrapper,
    distributor: Addr,
    lottery_balance: Uint256,
    target_award: Uint256,
    current_emission_rate: Decimal256,
) -> StdResult<GlowEmissionRateResponse> {
    // get the amount of money in the lottery pool

    let glow_emission_rate: GlowEmissionRateResponse =
        querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: distributor.to_string(),
            msg: to_binary(&DistributorQueryMsg::GlowEmissionRate {
                current_award: lottery_balance,
                target_award,
                current_emission_rate,
            })?,
        }))?;

    Ok(glow_emission_rate)
}

pub fn query_address_voting_balance_at_height(
    querier: &QuerierWrapper,
    gov: &Addr,
    block_height: u64,
    address: &Addr,
) -> StdResult<BalanceResponse> {
    let balance: BalanceResponse = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: gov.to_string(),
        msg: to_binary(&QueryMsg::BalanceAt {
            address: address.to_string(),
            height: block_height,
        })?,
    }))?;

    Ok(balance)
}

pub fn query_total_voting_balance_at_height(
    querier: &QuerierWrapper,
    gov: &Addr,
    block_height: u64,
) -> StdResult<TotalSupplyResponse> {
    let total_supply: TotalSupplyResponse =
        querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: gov.to_string(),
            msg: to_binary(&QueryMsg::TotalSupplyAt {
                height: block_height,
            })?,
        }))?;

    Ok(total_supply)
}

pub fn query_oracle(deps: Deps, oracle_addr: String, round: u64) -> StdResult<OracleResponse> {
    let oracle_response: OracleResponse =
        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: oracle_addr,
            msg: to_binary(&QueryOracle::GetRandomness { round })?,
        }))?;

    Ok(oracle_response)
}

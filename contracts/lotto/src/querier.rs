use crate::oracle::{OracleResponse, QueryMsg as QueryOracle};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    to_binary, Addr, BalanceResponse, BankQuery, Deps, QuerierWrapper, QueryRequest, StdResult,
    WasmQuery,
};
use glow_protocol::distributor::{GlowEmissionRateResponse, QueryMsg as DistributorQueryMsg};
use moneymarket::market::{EpochStateResponse, QueryMsg as AnchorMsg};

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
    let balance: BalanceResponse = deps.querier.query(&QueryRequest::Bank(BankQuery::Balance {
        address: account_addr,
        denom,
    }))?;
    Ok(balance.amount.amount.into())
}

pub fn query_glow_emission_rate(
    querier: &QuerierWrapper,
    distributor: Addr,
    current_award: Decimal256,
    target_award: Decimal256,
    current_emission_rate: Decimal256,
) -> StdResult<GlowEmissionRateResponse> {
    let glow_emission_rate: GlowEmissionRateResponse =
        querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: distributor.to_string(),
            msg: to_binary(&DistributorQueryMsg::GlowEmissionRate {
                current_award,
                target_award,
                current_emission_rate,
            })?,
        }))?;

    Ok(glow_emission_rate)
}

pub fn query_oracle(deps: Deps, oracle_addr: String, round: u64) -> StdResult<OracleResponse> {
    let oracle_response: OracleResponse =
        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: oracle_addr,
            msg: to_binary(&QueryOracle::GetRandomness { round })?,
        }))?;

    Ok(oracle_response)
}

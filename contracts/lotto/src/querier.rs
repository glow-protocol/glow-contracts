use crate::oracle::{OracleResponse, QueryMsg as QueryOracle};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::Uint128;
use cosmwasm_std::{
    to_binary, Addr, BalanceResponse as BankBalanceResponse, BankQuery, Deps, QuerierWrapper,
    QueryRequest, StdResult, WasmQuery,
};
use glow_protocol::distributor::{GlowEmissionRateResponse, QueryMsg as DistributorQueryMsg};
use glow_protocol::ve_token::{QueryMsg as VEQueryMessage, StakerResponse, StateResponse};
use moneymarket::market::{EpochStateResponse, QueryMsg as AnchorMsg};

pub fn query_exchange_rate(
    deps: Deps,
    money_market_addr: String,
    block_height: u64,
) -> StdResult<EpochStateResponse> {
    let epoch_state: EpochStateResponse =
        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: money_market_addr,
            msg: to_binary(&AnchorMsg::EpochState {
                block_height: Some(block_height),
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

#[allow(dead_code)]
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

pub fn query_address_voting_balance_at_timestamp(
    querier: &QuerierWrapper,
    ve_addr: &Addr,
    timestamp: u64,
    address: &Addr,
) -> StdResult<Uint128> {
    let balance: StdResult<StakerResponse> = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: ve_addr.to_string(),
        msg: to_binary(&VEQueryMessage::Staker {
            address: address.to_string(),
            timestamp: Some(timestamp),
        })?,
    }));

    Ok(balance.map_or(Uint128::zero(), |s| s.balance))
}

pub fn query_total_voting_balance_at_timestamp(
    querier: &QuerierWrapper,
    ve_addr: &Addr,
    timestamp: u64,
) -> StdResult<Uint128> {
    let total_supply: StdResult<StateResponse> =
        querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: ve_addr.to_string(),
            msg: to_binary(&VEQueryMessage::State {
                timestamp: Some(timestamp),
            })?,
        }));

    Ok(total_supply.map_or(Uint128::zero(), |t| t.total_balance))
}

pub fn query_oracle(deps: Deps, oracle_addr: String, round: u64) -> StdResult<OracleResponse> {
    let oracle_response: OracleResponse =
        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: oracle_addr,
            msg: to_binary(&QueryOracle::GetRandomness { round })?,
        }))?;

    Ok(oracle_response)
}

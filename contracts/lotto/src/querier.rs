use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    to_binary, Addr, BalanceResponse as BankBalanceResponse, BankQuery, Deps, QuerierWrapper,
    QueryRequest, StdResult, WasmQuery,
};
use glow_protocol::prize_distributor::QueryMsg as PrizeDistributorQueryMsg;
use glow_protocol::{
    distributor::{GlowEmissionRateResponse, QueryMsg as DistributorQueryMsg},
    prize_distributor::PrizeDistributionPendingResponse,
};
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

pub fn query_prize_distribution_pending(
    deps: Deps,
    prize_distributor_address: Addr,
) -> StdResult<PrizeDistributionPendingResponse> {
    let prize_distribution_pending_response: PrizeDistributionPendingResponse =
        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: prize_distributor_address.to_string(),
            msg: to_binary(&PrizeDistributorQueryMsg::PrizeDistributionPending {})?,
        }))?;

    Ok(prize_distribution_pending_response)
}

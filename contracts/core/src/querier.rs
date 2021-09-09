use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    from_binary, to_binary, Addr, AllBalanceResponse, BalanceResponse, BankQuery, Binary, Coin,
    Deps, QuerierWrapper, QueryRequest, StdResult, WasmQuery,
};
use cosmwasm_storage::to_length_prefixed;
use cw20::TokenInfoResponse;
use glow_protocol::core::Claim;
use glow_protocol::distributor::{GlowEmissionRateResponse, QueryMsg as DistributorQueryMsg};
use moneymarket::market::{EpochStateResponse, QueryMsg as AnchorMsg};
use terra_cosmwasm::TerraQuerier;

use crate::state::read_depositor_info;

pub fn query_exchange_rate(deps: Deps, money_market_addr: String) -> StdResult<EpochStateResponse> {
    let epoch_state: EpochStateResponse =
        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: money_market_addr,
            msg: to_binary(&AnchorMsg::EpochState {
                block_height: None,
                distributed_interest: None,
            })?,
        }))?;

    Ok(epoch_state)
}

#[allow(dead_code)]
pub fn query_all_balances(deps: Deps, account_addr: String) -> StdResult<Vec<Coin>> {
    // load price form the oracle
    let all_balances: AllBalanceResponse =
        deps.querier
            .query(&QueryRequest::Bank(BankQuery::AllBalances {
                address: account_addr,
            }))?;
    Ok(all_balances.amount)
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

#[allow(dead_code)]
pub fn query_depositor_claims(deps: Deps, addr: String) -> StdResult<Vec<Claim>> {
    let address_raw = deps.api.addr_canonicalize(&addr)?;
    let claims = read_depositor_info(deps.storage, &address_raw).unbonding_info;
    Ok(claims)
}

#[allow(dead_code)]
pub fn query_supply(deps: Deps, contract_addr: String) -> StdResult<Uint256> {
    // load price form the oracle
    let res: Binary = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Raw {
        contract_addr,
        key: Binary::from(to_length_prefixed(b"token_info")),
    }))?;

    let token_info: TokenInfoResponse = from_binary(&res)?;
    Ok(Uint256::from(token_info.total_supply))
}

#[allow(dead_code)]
pub fn query_tax_rate(deps: Deps) -> StdResult<Decimal256> {
    let terra_querier = TerraQuerier::new(&deps.querier);
    Ok(terra_querier.query_tax_rate()?.rate.into())
}

#[allow(dead_code)]
pub fn compute_tax(deps: Deps, coin: &Coin) -> StdResult<Uint256> {
    let terra_querier = TerraQuerier::new(&deps.querier);
    let tax_rate = Decimal256::from((terra_querier.query_tax_rate()?).rate);
    let tax_cap = Uint256::from((terra_querier.query_tax_cap(coin.denom.to_string())?).cap);
    let amount = Uint256::from(coin.amount);
    Ok(std::cmp::min(
        amount * (Decimal256::one() - Decimal256::one() / (Decimal256::one() + tax_rate)),
        tax_cap,
    ))
}

#[allow(dead_code)]
pub fn deduct_tax(deps: Deps, coin: Coin) -> StdResult<Coin> {
    let tax_amount = compute_tax(deps, &coin)?;
    Ok(Coin {
        denom: coin.denom,
        amount: (Uint256::from(coin.amount) - tax_amount).into(),
    })
}

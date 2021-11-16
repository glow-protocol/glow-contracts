use cosmwasm_std::{to_binary, Addr, QuerierWrapper, QueryRequest, StdResult, WasmQuery};
use cw20::{Cw20QueryMsg, MinterResponse};

/// Query minter of GLOW CW20 contract, set to Gov at launch
pub fn query_glow_minter(querier: &QuerierWrapper, glow_token: Addr) -> StdResult<String> {
    let res: MinterResponse = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: glow_token.to_string(),
        msg: to_binary(&Cw20QueryMsg::Minter {})?,
    }))?;

    Ok(res.minter)
}

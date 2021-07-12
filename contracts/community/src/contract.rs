use crate::state::{read_config, store_config, Config};

use cosmwasm_std::{
    log, to_binary, Api, Binary, CosmosMsg, Env, Extern, HandleResponse, HandleResult, HumanAddr,
    InitResponse, MigrateResponse, MigrateResult, Querier, StdError, StdResult, Storage, Uint128,
    WasmMsg,
};

use glow_protocol::community::{ConfigResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg};

use cw20::Cw20HandleMsg;

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    store_config(
        &mut deps.storage,
        &Config {
            gov_contract: deps.api.canonical_address(&msg.gov_contract)?,
            glow_token: deps.api.canonical_address(&msg.glow_token)?,
            spend_limit: msg.spend_limit,
        },
    )?;

    Ok(InitResponse::default())
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::UpdateConfig { spend_limit } => update_config(deps, env, spend_limit),
        HandleMsg::Spend { recipient, amount } => spend(deps, env, recipient, amount),
    }
}

pub fn update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    spend_limit: Option<Uint128>,
) -> HandleResult {
    let mut config: Config = read_config(&deps.storage)?;
    if config.gov_contract != deps.api.canonical_address(&env.message.sender)? {
        return Err(StdError::unauthorized());
    }

    if let Some(spend_limit) = spend_limit {
        config.spend_limit = spend_limit;
    }

    store_config(&mut deps.storage, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![log("action", "update_config")],
        data: None,
    })
}

/// Spend
/// Owner (governance contract) can execute spend operation to send
/// `amount` of GLOW tokens to `recipient` for community purpose
pub fn spend<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    recipient: HumanAddr,
    amount: Uint128,
) -> HandleResult {
    let config: Config = read_config(&deps.storage)?;
    if config.gov_contract != deps.api.canonical_address(&env.message.sender)? {
        return Err(StdError::unauthorized());
    }

    if config.spend_limit < amount {
        return Err(StdError::generic_err("Cannot spend more than spend_limit"));
    }

    let glow_token = deps.api.human_address(&config.glow_token)?;
    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: glow_token,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Transfer {
                recipient: recipient.clone(),
                amount,
            })?,
        })],
        log: vec![
            log("action", "spend"),
            log("recipient", recipient),
            log("amount", amount),
        ],
        data: None,
    })
}

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
    }
}

pub fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let state = read_config(&deps.storage)?;
    let resp = ConfigResponse {
        gov_contract: deps.api.human_address(&state.gov_contract)?,
        glow_token: deps.api.human_address(&state.glow_token)?,
        spend_limit: state.spend_limit,
    };

    Ok(resp)
}

pub fn migrate<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: MigrateMsg,
) -> MigrateResult {
    Ok(MigrateResponse::default())
}

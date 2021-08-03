use crate::state::{read_config, store_config, Config};

use cosmwasm_bignumber::Decimal256;
use cosmwasm_std::{
    log, to_binary, Api, Binary, CanonicalAddr, CosmosMsg, Env, Extern, HandleResponse,
    HandleResult, HumanAddr, InitResponse, MigrateResponse, MigrateResult, Querier, StdError,
    StdResult, Storage, Uint128, WasmMsg,
};

use glow_protocol::distributor::{
    ConfigResponse, GlowEmissionRateResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg,
};

use cw20::Cw20HandleMsg;

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    let whitelist = msg
        .whitelist
        .into_iter()
        .map(|w| deps.api.canonical_address(&w))
        .collect::<StdResult<Vec<CanonicalAddr>>>()?;

    store_config(
        &mut deps.storage,
        &Config {
            owner: deps.api.canonical_address(&msg.owner)?,
            glow_token: deps.api.canonical_address(&msg.glow_token)?,
            whitelist,
            spend_limit: msg.spend_limit,
            emission_cap: msg.emission_cap,
            emission_floor: msg.emission_floor,
            increment_multiplier: msg.increment_multiplier,
            decrement_multiplier: msg.decrement_multiplier,
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
        HandleMsg::UpdateConfig {
            owner,
            spend_limit,
            emission_cap,
            emission_floor,
            increment_multiplier,
            decrement_multiplier,
        } => update_config(
            deps,
            env,
            owner,
            spend_limit,
            emission_cap,
            emission_floor,
            increment_multiplier,
            decrement_multiplier,
        ),
        HandleMsg::Spend { recipient, amount } => spend(deps, env, recipient, amount),
        HandleMsg::AddDistributor { distributor } => add_distributor(deps, env, distributor),
        HandleMsg::RemoveDistributor { distributor } => remove_distributor(deps, env, distributor),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: Option<HumanAddr>,
    spend_limit: Option<Uint128>,
    emission_cap: Option<Decimal256>,
    emission_floor: Option<Decimal256>,
    increment_multiplier: Option<Decimal256>,
    decrement_multiplier: Option<Decimal256>,
) -> HandleResult {
    let mut config: Config = read_config(&deps.storage)?;
    if config.owner != deps.api.canonical_address(&env.message.sender)? {
        return Err(StdError::unauthorized());
    }

    if let Some(owner) = owner {
        config.owner = deps.api.canonical_address(&owner)?;
    }

    if let Some(spend_limit) = spend_limit {
        config.spend_limit = spend_limit;
    }

    if let Some(emission_cap) = emission_cap {
        config.emission_cap = emission_cap;
    }

    if let Some(emission_floor) = emission_floor {
        config.emission_floor = emission_floor;
    }

    if let Some(increment_multiplier) = increment_multiplier {
        config.increment_multiplier = increment_multiplier;
    }

    if let Some(decrement_multiplier) = decrement_multiplier {
        config.decrement_multiplier = decrement_multiplier;
    }

    store_config(&mut deps.storage, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![log("action", "update_config")],
        data: None,
    })
}

pub fn add_distributor<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    distributor: HumanAddr,
) -> HandleResult {
    let mut config: Config = read_config(&deps.storage)?;
    if config.owner != deps.api.canonical_address(&env.message.sender)? {
        return Err(StdError::unauthorized());
    }

    let distributor_raw = deps.api.canonical_address(&distributor)?;
    if config
        .whitelist
        .clone()
        .into_iter()
        .any(|w| w == distributor_raw)
    {
        return Err(StdError::generic_err("Distributor already registered"));
    }

    config.whitelist.push(distributor_raw);
    store_config(&mut deps.storage, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "add_distributor"),
            log("distributor", distributor),
        ],
        data: None,
    })
}

pub fn remove_distributor<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    distributor: HumanAddr,
) -> HandleResult {
    let mut config: Config = read_config(&deps.storage)?;
    if config.owner != deps.api.canonical_address(&env.message.sender)? {
        return Err(StdError::unauthorized());
    }

    let distributor = deps.api.canonical_address(&distributor)?;
    let whitelist: Vec<CanonicalAddr> = config
        .whitelist
        .clone()
        .into_iter()
        .filter(|w| *w != distributor)
        .collect();

    if config.whitelist.len() == whitelist.len() {
        return Err(StdError::generic_err("Distributor not found"));
    }

    config.whitelist = whitelist;
    store_config(&mut deps.storage, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "remove_distributor"),
            log("distributor", distributor),
        ],
        data: None,
    })
}

/// Spend
/// Owner can execute spend operation to send
/// `amount` of GLOW token to `recipient` for community purposes
pub fn spend<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    recipient: HumanAddr,
    amount: Uint128,
) -> HandleResult {
    let config: Config = read_config(&deps.storage)?;
    let sender_raw = deps.api.canonical_address(&env.message.sender)?;

    if config
        .whitelist
        .into_iter()
        .find(|w| *w == sender_raw)
        .is_none()
    {
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
        QueryMsg::GlowEmissionRate {
            current_award,
            target_award,
            current_emission_rate,
        } => to_binary(&query_glow_emission_rate(
            deps,
            current_award,
            target_award,
            current_emission_rate,
        )?),
    }
}

pub fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = read_config(&deps.storage)?;
    let resp = ConfigResponse {
        owner: deps.api.human_address(&config.owner)?,
        glow_token: deps.api.human_address(&config.glow_token)?,
        whitelist: config
            .whitelist
            .into_iter()
            .map(|w| deps.api.human_address(&w))
            .collect::<StdResult<Vec<HumanAddr>>>()?,
        spend_limit: config.spend_limit,
        emission_cap: config.emission_cap,
        emission_floor: config.emission_floor,
        increment_multiplier: config.increment_multiplier,
        decrement_multiplier: config.decrement_multiplier,
    };

    Ok(resp)
}

#[allow(clippy::comparison_chain)]
fn query_glow_emission_rate<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    current_award: Decimal256,
    target_award: Decimal256,
    current_emission_rate: Decimal256,
) -> StdResult<GlowEmissionRateResponse> {
    let config: Config = read_config(&deps.storage)?;

    let emission_rate = if current_award < target_award {
        current_emission_rate * config.increment_multiplier
    } else if current_award > target_award {
        current_emission_rate * config.decrement_multiplier
    } else {
        current_emission_rate
    };

    let emission_rate = if emission_rate > config.emission_cap {
        config.emission_cap
    } else if emission_rate < config.emission_floor {
        config.emission_floor
    } else {
        emission_rate
    };

    Ok(GlowEmissionRateResponse { emission_rate })
}

// TODO: should we restrict/avoid migrations in this contract?
pub fn migrate<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: MigrateMsg,
) -> MigrateResult {
    Ok(MigrateResponse::default())
}

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use crate::state::{read_config, store_config, Config};

use cosmwasm_bignumber::Decimal256;
use cosmwasm_std::{
    attr, to_binary, Binary, CanonicalAddr, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response,
    StdError, StdResult, Uint128, WasmMsg,
};

use glow_protocol::distributor::{
    ConfigResponse, ExecuteMsg, GlowEmissionRateResponse, InstantiateMsg, MigrateMsg, QueryMsg,
};

use cw20::Cw20ExecuteMsg;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    let whitelist = msg
        .whitelist
        .into_iter()
        .map(|w| deps.api.addr_canonicalize(&w))
        .collect::<StdResult<Vec<CanonicalAddr>>>()?;

    if msg.increment_multiplier < Decimal256::one() {
        return Err(StdError::generic_err(
            "Increment multiplier must be equal or greater than 1",
        ));
    }

    if msg.decrement_multiplier > Decimal256::one() {
        return Err(StdError::generic_err(
            "Decrement multiplier must be equal or smaller than 1",
        ));
    }

    if msg.emission_cap < msg.emission_floor {
        return Err(StdError::generic_err(
            "Emission cap must be greater or equal than emission floor",
        ));
    }

    store_config(
        deps.storage,
        &Config {
            owner: deps.api.addr_canonicalize(&msg.owner)?,
            glow_token: deps.api.addr_canonicalize(&msg.glow_token)?,
            whitelist,
            spend_limit: msg.spend_limit,
            emission_cap: msg.emission_cap,
            emission_floor: msg.emission_floor,
            increment_multiplier: msg.increment_multiplier,
            decrement_multiplier: msg.decrement_multiplier,
        },
    )?;

    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> StdResult<Response> {
    match msg {
        ExecuteMsg::UpdateConfig {
            owner,
            spend_limit,
            emission_cap,
            emission_floor,
            increment_multiplier,
            decrement_multiplier,
        } => update_config(
            deps,
            info,
            owner,
            spend_limit,
            emission_cap,
            emission_floor,
            increment_multiplier,
            decrement_multiplier,
        ),
        ExecuteMsg::Spend { recipient, amount } => spend(deps, info, recipient, amount),
        ExecuteMsg::AddDistributor { distributor } => add_distributor(deps, info, distributor),
        ExecuteMsg::RemoveDistributor { distributor } => {
            remove_distributor(deps, info, distributor)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    spend_limit: Option<Uint128>,
    emission_cap: Option<Decimal256>,
    emission_floor: Option<Decimal256>,
    increment_multiplier: Option<Decimal256>,
    decrement_multiplier: Option<Decimal256>,
) -> StdResult<Response> {
    let mut config: Config = read_config(deps.as_ref().storage)?;
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
    }

    if let Some(owner) = owner {
        config.owner = deps.api.addr_canonicalize(&owner)?;
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
        if increment_multiplier < Decimal256::one() {
            return Err(StdError::generic_err(
                "Increment multiplier must be equal or greater than 1",
            ));
        }
        config.increment_multiplier = increment_multiplier;
    }

    if let Some(decrement_multiplier) = decrement_multiplier {
        if decrement_multiplier > Decimal256::one() {
            return Err(StdError::generic_err(
                "Decrement multiplier must be equal or smaller than 1",
            ));
        }
        config.decrement_multiplier = decrement_multiplier;
    }

    if config.emission_cap < config.emission_floor {
        return Err(StdError::generic_err(
            "Emission cap must be greater or equal than emission floor",
        ));
    }

    store_config(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![attr("action", "update_config")]))
}

pub fn add_distributor(
    deps: DepsMut,
    info: MessageInfo,
    distributor: String,
) -> StdResult<Response> {
    let mut config: Config = read_config(deps.storage)?;
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
    }

    let distributor_raw = deps.api.addr_canonicalize(&distributor)?;
    if config
        .whitelist
        .clone()
        .into_iter()
        .any(|w| w == distributor_raw)
    {
        return Err(StdError::generic_err("Distributor already registered"));
    }

    config.whitelist.push(distributor_raw);
    store_config(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "add_distributor"),
        attr("distributor", distributor),
    ]))
}

pub fn remove_distributor(
    deps: DepsMut,
    info: MessageInfo,
    distributor: String,
) -> StdResult<Response> {
    let mut config: Config = read_config(deps.storage)?;
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
    }

    let distributor = deps.api.addr_canonicalize(&distributor)?;
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
    store_config(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "remove_distributor"),
        attr("distributor", distributor.to_string()),
    ]))
}

/// Spend
/// Owner can execute spend operation to send
/// `amount` of GLOW token to `recipient` for community purposes
pub fn spend(
    deps: DepsMut,
    info: MessageInfo,
    recipient: String,
    amount: Uint128,
) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;
    let sender_raw = deps.api.addr_canonicalize(info.sender.as_str())?;

    if !config.whitelist.into_iter().any(|w| w == sender_raw) {
        return Err(StdError::generic_err("unauthorized"));
    }

    if config.spend_limit < amount {
        return Err(StdError::generic_err("Cannot spend more than spend_limit"));
    }

    let glow_token = deps.api.addr_humanize(&config.glow_token)?.to_string();
    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: glow_token,
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: recipient.clone(),
                amount,
            })?,
        })])
        .add_attributes(vec![
            ("action", "spend"),
            ("recipient", recipient.as_str()),
            ("amount", amount.to_string().as_str()),
        ]))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
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

pub fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = read_config(deps.storage)?;
    let resp = ConfigResponse {
        owner: deps.api.addr_humanize(&config.owner)?.to_string(),
        glow_token: deps.api.addr_humanize(&config.glow_token)?.to_string(),
        whitelist: config
            .whitelist
            .into_iter()
            .map(|w| match deps.api.addr_humanize(&w) {
                Ok(addr) => Ok(addr.to_string()),
                Err(e) => Err(e),
            })
            .collect::<StdResult<Vec<String>>>()?,
        spend_limit: config.spend_limit,
        emission_cap: config.emission_cap,
        emission_floor: config.emission_floor,
        increment_multiplier: config.increment_multiplier,
        decrement_multiplier: config.decrement_multiplier,
    };

    Ok(resp)
}

#[allow(clippy::comparison_chain)]
fn query_glow_emission_rate(
    deps: Deps,
    current_award: Decimal256,
    target_award: Decimal256,
    current_emission_rate: Decimal256,
) -> StdResult<GlowEmissionRateResponse> {
    let config: Config = read_config(deps.storage)?;

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

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}

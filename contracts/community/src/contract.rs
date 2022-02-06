#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use crate::state::{read_config, read_old_config, store_config, Config};

use cosmwasm_std::{
    attr, to_binary, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response,
    StdError, StdResult, Uint128, WasmMsg,
};

use glow_protocol::community::{ConfigResponse, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};

use cosmwasm_bignumber::Decimal256;
use cw20::Cw20ExecuteMsg;
use glow_protocol::lotto::ExecuteMsg as LottoMsg;
use terraswap::asset::{Asset, AssetInfo, PairInfo};
use terraswap::pair::ExecuteMsg as TerraswapExecuteMsg;
use terraswap::querier::{query_balance, query_pair_info};

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    store_config(
        deps.storage,
        &Config {
            owner: deps.api.addr_canonicalize(&msg.owner)?,
            stable_denom: msg.stable_denom,
            glow_token: deps.api.addr_canonicalize(&msg.glow_token)?,
            lotto_contract: deps.api.addr_canonicalize(&msg.lotto_contract)?,
            gov_contract: deps.api.addr_canonicalize(&msg.gov_contract)?,
            terraswap_factory: deps.api.addr_canonicalize(&msg.terraswap_factory)?,
            spend_limit: msg.spend_limit,
        },
    )?;

    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::UpdateConfig { spend_limit, owner } => {
            update_config(deps, info, spend_limit, owner)
        }
        ExecuteMsg::Spend { recipient, amount } => spend(deps, info, recipient, amount),
        ExecuteMsg::TransferStable { amount, recipient } => {
            transfer_stable(deps, info, recipient, amount)
        }
        ExecuteMsg::SponsorLotto {
            amount,
            award,
            prize_distribution,
        } => sponsor_lotto(deps, info, amount, award, prize_distribution),
        ExecuteMsg::WithdrawSponsor {} => withdraw_sponsor(deps, info),
        ExecuteMsg::Swap { amount } => execute_swap(deps, info, env, amount),
        ExecuteMsg::Burn { amount } => execute_burn(deps, info, amount),
    }
}

/// Update Config
/// Owner (governance contract) can update the Config
pub fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    spend_limit: Option<Uint128>,
    owner: Option<String>,
) -> StdResult<Response> {
    let mut config: Config = read_config(deps.storage)?;
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
    }

    if let Some(spend_limit) = spend_limit {
        config.spend_limit = spend_limit;
    }

    if let Some(owner) = owner {
        config.owner = deps.api.addr_canonicalize(&owner)?;
    }

    store_config(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![attr("action", "update_config")]))
}

/// Spend
/// Owner (governance contract) can execute spend operation to send
/// `amount` of GLOW tokens to `recipient` for community purpose
pub fn spend(
    deps: DepsMut,
    info: MessageInfo,
    recipient: String,
    amount: Uint128,
) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
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
            ("amount", &amount.to_string()),
        ]))
}

/// Transfer Stable
/// Owner (governance contract) can execute transfer stable operation to send
/// `amount` of UST to `recipient` for community purpose
pub fn transfer_stable(
    deps: DepsMut,
    info: MessageInfo,
    recipient: String,
    amount: Uint128,
) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
    }

    // Validate recipient
    let recipient_address = deps.api.addr_validate(recipient.as_str())?;

    if config.spend_limit < amount {
        return Err(StdError::generic_err("Cannot spend more than spend_limit"));
    }

    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Bank(BankMsg::Send {
            to_address: recipient_address.to_string(),
            amount: vec![Coin {
                denom: config.stable_denom,
                amount,
            }],
        })])
        .add_attributes(vec![
            ("action", "spend"),
            ("recipient", recipient.as_str()),
            ("amount", &amount.to_string()),
        ]))
}

/// Sponsor Lotto
/// Owner (governance contract) can execute sponsor lotto operation for a given
/// `amount` of uusd, setting an optional `award` and `prize_distribution` parameters
pub fn sponsor_lotto(
    deps: DepsMut,
    info: MessageInfo,
    amount: Uint128,
    award: Option<bool>,
    prize_distribution: Option<[Decimal256; 7]>,
) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
    }

    let lotto = deps.api.addr_humanize(&config.lotto_contract)?.to_string();

    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: lotto,
            funds: vec![Coin {
                denom: config.stable_denom,
                amount,
            }],
            msg: to_binary(&LottoMsg::Sponsor {
                award,
                prize_distribution,
            })?,
        })])
        .add_attributes(vec![
            ("action", "sponsor_lotto"),
            ("amount", &amount.to_string()),
        ]))
}

/// Withdraw Sponsor
/// Owner (governance contract) can execute withdraw sponsor lotto operation
pub fn withdraw_sponsor(deps: DepsMut, info: MessageInfo) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
    }

    let lotto = deps.api.addr_humanize(&config.lotto_contract)?.to_string();

    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: lotto,
            funds: vec![],
            msg: to_binary(&LottoMsg::SponsorWithdraw {})?,
        })])
        .add_attributes(vec![("action", "withdraw_sponsor")]))
}

/// Swap
/// Owner can execute sweep function to swap
/// asset config native denom => GLOW token
pub fn execute_swap(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    amount: Uint128,
) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;

    // Check only owner can call
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
    }

    let glow_token = deps.api.addr_humanize(&config.glow_token)?;
    let terraswap_factory_addr = deps.api.addr_humanize(&config.terraswap_factory)?;

    let pair_info: PairInfo = query_pair_info(
        &deps.querier,
        terraswap_factory_addr,
        &[
            AssetInfo::NativeToken {
                denom: config.stable_denom.to_string(),
            },
            AssetInfo::Token {
                contract_addr: glow_token.to_string(),
            },
        ],
    )?;

    let contract_balance = query_balance(
        &deps.querier,
        env.contract.address,
        config.stable_denom.to_string(),
    )?;

    if amount > contract_balance {
        return Err(StdError::generic_err(
            "Amount of stable denom to spend cannot be greater than contract balance",
        ));
    }

    let swap_asset = Asset {
        info: AssetInfo::NativeToken {
            denom: config.stable_denom.to_string(),
        },
        amount,
    };

    Ok(Response::new()
        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pair_info.contract_addr,
            msg: to_binary(&TerraswapExecuteMsg::Swap {
                offer_asset: Asset {
                    amount,
                    ..swap_asset
                },
                max_spread: None,
                belief_price: None,
                to: None,
            })?,
            funds: vec![Coin {
                denom: config.stable_denom.clone(),
                amount,
            }],
        }))
        .add_attributes(vec![
            attr("action", "swap"),
            attr(
                "amount_spent",
                format!("{:?}{:?}", amount.to_string(), config.stable_denom),
            ),
        ]))
}

/// Burn
/// Owner (governance contract) can execute a burn operation of `amount` of Glow tokens
pub fn execute_burn(deps: DepsMut, info: MessageInfo, amount: Uint128) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;

    // Check only owner can call
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("Unauthorized"));
    }

    // The spend limit is sanity-check, as this contract manages a large sum of GLOW supply
    if config.spend_limit < amount {
        return Err(StdError::generic_err("Cannot burn more than spend_limit"));
    }

    let glow_token = deps.api.addr_humanize(&config.glow_token)?.to_string();

    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: glow_token,
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Burn { amount })?,
        })])
        .add_attributes(vec![("action", "burn"), ("amount", &amount.to_string())]))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
    }
}

pub fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = read_config(deps.storage)?;
    let resp = ConfigResponse {
        owner: deps.api.addr_humanize(&config.owner)?.to_string(),
        stable_denom: config.stable_denom,
        glow_token: deps.api.addr_humanize(&config.glow_token)?.to_string(),
        lotto_contract: deps.api.addr_humanize(&config.lotto_contract)?.to_string(),
        gov_contract: deps.api.addr_humanize(&config.gov_contract)?.to_string(),
        terraswap_factory: deps
            .api
            .addr_humanize(&config.terraswap_factory)?
            .to_string(),
        spend_limit: config.spend_limit,
    };

    Ok(resp)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, msg: MigrateMsg) -> StdResult<Response> {
    //migrate config
    let old_config = read_old_config(deps.storage)?;
    let new_config = Config {
        owner: old_config.owner,
        stable_denom: msg.stable_denom,
        glow_token: old_config.glow_token,
        lotto_contract: deps.api.addr_canonicalize(&msg.lotto_contract)?,
        gov_contract: deps.api.addr_canonicalize(&msg.gov_contract)?,
        terraswap_factory: deps.api.addr_canonicalize(&msg.terraswap_factory)?,
        spend_limit: old_config.spend_limit,
    };

    store_config(deps.storage, &new_config)?;

    Ok(Response::default())
}

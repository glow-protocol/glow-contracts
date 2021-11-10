#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use crate::error::ContractError;

use crate::state::{
    read_claimed, read_config, read_expiry_at_seconds, read_latest_stage, read_merkle_root,
    store_claimed, store_config, store_expiry_at_seconds, store_latest_stage, store_merkle_root,
    Config,
};

use glow_protocol::airdrop::{
    ConfigResponse, ExecuteMsg, ExpiryAtSecondsResponse, InstantiateMsg, IsClaimedResponse,
    LatestStageResponse, MerkleRootResponse, MigrateMsg, QueryMsg,
};

use glow_protocol::querier::query_token_balance;

use cosmwasm_std::{
    attr, to_binary, Binary, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
    Uint128, WasmMsg,
};

use cw20::Cw20ExecuteMsg;
use sha3::Digest;
use std::convert::TryInto;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    store_config(
        deps.storage,
        &Config {
            owner: deps.api.addr_canonicalize(&msg.owner)?,
            glow_token: deps.api.addr_canonicalize(&msg.glow_token)?,
        },
    )?;

    let stage: u8 = 0;
    store_latest_stage(deps.storage, stage)?;

    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateConfig { owner } => update_config(deps, info, owner),
        ExecuteMsg::WithdrawExpiredTokens { recipient } => {
            execute_withdraw_expired_tokens(deps, env, info, recipient)
        }
        ExecuteMsg::RegisterMerkleRoot {
            merkle_root,
            expiry_at_seconds,
        } => register_merkle_root(deps, env, info, merkle_root, expiry_at_seconds),
        ExecuteMsg::Claim {
            stage,
            amount,
            proof,
        } => claim(deps, env, info, stage, amount, proof),
    }
}

pub fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
) -> Result<Response, ContractError> {
    let mut config: Config = read_config(deps.as_ref().storage)?;
    if deps.api.addr_canonicalize(info.sender.as_str())? != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    if let Some(owner) = owner {
        config.owner = deps.api.addr_canonicalize(&owner)?;
    }

    store_config(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![attr("action", "update_config")]))
}

pub fn execute_withdraw_expired_tokens(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    recipient: String,
) -> Result<Response, ContractError> {
    // only the admin is authorized to withdraw
    let config: Config = read_config(deps.as_ref().storage)?;
    if deps.api.addr_canonicalize(info.sender.as_str())? != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    // the admin can only withdraw if all airdrop stage expiries have passed
    let latest_stage: u8 = read_latest_stage(deps.storage)?;

    // check that at least one stage has been created
    if latest_stage == 0 {
        return Err(ContractError::NoRegisteredAirdrops {});
    }

    // check that every stage has expired
    for stage in 1..=latest_stage {
        // If the expiry at seconds time has yet to pass for any stage, return err
        if read_expiry_at_seconds(deps.as_ref().storage, stage)? > env.block.time.seconds() {
            return Err(ContractError::AirdropNotExpired {});
        }
    }
    // get the glow cw20 contract address
    let glow_cw20_address = deps.api.addr_humanize(&config.glow_token)?;

    // get the glow balance of this airdrop contract
    let token_balance = query_token_balance(
        deps.as_ref(),
        glow_cw20_address.clone(),
        env.contract.address,
    )?;

    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: glow_cw20_address.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: recipient.clone(),
                amount: token_balance.into(),
            })?,
        })])
        .add_attributes(vec![
            ("action", "withdraw_expired_tokens"),
            ("to", &recipient),
            ("amount", &token_balance.to_string()),
        ]))
}

pub fn register_merkle_root(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    merkle_root: String,
    expiry_at_seconds: u64,
) -> Result<Response, ContractError> {
    let config: Config = read_config(deps.as_ref().storage)?;
    if deps.api.addr_canonicalize(info.sender.as_str())? != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    // Validate that the expiry_at_seconds is at a time in the future.
    if expiry_at_seconds <= env.block.time.seconds() {
        return Err(ContractError::InvalidExpiryAtSeconds {});
    }

    let mut root_buf: [u8; 32] = [0; 32];
    match hex::decode_to_slice(merkle_root.to_string(), &mut root_buf) {
        Ok(()) => {}
        _ => return Err(ContractError::InvalidHexMerkle {}),
    }

    let latest_stage: u8 = read_latest_stage(deps.storage)?;
    let stage = latest_stage + 1;

    store_merkle_root(deps.storage, stage, merkle_root.to_string())?;
    store_latest_stage(deps.storage, stage)?;
    store_expiry_at_seconds(deps.storage, stage, expiry_at_seconds)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "register_merkle_root"),
        attr("stage", stage.to_string()),
        attr("merkle_root", merkle_root),
        attr("expiry_at_seconds", expiry_at_seconds.to_string()),
    ]))
}

pub fn claim(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    stage: u8,
    amount: Uint128,
    proof: Vec<String>,
) -> Result<Response, ContractError> {
    let config: Config = read_config(deps.storage)?;
    let merkle_root: String = read_merkle_root(deps.storage, stage)?;

    let user_raw = deps.api.addr_canonicalize(info.sender.as_str())?;

    // If user claimed target stage, return err
    if read_claimed(deps.as_ref().storage, &user_raw, stage)? {
        return Err(ContractError::AlreadyClaimed {});
    }

    // If the expiry at seconds time has passed, return err
    if read_expiry_at_seconds(deps.as_ref().storage, stage)? <= env.block.time.seconds() {
        return Err(ContractError::AirdropExpired {});
    }

    let user_input: String = info.sender.to_string() + &amount.to_string();
    let mut hash: [u8; 32] = sha3::Keccak256::digest(user_input.as_bytes())
        .as_slice()
        .try_into()
        .expect("Wrong length");

    for p in proof {
        let mut proof_buf: [u8; 32] = [0; 32];
        match hex::decode_to_slice(p, &mut proof_buf) {
            Ok(()) => {}
            _ => return Err(ContractError::InvalidHexProof {}),
        }

        hash = if bytes_cmp(hash, proof_buf) == std::cmp::Ordering::Less {
            sha3::Keccak256::digest(&[hash, proof_buf].concat())
                .as_slice()
                .try_into()
                .expect("Wrong length")
        } else {
            sha3::Keccak256::digest(&[proof_buf, hash].concat())
                .as_slice()
                .try_into()
                .expect("Wrong length")
        };
    }

    let mut root_buf: [u8; 32] = [0; 32];
    hex::decode_to_slice(merkle_root, &mut root_buf).unwrap();
    if root_buf != hash {
        return Err(ContractError::MerkleVerification {});
    }

    // Update claim index to the current stage
    store_claimed(deps.storage, &user_raw, stage)?;

    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.addr_humanize(&config.glow_token)?.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: info.sender.to_string(),
                amount,
            })?,
        })])
        .add_attributes(vec![
            ("action", "claim"),
            ("stage", &stage.to_string()),
            ("address", info.sender.as_str()),
            ("amount", &amount.to_string()),
        ]))
}

fn bytes_cmp(a: [u8; 32], b: [u8; 32]) -> std::cmp::Ordering {
    let mut i = 0;
    while i < 32 {
        match a[i].cmp(&b[i]) {
            std::cmp::Ordering::Greater => return std::cmp::Ordering::Greater,
            std::cmp::Ordering::Less => return std::cmp::Ordering::Less,
            _ => i += 1,
        }
    }

    std::cmp::Ordering::Equal
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::MerkleRoot { stage } => to_binary(&query_merkle_root(deps, stage)?),
        QueryMsg::LatestStage {} => to_binary(&query_latest_stage(deps)?),
        QueryMsg::IsClaimed { stage, address } => {
            to_binary(&query_is_claimed(deps, stage, address)?)
        }
        QueryMsg::ExpiryAtSeconds { stage } => to_binary(&query_expiry_at_seconds(deps, stage)?),
    }
}

pub fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let state = read_config(deps.storage)?;
    let resp = ConfigResponse {
        owner: deps.api.addr_humanize(&state.owner)?.to_string(),
        glow_token: deps.api.addr_humanize(&state.glow_token)?.to_string(),
    };

    Ok(resp)
}

pub fn query_merkle_root(deps: Deps, stage: u8) -> StdResult<MerkleRootResponse> {
    let merkle_root = read_merkle_root(deps.storage, stage)?;
    let resp = MerkleRootResponse { stage, merkle_root };

    Ok(resp)
}

pub fn query_latest_stage(deps: Deps) -> StdResult<LatestStageResponse> {
    let latest_stage = read_latest_stage(deps.storage)?;
    let resp = LatestStageResponse { latest_stage };

    Ok(resp)
}

pub fn query_is_claimed(deps: Deps, stage: u8, address: String) -> StdResult<IsClaimedResponse> {
    let user_raw = deps.api.addr_canonicalize(&address)?;
    let resp = IsClaimedResponse {
        is_claimed: read_claimed(deps.storage, &user_raw, stage)?,
    };

    Ok(resp)
}

pub fn query_expiry_at_seconds(deps: Deps, stage: u8) -> StdResult<ExpiryAtSecondsResponse> {
    let expiry_at_seconds = read_expiry_at_seconds(deps.storage, stage)?;
    let resp = ExpiryAtSecondsResponse { expiry_at_seconds };

    Ok(resp)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    Ok(Response::default())
}

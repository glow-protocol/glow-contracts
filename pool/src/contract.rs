use cosmwasm_std::{
    from_binary, log, to_binary, Api, Binary, CanonicalAddr, HumanAddr, CosmosMsg, Env, Extern,
    HandleResponse, HandleResult, InitResponse, InitResult, Querier, StdError, StdResult, Storage,
    Uint128, WasmMsg, BankMsg, Coin
};

use crate::msg::{HandleMsg, InitMsg, QueryMsg, Cw20HookMsg, ConfigResponse, StateResponse};
use crate::state::{read_config, read_state, store_config, store_state,
                   Config, State, read_sequence_info, store_sequence_info};
use crate::prize_strategy::{is_valid_sequence};
use cosmwasm_bignumber::{Uint256,Decimal256};
use serde::__private::de::IdentifierDeserializer;
use snafu::guide::examples::backtrace::Error::UsedInTightLoop;

use cw20::{Cw20CoinHuman, Cw20ReceiveMsg, Cw20HandleMsg, MinterResponse};

use terraswap::hook::InitHook;
use terraswap::token::InitMsg as TokenInitMsg;

use moneymarket::market::HandleMsg as AnchorMsg;

// We are asking the contract owner to provide an initial reserve to start accruing interest
// Also, reserve accrues interest but it's not entitled to tickets, so no prizes
pub const INITIAL_DEPOSIT_AMOUNT: u128 = 10_000_000_000; // fund reserve with 10k
pub const SEQUENCE_DIGITS: u8 = 5;

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> InitResult {
    let initial_deposit = env
        .message
        .sent_funds
        .iter()
        .find(|c| c.denom == msg.stable_denom)
        .map(|c| c.amount)
        .unwrap_or_else(|| Uint128::zero());

    if initial_deposit != Uint128(INITIAL_DEPOSIT_AMOUNT) {
        return Err(StdError::generic_err(format!(
            "Must deposit initial reserve funds {:?}{:?}",
            INITIAL_DEPOSIT_AMOUNT,
            msg.stable_denom.clone()
        )));
    }

    store_config(
        &mut deps.storage,
        &Config {
            contract_addr: deps.api.canonical_address(&env.contract.address)?,
            owner: deps.api.canonical_address(&msg.owner)?,
            b_terra_contract: CanonicalAddr::default(),
            stable_denom: msg.stable_denom.clone(),
            anchor_contract: deps.api.canonical_address(&msg.anchor_contract)?,
            lottery_interval: msg.lottery_interval,
            block_time: msg.block_time,
            ticket_prize: msg.ticket_prize,
            reserve_factor: msg.reserve_factor,
            split_factor: msg.split_factor,
            ticket_exchange_rate: msg.ticket_exchange_rate,
        },
    )?;

    store_state(
        &mut deps.storage,
        &State {
            total_tickets: Uint256::zero(),
            total_reserve: Decimal256::from_uint256(initial_deposit),
            last_interest: Decimal256::zero(),
            total_accrued_interest: Decimal256::zero(),
            award_available: Decimal256::zero(),
            next_lottery_time: msg.lottery_interval,
            spendable_balance: Decimal256::zero(),
            total_deposits: Decimal256::zero(),
            total_assets: Decimal256::from_uint256(initial_deposit),
        },
    )?;

    Ok(InitResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
            code_id: msg.b_terra_code_id,
            send: vec![],
            label: None,
            msg: to_binary(&TokenInitMsg {
                name: format!(
                    "Barbell Terra {}",
                    msg.stable_denom[1..].to_uppercase()
                ),
                symbol: format!(
                    "b{}T",
                    msg.stable_denom[1..(msg.stable_denom.len() - 1)].to_uppercase()
                ),
                decimals: 6u8,
                initial_balances: vec![Cw20CoinHuman {
                    address: env.contract.address.clone(),
                    amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
                }],
                mint: Some(MinterResponse {
                    minter: env.contract.address.clone(),
                    cap: None,
                }),
                init_hook: Some(InitHook {
                    contract_addr: env.contract.address,
                    msg: to_binary(&HandleMsg::RegisterSTerra {})?,
                }),
            })?,
        })],
        log: vec![],
    })
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> HandleResult {
    match msg {
        HandleMsg::Receive(msg) => receive_cw20(deps, env, msg),
        HandleMsg::DepositStable {} => deposit_stable(deps, env),
        HandleMsg::SingleDeposit {combination} => single_deposit(deps, env, combination),
        HandleMsg::RegisterSTerra {} => register_b_terra(deps, env),
        HandleMsg::UpdateConfig {
            owner,
            period_prize
        } => update_config(deps, env, owner, period_prize)
    }
}

pub fn receive_cw20<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    cw20_msg: Cw20ReceiveMsg,
) -> HandleResult {
    let contract_addr = env.message.sender.clone();
    if let Some(msg) = cw20_msg.msg {
        match from_binary(&msg)? {
            Cw20HookMsg::RedeemStable {} => {
                // only core-pool contract can execute this message
                let config: Config = read_config(&deps.storage)?;
                if deps.api.canonical_address(&contract_addr)? != config.b_terra_contract {
                    return Err(StdError::unauthorized());
                }

                redeem_stable(deps, env, cw20_msg.sender, cw20_msg.amount)
            }
        }
    } else {
        Err(StdError::generic_err(
            "Invalid request: \"redeem stable\" message not included in request",
        ))
    }
}

// Return stablecoins to user and burn b_terra
pub fn redeem_stable<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    sender: HumanAddr,
    burn_amount: Uint128
) -> HandleResult {
    let config = read_config(&deps.storage)?;
    let mut state = read_state(&deps.storage)?;

    let redeem_amount = Uint256::from(burn_amount) / config.ticket_exchange_rate; //TODO: create a proper exchange rate function

    //TODO: assert redeem amount fn here
    //TODO: update internal balances such as total_assets and total_tickets

    store_state(&mut deps.storage, &state)?; //TODO: check where the state has been modified

    Ok(HandleResponse {
        messages: vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps.api.human_address(&config.b_terra_contract)?,
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Burn {
                    amount: burn_amount,
                })?,
            }),
            CosmosMsg::Bank(BankMsg::Send {
                from_address: env.contract.address,
                to_address: sender,
                amount: vec![
                    Coin {
                        denom: config.stable_denom,
                        amount: redeem_amount.into(),
                    },
                ],
            }),
        ],
        log: vec![
            log("action", "redeem_stable"),
            log("burn_amount", burn_amount),
            log("redeem_amount", redeem_amount),
        ],
        data: None,
    })
}

// Deposit UST into the pool contract.
pub fn deposit_stable<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let config = read_config(&deps.storage)?;

    // Check deposit is in base stable denom
    let deposit_amount = env
        .message
        .sent_funds
        .iter()
        .find(|c| c.denom == config.stable_denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    if deposit_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Deposit amount must be greater than 0 {}",
            config.stable_denom
        )));
    }
    let mint_amount =  deposit_amount * config.ticket_exchange_rate;

    let mut state = read_state(&deps.storage)?;

    state.total_assets +=  Decimal256::from_uint256(deposit_amount);

    store_state(&mut deps.storage, &state)?;

    // Mint bUST for the sender and deposit UST to Anchor Money market

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.b_terra_contract)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: env.message.sender.clone(),
                amount: mint_amount.into(),
            })?,
        }),
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.anchor_contract)?,
            send: vec![
                Coin {
                    denom: config.stable_denom,
                    amount: Uint128::from(deposit_amount),
                }
            ],
            msg: to_binary(&AnchorMsg::DepositStable {})?
        })
        ],
        log: vec![
            log("action", "deposit_stable"),
            log("depositor", env.message.sender),
            log("mint_amount", mint_amount),
            log("deposit_amount", deposit_amount),
        ],
        data: None,
    })
}

// Single Deposit buys one ticket
pub fn single_deposit<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    combination: String
) -> HandleResult {

    let config = read_config(&deps.storage)?;
    let mut state = read_state(&deps.storage)?;

    // Check deposit is in base stable denom
    let deposit_amount = env
        .message
        .sent_funds
        .iter()
        .find(|c| c.denom == config.stable_denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    if deposit_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Deposit amount must be greater than 0 {}",
            config.stable_denom
        )));
    }

    //TODO: consider accepting any amount and moving the rest to spendable balance
    if deposit_amount != config.ticket_prize {
        return Err(StdError::generic_err(format!(
            "Deposit amount must be equal to a ticket prize {} {}",
            config.ticket_prize,
            config.stable_denom
        )));
    }

    //TODO: add a time buffer here with block_time
    if env.block.time > state.next_lottery_time {
        return Err(StdError::generic_err(
            "Current lottery is about to start, wait until the next one begins"
        ))
    }


    if !is_valid_sequence(&combination, SEQUENCE_DIGITS) {
        return Err(StdError::generic_err(format!(
            "Ticket sequence must be {} characters between 0-9",
            SEQUENCE_DIGITS
        )));
    }

    // TODO: query anchor to check how much aUST we will get from this deposit, also is this possible?
    // multiply aUST amount by the split factor, and store the amount of aUST will
    // get direct interest for the user and the amount of aUST that's going to go
    // to the lottery pool


    let depositor = deps.api.canonical_address(&env.message.sender)?;
    store_sequence_info(&mut deps.storage, depositor, &combination)?;

    state.total_tickets += Uint256::from(1); //TODO: check if there is a cleaner way to do this
    state.total_deposits += Decimal256::from_uint256(deposit_amount);
    state.total_assets += Decimal256::from_uint256(deposit_amount);

    store_state(&mut deps.storage, &state)?;


    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.anchor_contract)?,
            send: vec![
                Coin {
                    denom: config.stable_denom,
                    amount: Uint128::from(deposit_amount),
                }
            ],
            msg: to_binary(&AnchorMsg::DepositStable {})?
        })
        ],
        log: vec![
            log("action", "deposit_stable"),
            log("depositor", env.message.sender),
            log("mint_amount", mint_amount),
            log("deposit_amount", deposit_amount),
        ],
        data: None,
    })
}

// Register b_terra_contract in the core_pool config.
pub fn register_b_terra<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> HandleResult {
    let mut config: Config = read_config(&deps.storage)?;
    if config.b_terra_contract != CanonicalAddr::default() {
        return Err(StdError::unauthorized());
    }
    config.b_terra_contract = deps.api.canonical_address(&env.message.sender)?;
    store_config(&mut deps.storage, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![log("bterra", env.message.sender)],
        data: None,
    })
}

pub fn update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: Option<HumanAddr>,
    ticket_price: Option<u64>
)-> HandleResult {

    let mut config: Config = read_config(&deps.storage)?;

    // check permission
    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }
    // change owner of the pool contract
    if let Some(owner) = owner {
        config.owner = deps.api.canonical_address(&owner)?;
    }
    // TODO: period prize is not doing anything, just for demo purposes
    if let Some(ticket_prize) = ticket_price {
        config.ticket_prize = ticket_prize;
    }

    store_config(&mut deps.storage, &config)?;
    Ok(HandleResponse {
        messages: vec![],
        log: vec![log("action", "update_config")],
        data: None,
    })
}



pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::State { block_height} => to_binary(&query_state(deps, block_height)?)
    }
}

pub fn query_config<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<ConfigResponse> {
    let config: Config = read_config(&deps.storage)?;

    Ok(ConfigResponse {
        owner: deps.api.human_address(&config.owner)?,
        stable_denom: config.stable_denom,
        anchor_contract: deps.api.human_address(&config.anchor_contract)?,
        period_prize: config.period_prize,
        ticket_exchange_rate: config.ticket_exchange_rate
    })
}

pub fn query_state<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>, block_height: Option<u64>) -> StdResult<StateResponse> {
    let state: State = read_state(&deps.storage)?;

    //Todo: add block_height logic

    Ok(StateResponse {
        total_tickets: state.total_tickets,
        total_reserves: state.total_reserves,
        last_interest: state.last_interest,
        total_accrued_interest: state.total_accrued_interest,
        award_available: state.award_available,
        total_assets: state.total_assets,
    })
}

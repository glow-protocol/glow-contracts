use cosmwasm_std::{
    from_binary, log, to_binary, Api, BankMsg, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern,
    HandleResponse, HandleResult, HumanAddr, InitResponse, InitResult, Querier, StdError,
    StdResult, Storage, Uint128, WasmMsg,
};

use crate::msg::{ConfigResponse, Cw20HookMsg, HandleMsg, InitMsg, QueryMsg, StateResponse};
use crate::prize_strategy::{_handle_prize, execute_lottery, is_valid_sequence};
use crate::querier::query_exchange_rate;
use crate::state::{
    read_config, read_depositor_info, read_sequence_info, read_state, sequence_bucket,
    store_config, store_depositor_info, store_sequence_info, store_state, Config, DepositorInfo,
    State,
};

use cosmwasm_bignumber::{Decimal256, Uint256};
use serde::__private::de::IdentifierDeserializer;
use snafu::guide::examples::backtrace::Error::UsedInTightLoop;

use cw20::{Cw20CoinHuman, Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};

use terraswap::hook::InitHook;
use terraswap::token::InitMsg as TokenInitMsg;

use crate::claims::{claim_deposits, Claim};
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
            a_terra_contract: CanonicalAddr::default(),
            stable_denom: msg.stable_denom.clone(),
            anchor_contract: deps.api.canonical_address(&msg.anchor_contract)?,
            lottery_interval: msg.lottery_interval,
            block_time: msg.block_time,
            ticket_prize: msg.ticket_prize,
            prize_distribution: msg.prize_distribution,
            reserve_factor: msg.reserve_factor,
            split_factor: msg.split_factor,
            ticket_exchange_rate: msg.ticket_exchange_rate,
            unbonding_period: msg.unbonding_period,
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
            current_lottery: Uint256::zero(),
            next_lottery_time: msg.lottery_interval,
            spendable_balance: Decimal256::zero(),
            current_balance: Uint256::from(initial_deposit),
            total_deposits: Decimal256::zero(),
            total_lottery_deposits: Decimal256::zero(),
            total_assets: Decimal256::from_uint256(initial_deposit),
        },
    )?;

    Ok(InitResponse::default())
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> HandleResult {
    match msg {
        HandleMsg::SingleDeposit { combination } => single_deposit(deps, env, combination),
        HandleMsg::Withdraw { amount } => withdraw(deps, env, amount),
        HandleMsg::ExecuteLottery {} => execute_lottery(deps, env),
        HandleMsg::_HandlePrize {} => _handle_prize(deps, env, info),
        HandleMsg::UpdateConfig {
            owner,
            period_prize,
        } => update_config(deps, env, owner, ticket_prize),
    }
}

// Single Deposit buys one ticket
pub fn single_deposit<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    combination: String,
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
            config.ticket_prize, config.stable_denom
        )));
    }

    //TODO: add a time buffer here with block_time
    if env.block.time > state.next_lottery_time {
        return Err(StdError::generic_err(
            "Current lottery is about to start, wait until the next one begins",
        ));
    }

    if !is_valid_sequence(&combination, SEQUENCE_DIGITS) {
        return Err(StdError::generic_err(format!(
            "Ticket sequence must be {} characters between 0-9",
            SEQUENCE_DIGITS
        )));
    }

    // Store ticket sequence in bucket
    store_sequence_info(&mut deps.storage, depositor, &combination)?;

    let depositor = deps.api.canonical_address(&env.message.sender)?;
    let mut depositor_info: DepositorInfo = read_depositor_info(&deps.storage, &depositor)?;

    let anchor_exchange_rate =
        query_exchange_rate(&deps, &deps.api.human_address(&config.anchor_contract)?)?;
    // add amount of aUST entitled from the deposit
    let minted_amount = Decimal256::from_uint256(deposit_amount) / anchor_exchange_rate;
    depositor_info.deposit_amount += minted_amount;
    depositor_info.tickets.push(combination);

    store_depositor_info(&mut deps.storage, &depositor, &depositor_info);

    state.total_tickets += Uint256::one();
    state.total_deposits += minted_amount;
    state.total_lottery_deposits += minted_amount * config.split_factor;
    state.total_assets += Decimal256::from_uint256(deposit_amount); //TODO: not doing anything yet

    store_state(&mut deps.storage, &state)?;

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.anchor_contract)?,
            send: vec![Coin {
                denom: config.stable_denom,
                amount: Uint128::from(deposit_amount),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        })],
        log: vec![
            log("action", "deposit_stable"),
            log("depositor", env.message.sender),
            log("mint_amount", mint_amount),
            log("deposit_amount", deposit_amount),
        ],
        data: None,
    })
}

// TODO: pub fn withdraw() - burn tickets and place funds in the unbonding_info as claims
// TODO: burn specific tickets - combinations: Option<Vec<String>>
pub fn withdraw<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    amount: Uint128, //amount of tickets
) -> HandleResult {
    let config = read_config(&deps.storage)?;
    let mut state = read_state(&deps.storage)?;

    let sender_raw = deps.api.canonical_address(&env.message.sender)?;
    let mut depositor: DepositorInfo = read_depositor_info(&deps.storage, &sender_raw)?;

    if amount == Uint128::zero() {
        return Err(StdError::generic_err(
            "Amount of tickets must be greater than zero",
        ));
    }

    if amount > depositor.tickets.len() as Uint128 {
        return Err(StdError::generic_err(format!(
            "User has {} tickets but {} tickets were requested to be withdrawn",
            amount,
            depositor.tickets.len()
        )));
    }

    let mut tickets = depositor.tickets.clone();
    let ticket_removed: Vec<String> = tickets.drain(0..amount).collect();
    depositor.tickets = tickets;
    let unbonding_amount = config.ticket_prize * amount;
    depositor.deposit_amount -= unbonding_amount;

    depositor.unbonding_info.push(Claim {
        amount: unbonding_amount,
        release_at: config.unbonding_period.after(&env.block),
    })?;

    // Remove depositor's address from holders Sequence
    ticket_removed.iter().map(|seq| {
        let mut holders: Vec<CanonicalAddr> = read_sequence_info(&mut deps.storage, seq)?;
        let index = holders.iter().position(|x| *x == sender_raw).unwrap();
        holders.remove(index)?;
        sequence_bucket(&mut deps.storage).save(seq.as_bytes(), &holders)?;
    });

    // TODO: update global variables

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "withdraw_ticket")
            log("tickets_amount", amount)
        ],
        data: None,
    })
}

// Send available UST to user from current redeemable balance and unbonded deposits
pub fn claim<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    amount: Option<Uint128>,
) -> HandleResult {
    if amount.unwrap() == 0 {
        return Err(StdError::generic_err(
            "Claim amount must be greater than zero",
        ));
    }

    // TODO: check balance of the contract is sufficient

    let mut state = read_state(&deps.storage)?;
    let config = read_config(&deps.storage)?;

    let sender_raw = deps.api.canonical_address(&env.message.sender)?;
    let mut to_send = claim_deposits(&mut deps.storage, &sender_raw, &env.block, amount)?;
    //TODO: doing two consecutive reads here, need to refactor
    let mut depositor: DepositorInfo = read_depositor_info(&deps.storage, &sender_raw)?;
    to_send += depositor.redeemable_amount;
    if to_send == Uint128(0) {
        return Err(StdError::generic_err(
            "Depositor does not have any amount to claim",
        ));
    }
    //TODO: double-check if there is enough balance to send in the contract
    //TODO: need to redeem amount of aUST to UST

    //TODO: update total assets and other global variables

    //TODO: we may deduct some tax in redemptions

    depositor.redeemable_amount = Uint128::zero();
    store_depositor_info(&mut deps.storage, &sender_raw, &depositor)?;

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Bank(BankMsg::Send {
            from_address: env.contract.address,
            to_address: env.message.sender,
            amount: vec![Coin {
                denom: config.stable_denom,
                amount: to_send.into(),
            }],
        })],
        log: vec![log("action", "claim"), log("redeemed_amount", to_send)],
        data: None,
    })
}

pub fn update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: Option<HumanAddr>,
    ticket_price: Option<u64>,
) -> HandleResult {
    let mut config: Config = read_config(&deps.storage)?;

    // check permission
    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }
    // change owner of the pool contract
    if let Some(owner) = owner {
        config.owner = deps.api.canonical_address(&owner)?;
    }

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
        QueryMsg::State { block_height } => to_binary(&query_state(deps, block_height)?),
    }
}

pub fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config: Config = read_config(&deps.storage)?;

    Ok(ConfigResponse {
        owner: deps.api.human_address(&config.owner)?,
        stable_denom: config.stable_denom,
        anchor_contract: deps.api.human_address(&config.anchor_contract)?,
        period_prize: config.period_prize,
        ticket_exchange_rate: config.ticket_exchange_rate,
    })
}

pub fn query_state<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    block_height: Option<u64>,
) -> StdResult<StateResponse> {
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

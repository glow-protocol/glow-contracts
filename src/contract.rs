use cosmwasm_std::{
    coin, log, to_binary, Api, BankMsg, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern,
    HandleResponse, HandleResult, HumanAddr, InitResponse, InitResult, Querier, StdError,
    StdResult, Storage, Uint128, WasmMsg,
};

use crate::msg::{
    ConfigResponse, DepositorInfoResponse, DepositorsInfoResponse, HandleMsg, InitMsg,
    LotteryInfoResponse, QueryMsg, StateResponse,
};
use crate::prize_strategy::{_handle_prize, execute_lottery, is_valid_sequence};
use crate::querier::{query_balance, query_exchange_rate, query_token_balance};
use crate::state::{
    read_config, read_depositor_info, read_depositors, read_lottery_info, read_sequence_info,
    read_state, sequence_bucket, store_config, store_depositor_info, store_sequence_info,
    store_state, Config, DepositorInfo, State,
};

use cosmwasm_bignumber::{Decimal256, Uint256};

use cw0::Duration;
use cw20::Cw20HandleMsg;

use crate::claims::{claim_deposits, Claim};
use moneymarket::market::{Cw20HookMsg, EpochStateResponse, HandleMsg as AnchorMsg};
use moneymarket::querier::deduct_tax;
use std::ops::{Add, Sub};

// We are asking the contract owner to provide an initial reserve to start accruing interest
// Also, reserve accrues interest but it's not entitled to tickets, so no prizes
pub const INITIAL_DEPOSIT_AMOUNT: u128 = 100_000_000; // fund reserve with $100
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
        .unwrap_or_else(Uint128::zero);

    if initial_deposit != Uint128(INITIAL_DEPOSIT_AMOUNT) {
        return Err(StdError::generic_err(format!(
            "Must deposit initial reserve funds {:?}{:?}",
            INITIAL_DEPOSIT_AMOUNT, msg.stable_denom
        )));
    }

    store_config(
        &mut deps.storage,
        &Config {
            contract_addr: deps.api.canonical_address(&env.contract.address)?,
            owner: deps.api.canonical_address(&msg.owner)?,
            a_terra_contract: deps.api.canonical_address(&msg.aterra_contract)?,
            stable_denom: msg.stable_denom.clone(),
            anchor_contract: deps.api.canonical_address(&msg.anchor_contract)?,
            lottery_interval: Duration::Time(msg.lottery_interval),
            block_time: Duration::Time(msg.block_time),
            ticket_prize: msg.ticket_prize,
            prize_distribution: msg.prize_distribution,
            reserve_factor: msg.reserve_factor,
            split_factor: msg.split_factor,
            unbonding_period: Duration::Time(msg.unbonding_period),
        },
    )?;

    store_state(
        &mut deps.storage,
        &State {
            total_tickets: Uint256::zero(),
            total_reserve: Decimal256::from_uint256(initial_deposit),
            total_deposits: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            shares_supply: Decimal256::zero(),
            award_available: Decimal256::zero(),
            spendable_balance: Decimal256::zero(),
            current_balance: Uint256::from(initial_deposit),
            current_lottery: 0,
            next_lottery_time: Duration::Time(msg.lottery_interval).after(&env.block),
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
        HandleMsg::BatchDeposit { combinations } => batch_deposit(deps, env, combinations),
        HandleMsg::Gift {
            combinations,
            recipient,
        } => gift_tickets(deps, env, combinations, recipient),
        HandleMsg::Sponsor {} => sponsor(deps, env),
        HandleMsg::Withdraw { amount, sequence } => withdraw(deps, env, amount, sequence),
        HandleMsg::Claim { amount } => claim(deps, env, amount),
        HandleMsg::ExecuteLottery {} => execute_lottery(deps, env),
        HandleMsg::_HandlePrize {} => _handle_prize(deps, env),
        HandleMsg::UpdateConfig {
            owner,
            lottery_interval,
            block_time,
            ticket_prize,
            prize_distribution,
            reserve_factor,
            split_factor,
            unbonding_period,
        } => update_config(
            deps,
            env,
            owner,
            lottery_interval,
            block_time,
            ticket_prize,
            prize_distribution,
            reserve_factor,
            split_factor,
            unbonding_period,
        ),
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

    //TODO: consider accepting any amount and moving the rest to redeemable_amount balance
    if deposit_amount != config.ticket_prize * Uint256::one() {
        return Err(StdError::generic_err(format!(
            "Deposit amount must be equal to a ticket prize: {} {}",
            config.ticket_prize, config.stable_denom
        )));
    }

    //TODO: add a time buffer here with block_time
    if state.next_lottery_time.is_expired(&env.block) {
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

    let depositor = deps.api.canonical_address(&env.message.sender)?;
    let mut depositor_info: DepositorInfo = read_depositor_info(&deps.storage, &depositor);

    // query exchange_rate from anchor money market
    let epoch_state: EpochStateResponse =
        query_exchange_rate(&deps, &deps.api.human_address(&config.anchor_contract)?)?;

    // Discount tx taxes
    let net_coin_amount = deduct_tax(deps, coin(deposit_amount.into(), "uusd"))?;
    let amount = net_coin_amount.amount;

    // add amount of aUST entitled from the deposit
    let minted_amount = Decimal256::from_uint256(amount) / epoch_state.exchange_rate;
    depositor_info.deposit_amount = depositor_info
        .deposit_amount
        .add(Decimal256::from_uint256(deposit_amount));
    depositor_info.shares = depositor_info.shares.add(minted_amount);
    depositor_info.tickets.push(combination.clone());

    // Update depositor information
    store_depositor_info(&mut deps.storage, &depositor, &depositor_info)?;
    // Store ticket sequence in bucket
    store_sequence_info(&mut deps.storage, depositor, &combination)?;

    // Update global state
    state.total_tickets = state.total_tickets.add(Uint256::one());
    state.total_deposits = state
        .total_deposits
        .add(Decimal256::from_uint256(deposit_amount));
    state.shares_supply = state.shares_supply.add(minted_amount);
    state.lottery_deposits = state
        .lottery_deposits
        .add(Decimal256::from_uint256(deposit_amount) * config.split_factor);
    store_state(&mut deps.storage, &state)?;

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.anchor_contract)?,
            send: vec![Coin {
                denom: config.stable_denom,
                amount,
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        })],
        log: vec![
            log("action", "single_deposit"),
            log("depositor", env.message.sender),
            log("deposit_amount", deposit_amount),
            log("shares_minted", minted_amount),
        ],
        data: None,
    })
}

// Batch deposit to buy several tickets at once
pub fn batch_deposit<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    combinations: Vec<String>,
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

    let amount_tickets = combinations.len() as u64;

    //TODO: consider accepting any amount and moving the rest to redeemable_amount balance
    let required_amount = config.ticket_prize * Uint256::from(amount_tickets);
    if deposit_amount != required_amount {
        return Err(StdError::generic_err(format!(
            "Deposit amount required for {} tickets is {} {}",
            amount_tickets, required_amount, config.stable_denom
        )));
    }

    //TODO: add a time buffer here with block_time
    if state.next_lottery_time.is_expired(&env.block) {
        return Err(StdError::generic_err(
            "Current lottery is about to start, wait until the next one begins",
        ));
    }

    for combination in combinations.clone() {
        if !is_valid_sequence(&combination, SEQUENCE_DIGITS) {
            return Err(StdError::generic_err(format!(
                "Ticket sequence must be {} characters between 0-9",
                SEQUENCE_DIGITS
            )));
        }
    }

    let depositor = deps.api.canonical_address(&env.message.sender)?;
    let mut depositor_info: DepositorInfo = read_depositor_info(&deps.storage, &depositor);

    // query exchange_rate from anchor money market
    let epoch_state: EpochStateResponse =
        query_exchange_rate(&deps, &deps.api.human_address(&config.anchor_contract)?)?;

    // Discount tx taxes
    let net_coin_amount = deduct_tax(deps, coin(deposit_amount.into(), "uusd"))?;
    let amount = net_coin_amount.amount;

    // add amount of aUST entitled from the deposit
    let minted_amount = Decimal256::from_uint256(amount) / epoch_state.exchange_rate;
    depositor_info.deposit_amount = depositor_info
        .deposit_amount
        .add(Decimal256::from_uint256(deposit_amount));
    depositor_info.shares = depositor_info.shares.add(minted_amount);

    for combination in combinations.clone() {
        depositor_info.tickets.push(combination.clone());
        // Store ticket sequence in bucket
        store_sequence_info(&mut deps.storage, depositor.clone(), &combination)?;
    }

    // Update depositor information
    store_depositor_info(&mut deps.storage, &depositor, &depositor_info)?;

    // Update global state
    state.total_tickets = state.total_tickets.add(Uint256::from(amount_tickets));
    state.total_deposits = state
        .total_deposits
        .add(Decimal256::from_uint256(deposit_amount));
    state.shares_supply = state.shares_supply.add(minted_amount);
    state.lottery_deposits = state
        .lottery_deposits
        .add(Decimal256::from_uint256(deposit_amount) * config.split_factor);
    store_state(&mut deps.storage, &state)?;

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.anchor_contract)?,
            send: vec![Coin {
                denom: config.stable_denom,
                amount,
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        })],
        log: vec![
            log("action", "batch_deposit"),
            log("depositor", env.message.sender),
            log("deposit_amount", deposit_amount),
            log("shares_minted", minted_amount),
        ],
        data: None,
    })
}

// Gift several tickets at once to a given address
pub fn gift_tickets<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    combinations: Vec<String>,
    to: HumanAddr,
) -> HandleResult {
    if to == env.message.sender {
        return Err(StdError::generic_err(
            "You cannot gift tickets to yourself, just make a regular deposit",
        ));
    }

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
            "Deposit amount to gift must be greater than 0 {}",
            config.stable_denom
        )));
    }

    let amount_tickets = combinations.len() as u64;

    //TODO: consider accepting any amount and moving the rest to redeemable_amount balance
    let required_amount = config.ticket_prize * Uint256::from(amount_tickets);
    if deposit_amount != required_amount {
        return Err(StdError::generic_err(format!(
            "Deposit amount required to gift {} tickets is {} {}",
            amount_tickets, required_amount, config.stable_denom
        )));
    }

    //TODO: add a time buffer here with block_time
    if state.next_lottery_time.is_expired(&env.block) {
        return Err(StdError::generic_err(
            "Current lottery is about to start, wait until the next one begins",
        ));
    }

    for combination in combinations.clone() {
        if !is_valid_sequence(&combination, SEQUENCE_DIGITS) {
            return Err(StdError::generic_err(format!(
                "Ticket sequence must be {} characters between 0-9",
                SEQUENCE_DIGITS
            )));
        }
    }

    let recipient = deps.api.canonical_address(&to)?;
    let mut depositor_info: DepositorInfo = read_depositor_info(&deps.storage, &recipient);

    // query exchange_rate from anchor money market
    let epoch_state: EpochStateResponse =
        query_exchange_rate(&deps, &deps.api.human_address(&config.anchor_contract)?)?;

    // Discount tx taxes
    let net_coin_amount = deduct_tax(deps, coin(deposit_amount.into(), "uusd"))?;
    let amount = net_coin_amount.amount;

    // add amount of aUST entitled from the deposit
    let minted_amount = Decimal256::from_uint256(amount) / epoch_state.exchange_rate;
    depositor_info.deposit_amount = depositor_info
        .deposit_amount
        .add(Decimal256::from_uint256(deposit_amount));
    depositor_info.shares = depositor_info.shares.add(minted_amount);

    for combination in combinations.clone() {
        depositor_info.tickets.push(combination.clone());
        // Store ticket sequence in bucket
        // TODO: should pass depositor as reference
        store_sequence_info(&mut deps.storage, recipient.clone(), &combination)?;
    }

    // Update depositor information
    store_depositor_info(&mut deps.storage, &recipient, &depositor_info)?;

    // Update global state
    state.total_tickets = state.total_tickets.add(Uint256::from(amount_tickets));
    state.total_deposits = state
        .total_deposits
        .add(Decimal256::from_uint256(deposit_amount));
    state.shares_supply = state.shares_supply.add(minted_amount);
    state.lottery_deposits = state
        .lottery_deposits
        .add(Decimal256::from_uint256(deposit_amount) * config.split_factor);
    store_state(&mut deps.storage, &state)?;

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.anchor_contract)?,
            send: vec![Coin {
                denom: config.stable_denom,
                amount,
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        })],
        log: vec![
            log("action", "gift_tickets"),
            log("gifter", env.message.sender),
            log("recipient", to),
            log("deposit_amount", deposit_amount),
            log("tickets", amount_tickets),
            log("shares_minted", minted_amount),
        ],
        data: None,
    })
}

// Make a donation deposit to the lottery pool
pub fn sponsor<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> HandleResult {
    let config = read_config(&deps.storage)?;
    //let mut state = read_state(&deps.storage)?;

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
            "Sponsorship amount must be greater than 0 {}",
            config.stable_denom
        )));
    }

    // TODO: store list of sponsors

    /*
    if deposit_amount != Decimal256::from_uint256(amount) {
        return Err(StdError::generic_err(format!(
            "Sponsorship amount must be equal to the selected amount: {} {}",
            amount, config.stable_denom
        )));
    }

    //TODO: add a time buffer here with block_time
    if state.next_lottery_time.is_expired(&env.block) {
        return Err(StdError::generic_err(
            "Current lottery is about to start, wait to sponsor when the next one begins",
        ));
    }
     */

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.anchor_contract)?,
            send: vec![Coin {
                denom: config.stable_denom,
                amount: deposit_amount.into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        })],
        log: vec![
            log("action", "sponsorship"),
            log("sponsor", env.message.sender),
            log("sponsorship_amount", deposit_amount),
        ],
        data: None,
    })
}

pub fn withdraw<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    amount: Option<u64>,      // amount of tickets
    sequence: Option<String>, // withdraw the ticket with this sequence
) -> HandleResult {
    let config = read_config(&deps.storage)?;
    let mut state = read_state(&deps.storage)?;

    let sender_raw = deps.api.canonical_address(&env.message.sender)?;
    let mut depositor: DepositorInfo = read_depositor_info(&deps.storage, &sender_raw);

    // TODO: check user does not send funds

    // If user does not specify an amount of tickets, we withdraw 1 ticket as default option
    let amount = amount.unwrap_or(1);

    if amount == 0 {
        return Err(StdError::generic_err(
            "Amount of tickets must be greater than zero",
        ));
    }

    if amount > depositor.tickets.len() as u64 {
        return Err(StdError::generic_err(format!(
            "User has {} tickets but {} tickets were requested to be withdrawn",
            depositor.tickets.len(),
            amount
        )));
    }

    let mut tickets = depositor.tickets.clone();
    let mut tickets_removed: Vec<String> = vec![];

    // TODO: test this logic
    if let Some(seq) = sequence {
        if let Some(index) = tickets.iter().position(|x| *x == seq) {
            tickets_removed.push(tickets.remove(index));
        } else {
            return Err(StdError::generic_err(format!(
                "It seems you don't have combination {}, so you can't withdraw it",
                seq,
            )));
        }
    } else {
        // Remove amount of tickets randomly from user's vector of sequences
        tickets_removed = tickets.drain(0..amount as usize).collect();
    }

    // Update depositor info with remaining tickets
    depositor.tickets = tickets;

    // Remove depositor's address from holders Sequence
    tickets_removed.iter().for_each(|seq| {
        let mut holders: Vec<CanonicalAddr> = read_sequence_info(&deps.storage, seq);
        let index = holders.iter().position(|x| *x == sender_raw).unwrap();
        holders.remove(index);
        sequence_bucket(&mut deps.storage)
            .save(seq.as_bytes(), &holders)
            .unwrap();
    });

    let unbonding_amount = config.ticket_prize * Decimal256::from_uint256(amount);

    // Place amount in unbonding state as a claim
    depositor.unbonding_info.push(Claim {
        amount: unbonding_amount,
        release_at: config.unbonding_period.after(&env.block),
    });

    // Withdraw from Anchor the proportional amount of total user deposits
    let unbonding_ratio: Decimal256 = unbonding_amount / depositor.deposit_amount;
    depositor.deposit_amount = depositor.deposit_amount.sub(unbonding_amount);

    // Calculate amount of pool shares to be redeemed
    let redeem_amount_shares = unbonding_ratio * depositor.shares;
    depositor.shares = depositor.shares.sub(redeem_amount_shares);

    store_depositor_info(&mut deps.storage, &sender_raw, &depositor)?;

    // Calculate fraction of shares to be redeemed out of the global pool
    // TODO: escape divide by zero error
    let withdraw_ratio = redeem_amount_shares / state.shares_supply;
    // Get contract's total balance of aUST
    let contract_a_balance = query_token_balance(
        deps,
        &deps.api.human_address(&config.a_terra_contract)?,
        &deps.api.human_address(&config.contract_addr)?,
    )?;

    // Calculate amount of aUST to be redeemed
    let redeem_amount = withdraw_ratio * contract_a_balance;

    // Update global state
    state.total_tickets = state.total_tickets.sub(Uint256::from(amount));
    state.shares_supply = state.shares_supply.sub(redeem_amount_shares);
    state.total_deposits = state.total_deposits.sub(unbonding_amount);
    state.lottery_deposits = state
        .lottery_deposits
        .sub(unbonding_amount * config.split_factor); // feels unnecessary
    store_state(&mut deps.storage, &state)?;

    // Message for redeem amount operation of aUST
    let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps.api.human_address(&config.a_terra_contract)?,
        send: vec![],
        msg: to_binary(&Cw20HandleMsg::Send {
            contract: deps.api.human_address(&config.anchor_contract)?,
            amount: redeem_amount.into(),
            msg: Some(to_binary(&Cw20HookMsg::RedeemStable {}).unwrap()),
        })?,
    });

    Ok(HandleResponse {
        messages: vec![redeem_msg],
        log: vec![
            log("action", "withdraw_ticket"),
            log("depositor", env.message.sender),
            log("tickets_amount", amount),
            log("redeem_amount_anchor", redeem_amount),
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
    if (amount.is_some()) && (amount.unwrap().is_zero()) {
        return Err(StdError::generic_err(
            "Claim amount must be greater than zero",
        ));
    }

    let config = read_config(&deps.storage)?;

    let sender_raw = deps.api.canonical_address(&env.message.sender)?;
    let mut to_send = claim_deposits(&mut deps.storage, &sender_raw, &env.block, amount)?;

    //TODO: doing two consecutive reads here, need to refactor
    let mut depositor: DepositorInfo = read_depositor_info(&deps.storage, &sender_raw);
    to_send += depositor.redeemable_amount;

    // Deduct taxes on the claim
    let net_coin_amount = deduct_tax(deps, coin(to_send.into(), "uusd"))?;
    let net_send = net_coin_amount.amount;

    // TODO: add check for when the amount requested is greater than to_send
    if net_send == Uint128(0) {
        return Err(StdError::generic_err(
            "Depositor does not have any amount to claim",
        ));
    }
    // Double-check if there is enough balance to send in the contract
    let balance = query_balance(
        deps,
        &deps.api.human_address(&config.contract_addr)?,
        String::from("uusd"),
    )?;

    if net_send > balance.into() {
        return Err(StdError::generic_err("Not enough funds to pay the claim"));
    }

    // TODO: add logic when claim amount is less than redeemable_amount

    depositor.redeemable_amount = Uint128::zero();
    store_depositor_info(&mut deps.storage, &sender_raw, &depositor)?;

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Bank(BankMsg::Send {
            from_address: env.clone().contract.address,
            to_address: env.clone().message.sender,
            amount: vec![Coin {
                denom: config.stable_denom,
                amount: net_send,
            }],
        })],
        log: vec![
            log("action", "claim"),
            log("depositor", env.message.sender),
            log("redeemed_amount", net_send),
            log("redeemable_amount_left", depositor.redeemable_amount),
        ],
        data: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: Option<HumanAddr>,
    lottery_interval: Option<u64>,
    block_time: Option<u64>,
    ticket_price: Option<Decimal256>,
    prize_distribution: Option<Vec<Decimal256>>,
    reserve_factor: Option<Decimal256>,
    split_factor: Option<Decimal256>,
    unbonding_period: Option<u64>,
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

    if let Some(lottery_interval) = lottery_interval {
        config.lottery_interval = Duration::Time(lottery_interval);
    }

    if let Some(block_time) = block_time {
        config.block_time = Duration::Time(block_time);
    }

    if let Some(ticket_prize) = ticket_price {
        config.ticket_prize = ticket_prize;
    }

    if let Some(prize_distribution) = prize_distribution {
        config.prize_distribution = prize_distribution;
    }

    if let Some(reserve_factor) = reserve_factor {
        config.reserve_factor = reserve_factor;
    }

    if let Some(split_factor) = split_factor {
        config.split_factor = split_factor;
    }

    if let Some(unbonding_period) = unbonding_period {
        config.unbonding_period = Duration::Time(unbonding_period);
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
        QueryMsg::LotteryInfo { lottery_id } => to_binary(&query_lottery_info(deps, lottery_id)?),
        QueryMsg::Depositor { address } => to_binary(&query_depositor(deps, address)?),
        QueryMsg::Depositors { start_after, limit } => {
            to_binary(&query_depositors(deps, start_after, limit)?)
        }
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
        lottery_interval: config.lottery_interval,
        block_time: config.block_time,
        ticket_prize: config.ticket_prize,
        prize_distribution: config.prize_distribution,
        reserve_factor: config.reserve_factor,
        split_factor: config.split_factor,
        unbonding_period: config.unbonding_period,
    })
}

pub fn query_state<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    _block_height: Option<u64>,
) -> StdResult<StateResponse> {
    let state: State = read_state(&deps.storage)?;

    //TODO: add block_height logic

    Ok(StateResponse {
        total_tickets: state.total_tickets,
        total_reserve: state.total_reserve,
        total_deposits: state.total_deposits,
        lottery_deposits: state.lottery_deposits,
        shares_supply: state.shares_supply,
        award_available: state.award_available,
        spendable_balance: state.spendable_balance,
        current_balance: state.current_balance,
        current_lottery: state.current_lottery,
        next_lottery_time: state.next_lottery_time,
    })
}

pub fn query_lottery_info<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    lottery_id: Option<u64>,
) -> StdResult<LotteryInfoResponse> {
    if let Some(id) = lottery_id {
        let lottery = read_lottery_info(&deps.storage, id);
        Ok(LotteryInfoResponse {
            lottery_id: id,
            sequence: lottery.sequence,
            awarded: lottery.awarded,
            total_prizes: lottery.total_prizes,
            winners: lottery
                .winners
                .into_iter()
                .map(|w| {
                    (
                        w.0,
                        w.1.into_iter()
                            .map(|addr| deps.api.human_address(&addr).unwrap())
                            .collect(),
                    )
                })
                .collect(),
        })
    } else {
        let current_lottery = read_state(&deps.storage)?.current_lottery;
        let lottery = read_lottery_info(&deps.storage, current_lottery);
        Ok(LotteryInfoResponse {
            lottery_id: current_lottery,
            sequence: lottery.sequence,
            awarded: lottery.awarded,
            total_prizes: lottery.total_prizes,
            winners: lottery
                .winners
                .into_iter()
                .map(|w| {
                    (
                        w.0,
                        w.1.into_iter()
                            .map(|addr| deps.api.human_address(&addr).unwrap())
                            .collect(),
                    )
                })
                .collect(), // transform CanonicalAddr to HumanAddr
        })
    }
    // TODO: return also winners -> transform Canonical to HumanAddr
}

pub fn query_depositor<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    addr: HumanAddr,
) -> StdResult<DepositorInfoResponse> {
    let address_raw = deps.api.canonical_address(&addr)?;
    let depositor = read_depositor_info(&deps.storage, &address_raw);
    Ok(DepositorInfoResponse {
        depositor: addr,
        deposit_amount: depositor.deposit_amount,
        shares: depositor.shares,
        redeemable_amount: depositor.redeemable_amount,
        tickets: depositor.tickets,
        unbonding_info: depositor.unbonding_info,
    })
}

pub fn query_depositors<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    start_after: Option<HumanAddr>,
    limit: Option<u32>,
) -> StdResult<DepositorsInfoResponse> {
    let start_after = if let Some(start_after) = start_after {
        Some(deps.api.canonical_address(&start_after)?)
    } else {
        None
    };

    let depositors = read_depositors(deps, start_after, limit)?;
    Ok(DepositorsInfoResponse { depositors })
}

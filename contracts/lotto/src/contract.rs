#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use crate::error::ContractError;
use crate::helpers::{
    calculate_winner_prize, claim_deposits, compute_depositor_reward, compute_reward,
    is_valid_sequence,
};
use crate::prize_strategy::{execute_lottery, execute_prize};
use crate::querier::{query_balance, query_exchange_rate, query_glow_emission_rate};
use crate::state::{
    read_depositor_info, read_depositors, read_lottery_info, read_sponsor_info,
    store_depositor_info, store_sponsor_info, Config, DepositorInfo, Pool, PrizeInfo, SponsorInfo,
    State, CONFIG, POOL, PRIZES, STATE, TICKETS,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, coin, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo,
    Response, StdError, StdResult, Uint128, WasmMsg,
};
use cw0::{Duration, Expiration};
use cw20::Cw20ExecuteMsg;
use cw_storage_plus::U64Key;
use glow_protocol::distributor::ExecuteMsg as FaucetExecuteMsg;
use glow_protocol::lotto::{
    Claim, ConfigResponse, DepositorInfoResponse, DepositorsInfoResponse, ExecuteMsg,
    InstantiateMsg, LotteryInfoResponse, PoolResponse, PrizeInfoResponse, QueryMsg, StateResponse,
    TicketInfoResponse,
};
use glow_protocol::querier::deduct_tax;
use moneymarket::market::{Cw20HookMsg, EpochStateResponse, ExecuteMsg as AnchorMsg};
use std::ops::{Add, Sub};
use terraswap::querier::query_token_balance;

pub const INITIAL_DEPOSIT_AMOUNT: u128 = 100_000_000;
pub const SEQUENCE_DIGITS: u8 = 5;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let initial_deposit = info
        .funds
        .iter()
        .find(|c| c.denom == msg.stable_denom)
        .map(|c| c.amount)
        .unwrap_or_else(Uint128::zero);

    if initial_deposit != Uint128::from(INITIAL_DEPOSIT_AMOUNT) {
        return Err(ContractError::InvalidDepositInstantiation {});
    }

    CONFIG.save(
        deps.storage,
        &Config {
            owner: deps.api.addr_validate(msg.owner.as_str())?,
            a_terra_contract: deps.api.addr_validate(msg.aterra_contract.as_str())?,
            gov_contract: Addr::unchecked(""),
            distributor_contract: Addr::unchecked(""),
            oracle_contract: deps.api.addr_validate(msg.oracle_contract.as_str())?,
            stable_denom: msg.stable_denom.clone(),
            anchor_contract: deps.api.addr_validate(msg.anchor_contract.as_str())?,
            lottery_interval: Duration::Time(msg.lottery_interval),
            block_time: Duration::Time(msg.block_time),
            round_delta: msg.round_delta,
            ticket_price: msg.ticket_price,
            max_holders: msg.max_holders,
            prize_distribution: msg.prize_distribution,
            target_award: msg.target_award,
            reserve_factor: msg.reserve_factor,
            split_factor: msg.split_factor,
            instant_withdrawal_fee: msg.instant_withdrawal_fee,
            unbonding_period: Duration::Time(msg.unbonding_period),
        },
    )?;

    STATE.save(
        deps.storage,
        &State {
            total_tickets: Uint256::zero(),
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(initial_deposit),
            current_lottery: 0,
            next_lottery_time: Duration::Time(msg.lottery_interval).after(&env.block),
            next_lottery_exec_time: Duration::Time(msg.lottery_interval + msg.block_time)
                .after(&env.block),
            next_epoch: Duration::Time(msg.lottery_interval + msg.block_time).after(&env.block),
            last_reward_updated: 0,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: msg.initial_emission_rate,
        },
    )?;

    POOL.save(
        deps.storage,
        &Pool {
            total_deposits: Decimal256::zero(),
            total_sponsor_amount: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            deposit_shares: Decimal256::zero(),
            lottery_shares: Decimal256::zero(),
            sponsor_shares: Decimal256::zero(),
        },
    )?;

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
        ExecuteMsg::RegisterContracts {
            gov_contract,
            distributor_contract,
        } => register_contracts(deps, info, gov_contract, distributor_contract),
        ExecuteMsg::Deposit { combinations } => deposit(deps, env, info, combinations),
        ExecuteMsg::Gift {
            combinations,
            recipient,
        } => gift_tickets(deps, env, info, combinations, recipient),
        ExecuteMsg::Sponsor { award } => sponsor(deps, env, info, award),
        ExecuteMsg::SponsorWithdraw {} => sponsor_withdraw(deps, env, info),
        ExecuteMsg::Withdraw { amount, instant } => withdraw(deps, env, info, amount, instant),
        ExecuteMsg::Claim { lottery } => claim(deps, env, info, lottery),
        ExecuteMsg::ClaimRewards {} => claim_rewards(deps, env, info),
        ExecuteMsg::ExecuteLottery {} => execute_lottery(deps, env, info),
        ExecuteMsg::ExecutePrize { limit } => execute_prize(deps, env, info, limit),
        ExecuteMsg::ExecuteEpochOps {} => execute_epoch_ops(deps, env),
        ExecuteMsg::UpdateConfig {
            owner,
            oracle_addr,
            reserve_factor,
            instant_withdrawal_fee,
            unbonding_period,
        } => update_config(
            deps,
            info,
            owner,
            oracle_addr,
            reserve_factor,
            instant_withdrawal_fee,
            unbonding_period,
        ),
        ExecuteMsg::UpdateLotteryConfig {
            lottery_interval,
            block_time,
            ticket_price,
            prize_distribution,
            round_delta,
        } => update_lottery_config(
            deps,
            info,
            lottery_interval,
            block_time,
            ticket_price,
            prize_distribution,
            round_delta,
        ),
    }
}

pub fn register_contracts(
    deps: DepsMut,
    info: MessageInfo,
    gov_contract: String,
    distributor_contract: String,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;

    // check permission
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    // can't be registered twice
    if config.gov_contract != Addr::unchecked("")
        || config.distributor_contract != Addr::unchecked("")
    {
        return Err(ContractError::AlreadyRegistered {});
    }

    config.gov_contract = deps.api.addr_validate(&gov_contract)?;
    config.distributor_contract = deps.api.addr_validate(&distributor_contract)?;
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::default())
}

// Deposit UST and get pool shares and tickets in return
pub fn deposit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    combinations: Vec<String>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;

    // Check deposit is in base stable denom
    let deposit_amount = info
        .funds
        .iter()
        .find(|c| c.denom == config.stable_denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    if deposit_amount.is_zero() {
        return Err(ContractError::InvalidDepositAmount {});
    }

    let amount_tickets = combinations.len() as u64;
    let required_amount = config.ticket_price * Uint256::from(amount_tickets);
    if deposit_amount < required_amount {
        return Err(ContractError::InsufficientDepositAmount(amount_tickets));
    }

    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    let depositor = info.sender.clone();
    let mut depositor_info: DepositorInfo = read_depositor_info(deps.storage, &depositor);

    for combination in combinations.clone() {
        if !is_valid_sequence(&combination, SEQUENCE_DIGITS) {
            return Err(ContractError::InvalidSequence {});
        }
    }

    let mut new_combinations = combinations.clone();
    // check if we need to round up number of combinations based on depositor total deposits
    if ((depositor_info.deposit_amount + Decimal256::from_uint256(deposit_amount))
        / config.ticket_price)
        >= (Decimal256::from_uint256(Uint256::from(
            (depositor_info.tickets.len() + combinations.len()) as u128,
        )) + Decimal256::one())
    {
        let current_time = env.block.time.nanos();
        let sequence = &current_time.to_string()[current_time.to_string().len() - 5..];

        new_combinations.push(sequence.to_string());
    }

    compute_reward(&mut state, &pool, env.block.height);
    compute_depositor_reward(&state, &mut depositor_info);

    // query exchange_rate from anchor money market
    let epoch_state: EpochStateResponse =
        query_exchange_rate(deps.as_ref(), config.anchor_contract.to_string())?;

    // Discount tx taxes
    let net_coin_amount = deduct_tax(deps.as_ref(), coin(deposit_amount.into(), "uusd"))?;
    let amount = net_coin_amount.amount;

    // add amount of aUST entitled from the deposit
    let minted_amount = Decimal256::from_uint256(amount) / epoch_state.exchange_rate;

    // We are storing the deposit amount without the tax deduction, so we subsidy it for UX reasons.
    depositor_info.deposit_amount = depositor_info
        .deposit_amount
        .add(Decimal256::from_uint256(deposit_amount));
    depositor_info.shares = depositor_info.shares.add(minted_amount);

    for combination in new_combinations {
        if let Some(holders) = TICKETS
            .may_load(deps.storage, combination.as_bytes())
            .unwrap()
        {
            if holders.len() >= config.max_holders as usize {
                return Err(ContractError::InvalidHolderSequence {});
            }
        }

        let add_ticket = |a: Option<Vec<Addr>>| -> StdResult<Vec<Addr>> {
            let mut b = a.unwrap_or_default();
            b.push(depositor.clone());
            Ok(b)
        };
        TICKETS
            .update(deps.storage, combination.as_bytes(), add_ticket)
            .unwrap();
        depositor_info.tickets.push(combination);
    }

    // Update global state and pool
    state.total_tickets = state.total_tickets.add(Uint256::from(amount_tickets));
    pool.lottery_shares = pool.lottery_shares.add(minted_amount * config.split_factor);
    pool.deposit_shares = pool
        .deposit_shares
        .add(minted_amount - minted_amount * config.split_factor);
    pool.total_deposits = pool
        .total_deposits
        .add(Decimal256::from_uint256(deposit_amount));
    pool.lottery_deposits = pool
        .lottery_deposits
        .add(Decimal256::from_uint256(deposit_amount) * config.split_factor);

    // Update depositor and state information
    store_depositor_info(deps.storage, &depositor, &depositor_info)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.anchor_contract.to_string(),
            funds: vec![Coin {
                denom: config.stable_denom,
                amount,
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        })])
        .add_attributes(vec![
            attr("action", "batch_deposit"),
            attr("depositor", info.sender.to_string()),
            attr("deposit_amount", deposit_amount.to_string()),
            attr("shares_minted", minted_amount.to_string()),
        ]))
}

// Gift several tickets at once to a given address
pub fn gift_tickets(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    combinations: Vec<String>,
    to: String,
) -> Result<Response, ContractError> {
    if to == info.sender {
        return Err(ContractError::InvalidGift {});
    }

    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;

    // Check deposit is in base stable denom
    let deposit_amount = info
        .funds
        .iter()
        .find(|c| c.denom == config.stable_denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    if deposit_amount.is_zero() {
        return Err(ContractError::InvalidGiftAmount {});
    }

    let amount_tickets = combinations.len() as u64;
    let required_amount = config.ticket_price * Uint256::from(amount_tickets);
    if deposit_amount != required_amount {
        return Err(ContractError::InsufficientGiftDepositAmount(amount_tickets));
    }

    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    for combination in combinations.clone() {
        if !is_valid_sequence(&combination, SEQUENCE_DIGITS) {
            return Err(ContractError::InvalidSequence {});
        }
    }

    let recipient = deps.api.addr_validate(to.as_str())?;
    let mut depositor_info: DepositorInfo = read_depositor_info(deps.storage, &recipient);

    let mut new_combinations = combinations.clone();
    // check if we need to round up number of combinations based on depositor total deposits
    if ((depositor_info.deposit_amount + Decimal256::from_uint256(deposit_amount))
        / config.ticket_price)
        >= (Decimal256::from_uint256(Uint256::from(
            (depositor_info.tickets.len() + combinations.len()) as u128,
        )) + Decimal256::one())
    {
        let current_time = env.block.time.nanos();
        let sequence = &current_time.to_string()[current_time.to_string().len() - 5..];

        new_combinations.push(sequence.to_string());
    }

    // Compute Glow rewards of recipient
    compute_reward(&mut state, &pool, env.block.height);
    compute_depositor_reward(&state, &mut depositor_info);

    // query exchange_rate from anchor money market
    let epoch_state: EpochStateResponse =
        query_exchange_rate(deps.as_ref(), config.anchor_contract.to_string())?;

    // Discount tx taxes
    let net_coin_amount = deduct_tax(deps.as_ref(), coin(deposit_amount.into(), "uusd"))?;
    let amount = net_coin_amount.amount;

    // add amount of aUST entitled from the deposit
    let minted_amount = Decimal256::from_uint256(amount) / epoch_state.exchange_rate;
    depositor_info.deposit_amount = depositor_info
        .deposit_amount
        .add(Decimal256::from_uint256(deposit_amount));
    depositor_info.shares = depositor_info.shares.add(minted_amount);

    for combination in combinations.clone() {
        if let Some(holders) = TICKETS
            .may_load(deps.storage, combination.as_bytes())
            .unwrap()
        {
            if holders.len() >= config.max_holders as usize {
                return Err(ContractError::InvalidHolderSequence {});
            }
        }

        let add_ticket = |a: Option<Vec<Addr>>| -> StdResult<Vec<Addr>> {
            let mut b = a.unwrap_or_default();
            b.push(recipient.clone());
            Ok(b)
        };

        TICKETS
            .update(deps.storage, combination.as_bytes(), add_ticket)
            .unwrap();

        depositor_info.tickets.push(combination);
    }

    // Update global state and pool
    state.total_tickets = state.total_tickets.add(Uint256::from(amount_tickets));
    pool.lottery_shares = pool.lottery_shares.add(minted_amount * config.split_factor);
    pool.deposit_shares = pool
        .deposit_shares
        .add(minted_amount - minted_amount * config.split_factor);
    pool.lottery_deposits = pool
        .lottery_deposits
        .add(Decimal256::from_uint256(deposit_amount) * config.split_factor);

    pool.total_deposits = pool
        .total_deposits
        .add(Decimal256::from_uint256(deposit_amount));

    // Update depositor and state information
    store_depositor_info(deps.storage, &recipient, &depositor_info)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    Ok(Response::new()
        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.anchor_contract.to_string(),
            funds: vec![Coin {
                denom: config.stable_denom,
                amount,
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        }))
        .add_attributes(vec![
            attr("action", "gift_tickets"),
            attr("gifter", info.sender.to_string()),
            attr("recipient", to),
            attr("deposit_amount", deposit_amount.to_string()),
            attr("tickets", amount_tickets.to_string()),
            attr("shares_minted", minted_amount.to_string()),
        ]))
}

// Make a donation deposit to the lottery pool
pub fn sponsor(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    award: Option<bool>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;

    // Check deposit is in base stable denom
    let sponsor_amount = info
        .funds
        .iter()
        .find(|c| c.denom == config.stable_denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    if sponsor_amount.is_zero() {
        return Err(ContractError::InvalidSponsorshipAmount {});
    }

    compute_reward(&mut state, &pool, env.block.height);

    let mut messages: Vec<CosmosMsg> = vec![];

    if let Some(true) = award {
        state.award_available = state
            .award_available
            .add(Decimal256::from_uint256(sponsor_amount));
    } else {
        // query exchange_rate from anchor money market
        let epoch_state: EpochStateResponse =
            query_exchange_rate(deps.as_ref(), config.anchor_contract.to_string())?;

        // Discount tx taxes
        let net_coin_amount = deduct_tax(deps.as_ref(), coin(sponsor_amount.into(), "uusd"))?;
        let net_sponsor_amount = net_coin_amount.amount;

        // add amount of aUST entitled from the deposit
        let minted_amount =
            Decimal256::from_uint256(net_sponsor_amount) / epoch_state.exchange_rate;

        // fetch sponsor_info
        let mut sponsor_info: SponsorInfo = read_sponsor_info(deps.storage, &info.sender);
        // add sponsor_amount to depositor
        sponsor_info.amount = sponsor_info
            .amount
            .add(Decimal256::from_uint256(net_sponsor_amount));
        sponsor_info.shares = sponsor_info.shares.add(minted_amount);
        store_sponsor_info(deps.storage, &info.sender, &sponsor_info)?;

        // update pool
        pool.total_sponsor_amount = pool
            .total_sponsor_amount
            .add(Decimal256::from_uint256(net_sponsor_amount));
        pool.sponsor_shares = pool.sponsor_shares.add(minted_amount);
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.anchor_contract.to_string(),
            funds: vec![Coin {
                denom: config.stable_denom,
                amount: net_sponsor_amount,
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        }));
    }

    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("action", "sponsorship"),
        attr("sponsor", info.sender.to_string()),
        attr("sponsorship_amount", sponsor_amount),
    ]))
}

pub fn sponsor_withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;

    let mut sponsor_info: SponsorInfo = read_sponsor_info(deps.storage, &info.sender);

    if sponsor_info.amount.is_zero() || pool.sponsor_shares.is_zero() {
        return Err(ContractError::InvalidSponsorWithdraw {});
    }

    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    compute_reward(&mut state, &pool, env.block.height);

    // Calculate aust amount to redeem based on depositor amount
    let contract_a_balance = query_token_balance(
        &deps.querier,
        config.a_terra_contract.clone(),
        env.contract.address,
    )?;
    let rate =
        query_exchange_rate(deps.as_ref(), config.anchor_contract.to_string())?.exchange_rate;
    let aust_to_redeem = sponsor_info.amount / rate;

    // Double-checking Lotto pool is solvent against sponsors
    if Decimal256::from_uint256(Uint256::from(contract_a_balance)) * rate
        < (pool.total_deposits + pool.total_sponsor_amount)
    {
        return Err(ContractError::InsufficientSponsorFunds {});
    }

    // Update global state
    pool.total_sponsor_amount = pool.total_sponsor_amount.sub(sponsor_info.amount);
    pool.sponsor_shares = pool.sponsor_shares.sub(sponsor_info.shares);

    // Update sponsor info
    sponsor_info.amount = Decimal256::zero();
    sponsor_info.shares = Decimal256::zero();

    let mut msgs: Vec<CosmosMsg> = vec![];

    // Message for redeem amount operation of aUST
    let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.a_terra_contract.to_string(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Send {
            contract: config.anchor_contract.to_string(),
            amount: (aust_to_redeem * Uint256::one()).into(),
            msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
        })?,
    });
    msgs.push(redeem_msg);

    // Discount tx taxes
    let net_coin_amount = deduct_tax(
        deps.as_ref(),
        coin((sponsor_info.amount * Uint256::one()).into(), "uusd"),
    )?;

    msgs.push(CosmosMsg::Bank(BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: vec![net_coin_amount],
    }));

    store_sponsor_info(deps.storage, &info.sender, &sponsor_info)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "withdraw_sponsor"),
        attr("depositor", info.sender.to_string()),
        attr("redeem_amount_anchor", aust_to_redeem.to_string()),
        attr("redeem_stable_amount", sponsor_info.amount.to_string()),
    ]))
}

pub fn withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: Option<Uint128>,
    instant: Option<bool>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;

    let shares_supply = pool.lottery_shares + pool.deposit_shares + pool.sponsor_shares;

    let mut depositor: DepositorInfo = read_depositor_info(deps.storage, &info.sender);
    if depositor.shares.is_zero() || shares_supply.is_zero() {
        return Err(ContractError::InvalidWithdraw {});
    }

    if (amount.is_some()) && (amount.unwrap().is_zero()) {
        return Err(ContractError::InvalidWithdraw {});
    }

    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }
    // Compute GLOW reward
    compute_reward(&mut state, &pool, env.block.height);
    compute_depositor_reward(&state, &mut depositor);

    // Calculate depositor current pooled deposits in uusd
    let depositor_ratio = depositor.shares / shares_supply;
    let contract_a_balance = query_token_balance(
        &deps.querier,
        config.a_terra_contract.clone(),
        env.clone().contract.address,
    )?;
    let aust_amount = depositor_ratio * Decimal256::from_uint256(contract_a_balance);
    let rate =
        query_exchange_rate(deps.as_ref(), config.anchor_contract.to_string())?.exchange_rate;
    let pooled_deposits = Uint256::one() * (aust_amount * rate);

    // Calculate ratio of deposits, shares and tickets to withdraw
    let mut withdraw_ratio = Decimal256::one();
    if let Some(amount) = amount {
        if Uint256::from(amount) > pooled_deposits {
            return Err(ContractError::InvalidWithdraw {});
        } else {
            withdraw_ratio = Decimal256::from_ratio(Uint256::from(amount), pooled_deposits);
        }
    }
    let aust_to_redeem = aust_amount * withdraw_ratio;
    let mut return_amount = pooled_deposits * withdraw_ratio;

    // Double-checking Lotto pool is solvent against deposits
    if Decimal256::from_uint256(Uint256::from(contract_a_balance)) * rate
        < (pool.total_deposits + pool.total_sponsor_amount)
    {
        return Err(ContractError::InsufficientPoolFunds {});
    }

    let tickets_amount = depositor.tickets.len() as u128;
    // Check for rounding error
    let rounded_tickets = Uint256::from(tickets_amount) * withdraw_ratio;
    let decimal_tickets = Decimal256::from_uint256(Uint256::from(tickets_amount)) * withdraw_ratio;

    let mut withdrawn_tickets: u128 = rounded_tickets.into();
    if decimal_tickets != Decimal256::from_uint256(rounded_tickets) {
        withdrawn_tickets += 1u128;
    }

    if withdrawn_tickets > tickets_amount {
        return Err(ContractError::InvalidWithdraw {});
    }

    for seq in depositor.tickets.drain(..withdrawn_tickets as usize) {
        TICKETS.update(deps.storage, seq.as_bytes(), |tickets| -> StdResult<_> {
            let mut new_tickets = tickets.unwrap();
            let index = new_tickets
                .iter()
                .position(|x| *x == info.sender.clone())
                .unwrap();
            let _elem = new_tickets.remove(index);
            Ok(new_tickets)
        })?;
    }

    let withdrawn_deposits = depositor.deposit_amount * withdraw_ratio;
    let withdrawn_shares = depositor.shares * withdraw_ratio;
    let withdrawn_lottery_shares = withdrawn_shares * config.split_factor;
    let withdrawn_deposit_shares = withdrawn_shares - withdrawn_shares * config.split_factor;

    // Update depositor info
    depositor.deposit_amount = depositor.deposit_amount.sub(withdrawn_deposits);
    depositor.shares = depositor.shares.sub(withdrawn_shares);

    // Update global state and pool
    state.total_tickets = state.total_tickets.sub(Uint256::from(withdrawn_tickets));
    pool.total_deposits = pool.total_deposits.sub(withdrawn_deposits);
    pool.lottery_deposits = pool
        .lottery_deposits
        .sub(withdrawn_deposits * config.split_factor);
    pool.lottery_shares = pool.lottery_shares.sub(withdrawn_lottery_shares);
    pool.deposit_shares = pool.deposit_shares.sub(withdrawn_deposit_shares);

    let mut msgs: Vec<CosmosMsg> = vec![];

    // Message for redeem amount operation of aUST
    let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.a_terra_contract.to_string(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Send {
            contract: config.anchor_contract.to_string(),
            amount: (aust_to_redeem * Uint256::one()).into(),
            msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
        })?,
    });
    msgs.push(redeem_msg);

    // Instant withdrawal. The user incurs a fee and receive the funds with this operation
    let mut withdrawal_fee = Uint256::zero();

    if let Some(true) = instant {
        // Apply instant withdrawal fee
        withdrawal_fee = return_amount * config.instant_withdrawal_fee;
        return_amount = return_amount.sub(withdrawal_fee);
        // Discount tx taxes
        let net_coin_amount = deduct_tax(deps.as_ref(), coin(return_amount.into(), "uusd"))?;

        msgs.push(CosmosMsg::Bank(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![net_coin_amount],
        }));
    } else {
        // Discount tx taxes
        let net_coin_amount = deduct_tax(deps.as_ref(), coin(return_amount.into(), "uusd"))?;
        // Place amount in unbonding state as a claim
        depositor.unbonding_info.push(Claim {
            amount: Decimal256::from_uint256(Uint256::from(net_coin_amount.amount)),
            release_at: config.unbonding_period.after(&env.block),
        });
    }

    store_depositor_info(deps.storage, &info.sender, &depositor)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "withdraw_ticket"),
        attr("depositor", info.sender.to_string()),
        attr("tickets_amount", withdrawn_tickets.to_string()),
        attr("redeem_amount_anchor", aust_to_redeem.to_string()),
        attr("redeem_stable_amount", return_amount.to_string()),
        attr("instant_withdrawal_fee", withdrawal_fee.to_string()),
    ]))
}

// Send available UST to user from current redeemable balance and unbonded deposits
pub fn claim(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    lottery: Option<u64>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    let mut to_send = claim_deposits(deps.storage, &info.sender, &env.block, None)?;
    let mut depositor: DepositorInfo = read_depositor_info(deps.as_ref().storage, &info.sender);

    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    // Compute Glow depositor rewards
    compute_reward(&mut state, &pool, env.block.height);
    compute_depositor_reward(&state, &mut depositor);

    if let Some(lottery_id) = lottery {
        let lottery = read_lottery_info(deps.storage, lottery_id);
        if !lottery.awarded {
            return Err(ContractError::InsufficientClaimableFunds {});
        }
        //Calculate and add to to_send
        let lottery_key: U64Key = U64Key::from(lottery_id);
        let prizes = PRIZES
            .may_load(deps.storage, (&info.sender, lottery_key.clone()))
            .unwrap();
        if let Some(prize) = prizes {
            if prize.claimed {
                return Err(ContractError::InvalidLotteryClaim {});
            }

            to_send += calculate_winner_prize(
                lottery.total_prizes,
                prize.matches,
                lottery.number_winners,
                config.prize_distribution,
            );

            PRIZES.save(
                deps.storage,
                (&info.sender, lottery_key),
                &PrizeInfo {
                    claimed: true,
                    matches: prize.matches,
                },
            )?;
        }
    }

    if to_send == Uint128::zero() {
        return Err(ContractError::InsufficientClaimableFunds {});
    }

    // Deduct taxes on the claim
    let net_coin_amount = deduct_tax(deps.as_ref(), coin(to_send.into(), "uusd"))?;
    let net_send = net_coin_amount.amount;

    // Double-check if there is enough balance to send in the contract
    let balance = query_balance(
        deps.as_ref(),
        env.contract.address.to_string(),
        String::from("uusd"),
    )?;

    if net_send > balance.into() {
        return Err(ContractError::InsufficientFunds {});
    }

    store_depositor_info(deps.storage, &info.sender, &depositor)?;
    STATE.save(deps.storage, &state)?;

    Ok(Response::new()
        .add_message(CosmosMsg::Bank(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![Coin {
                denom: config.stable_denom,
                amount: net_send,
            }],
        }))
        .add_attributes(vec![
            attr("action", "claim"),
            attr("depositor", info.sender.to_string()),
            attr("redeemed_amount", net_send),
        ]))
}

pub fn execute_epoch_ops(deps: DepsMut, env: Env) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    if !state.next_epoch.is_expired(&env.block) {
        return Err(ContractError::InvalidEpochExecution {});
    }

    // Compute global Glow rewards
    compute_reward(&mut state, &pool, env.block.height);

    // Query updated Glow emission rate and update state
    state.glow_emission_rate = query_glow_emission_rate(
        &deps.querier,
        config.distributor_contract,
        state.award_available,
        config.target_award,
        state.glow_emission_rate,
    )?
    .emission_rate;

    // Compute total_reserves to fund gov contract
    let total_reserves = state.total_reserve * Uint256::one();
    let messages: Vec<CosmosMsg> = if !total_reserves.is_zero() {
        vec![CosmosMsg::Bank(BankMsg::Send {
            to_address: config.gov_contract.to_string(),
            amount: vec![deduct_tax(
                deps.as_ref(),
                Coin {
                    denom: config.stable_denom,
                    amount: total_reserves.into(),
                },
            )?],
        })]
    } else {
        vec![]
    };

    // Empty total reserve and store state
    state.next_epoch = Expiration::AtTime(env.block.time).add(config.lottery_interval)?;
    state.total_reserve = Decimal256::zero();
    STATE.save(deps.storage, &state)?;

    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("action", "execute_epoch_operations"),
        attr("total_reserves", total_reserves.to_string()),
        attr("glow_emission_rate", state.glow_emission_rate.to_string()),
    ]))
}

pub fn claim_rewards(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    let depositor_address = info.sender.as_str();
    let mut depositor: DepositorInfo = read_depositor_info(deps.storage, &info.sender);

    // Compute Glow depositor rewards
    compute_reward(&mut state, &pool, env.block.height);
    compute_depositor_reward(&state, &mut depositor);

    let claim_amount = depositor.pending_rewards * Uint256::one();
    depositor.pending_rewards = Decimal256::zero();

    STATE.save(deps.storage, &state)?;
    store_depositor_info(deps.storage, &info.sender, &depositor)?;

    let messages: Vec<CosmosMsg> = if !claim_amount.is_zero() {
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.distributor_contract.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: depositor_address.to_string(),
                amount: claim_amount.into(),
            })?,
        })]
    } else {
        vec![]
    };

    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("action", "claim_rewards"),
        attr("claim_amount", claim_amount),
    ]))
}

#[allow(clippy::too_many_arguments)]
pub fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    oracle_addr: Option<String>,
    reserve_factor: Option<Decimal256>,
    instant_withdrawal_fee: Option<Decimal256>,
    unbonding_period: Option<u64>,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;

    // check permission
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }
    // change owner of Glow lotto contract
    if let Some(owner) = owner {
        config.owner = deps.api.addr_validate(owner.as_str())?;
    }

    // change oracle contract addr
    if let Some(oracle_addr) = oracle_addr {
        config.owner = deps.api.addr_validate(oracle_addr.as_str())?;
    }

    if let Some(reserve_factor) = reserve_factor {
        if reserve_factor > Decimal256::one() {
            return Err(ContractError::InvalidReserveFactor {});
        }

        config.reserve_factor = reserve_factor;
    }

    if let Some(instant_withdrawal_fee) = instant_withdrawal_fee {
        if instant_withdrawal_fee > Decimal256::one() {
            return Err(ContractError::InvalidSplitFactor {});
        }
    }

    if let Some(unbonding_period) = unbonding_period {
        config.block_time = Duration::Time(unbonding_period);
    }

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![("action", "update_config")]))
}

pub fn update_lottery_config(
    deps: DepsMut,
    info: MessageInfo,
    lottery_interval: Option<u64>,
    block_time: Option<u64>,
    ticket_price: Option<Decimal256>,
    prize_distribution: Option<[Decimal256; 6]>,
    round_delta: Option<u64>,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;

    // check permission
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    if let Some(lottery_interval) = lottery_interval {
        config.lottery_interval = Duration::Time(lottery_interval);
    }

    if let Some(block_time) = block_time {
        config.block_time = Duration::Time(block_time);
    }

    if let Some(round_delta) = round_delta {
        config.round_delta = round_delta;
    }

    if let Some(ticket_price) = ticket_price {
        config.ticket_price = ticket_price;
    }

    if let Some(prize_distribution) = prize_distribution {
        if prize_distribution.len() != 5 {
            return Err(ContractError::InvalidPrizeDistribution {});
        }

        let mut sum = Decimal256::zero();
        for item in prize_distribution.iter() {
            sum += *item;
        }

        if sum != Decimal256::one() {
            return Err(ContractError::InvalidPrizeDistribution {});
        }

        config.prize_distribution = prize_distribution;
    }

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![("action", "update_lottery_config")]))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::State { block_height } => to_binary(&query_state(deps, env, block_height)?),
        QueryMsg::Pool {} => to_binary(&query_pool(deps)?),
        QueryMsg::LotteryInfo { lottery_id } => {
            to_binary(&query_lottery_info(deps, env, lottery_id)?)
        }
        QueryMsg::TicketInfo { sequence } => to_binary(&query_ticket_info(deps, sequence)?),
        QueryMsg::PrizeInfo {
            address,
            lottery_id,
        } => to_binary(&query_prizes(deps, address, lottery_id)?),
        QueryMsg::Depositor { address } => to_binary(&query_depositor(deps, address)?),
        QueryMsg::Depositors { start_after, limit } => {
            to_binary(&query_depositors(deps, start_after, limit)?)
        }
    }
}

pub fn query_ticket_info(deps: Deps, ticket: String) -> StdResult<TicketInfoResponse> {
    let holders = TICKETS
        .may_load(deps.storage, ticket.as_ref())?
        .unwrap_or_default();
    Ok(TicketInfoResponse { holders })
}

pub fn query_prizes(deps: Deps, address: String, lottery_id: u64) -> StdResult<PrizeInfoResponse> {
    let lottery_key = U64Key::from(lottery_id);
    let addr = deps.api.addr_validate(&address)?;
    let prize_info = PRIZES
        .may_load(deps.storage, (&addr, lottery_key))?
        .unwrap_or_default();

    Ok(PrizeInfoResponse {
        holder: addr,
        lottery_id,
        claimed: prize_info.claimed,
        matches: prize_info.matches,
    })
}

pub fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;

    Ok(ConfigResponse {
        owner: config.owner.to_string(),
        stable_denom: config.stable_denom,
        a_terra_contract: config.a_terra_contract.to_string(),
        anchor_contract: config.anchor_contract.to_string(),
        gov_contract: config.gov_contract.to_string(),
        distributor_contract: config.distributor_contract.to_string(),
        lottery_interval: config.lottery_interval,
        block_time: config.block_time,
        ticket_price: config.ticket_price,
        max_holders: config.max_holders,
        prize_distribution: config.prize_distribution,
        target_award: config.target_award,
        reserve_factor: config.reserve_factor,
        split_factor: config.split_factor,
        instant_withdrawal_fee: config.instant_withdrawal_fee,
        unbonding_period: config.unbonding_period,
    })
}

pub fn query_state(deps: Deps, env: Env, block_height: Option<u64>) -> StdResult<StateResponse> {
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    let block_height = if let Some(block_height) = block_height {
        block_height
    } else {
        env.block.height
    };

    if block_height < state.last_reward_updated {
        return Err(StdError::generic_err(
            "Block_height must be greater than last_reward_updated",
        ));
    }

    // Compute reward rate with given block height
    compute_reward(&mut state, &pool, block_height);

    Ok(StateResponse {
        total_tickets: state.total_tickets,
        total_reserve: state.total_reserve,
        award_available: state.award_available,
        current_lottery: state.current_lottery,
        next_lottery_time: state.next_lottery_time,
        next_lottery_exec_time: state.next_lottery_exec_time,
        next_epoch: state.next_epoch,
        last_reward_updated: state.last_reward_updated,
        global_reward_index: state.global_reward_index,
        glow_emission_rate: state.glow_emission_rate,
    })
}

pub fn query_pool(deps: Deps) -> StdResult<PoolResponse> {
    let pool = POOL.load(deps.storage)?;

    Ok(PoolResponse {
        total_deposits: pool.total_deposits,
        total_sponsor_amount: pool.total_sponsor_amount,
        lottery_deposits: pool.lottery_deposits,
        deposit_shares: pool.deposit_shares,
        lottery_shares: pool.lottery_shares,
        sponsor_shares: pool.sponsor_shares,
    })
}

pub fn query_lottery_info(
    deps: Deps,
    env: Env,
    lottery_id: Option<u64>,
) -> StdResult<LotteryInfoResponse> {
    if let Some(id) = lottery_id {
        let lottery = read_lottery_info(deps.storage, id);
        Ok(LotteryInfoResponse {
            lottery_id: id,
            sequence: lottery.sequence,
            awarded: lottery.awarded,
            total_prizes: lottery.total_prizes,
            number_winners: lottery.number_winners,
        })
    } else {
        let current_lottery = query_state(deps, env, None)?.current_lottery;
        let lottery = read_lottery_info(deps.storage, current_lottery);
        Ok(LotteryInfoResponse {
            lottery_id: current_lottery,
            sequence: lottery.sequence,
            awarded: lottery.awarded,
            total_prizes: lottery.total_prizes,
            number_winners: lottery.number_winners,
        })
    }
}

pub fn query_depositor(deps: Deps, addr: String) -> StdResult<DepositorInfoResponse> {
    let address = deps.api.addr_validate(&addr)?;
    let depositor = read_depositor_info(deps.storage, &address);
    Ok(DepositorInfoResponse {
        depositor: addr,
        deposit_amount: depositor.deposit_amount,
        shares: depositor.shares,
        reward_index: depositor.reward_index,
        pending_rewards: depositor.pending_rewards,
        tickets: depositor.tickets,
        unbonding_info: depositor.unbonding_info,
    })
}

pub fn query_depositors(
    deps: Deps,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<DepositorsInfoResponse> {
    let start_after = if let Some(start_after) = start_after {
        Some(deps.api.addr_validate(&start_after)?)
    } else {
        None
    };

    let depositors = read_depositors(deps, start_after, limit)?;
    Ok(DepositorsInfoResponse { depositors })
}

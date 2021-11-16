#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use crate::error::ContractError;
use crate::helpers::{
    calculate_winner_prize, claim_deposits, compute_depositor_reward, compute_reward,
    compute_sponsor_reward, is_valid_sequence, pseudo_random_seq,
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
    Response, StdError, StdResult, Timestamp, Uint128, WasmMsg,
};
use cw0::{Duration, Expiration};
use cw20::Cw20ExecuteMsg;
use cw_storage_plus::U64Key;
use glow_protocol::distributor::ExecuteMsg as FaucetExecuteMsg;
use glow_protocol::lotto::{
    Claim, ConfigResponse, DepositorInfoResponse, DepositorsInfoResponse, ExecuteMsg,
    InstantiateMsg, LotteryInfoResponse, MigrateMsg, PoolResponse, PrizeInfoResponse, QueryMsg,
    SponsorInfoResponse, StateResponse, TicketInfoResponse,
};
use glow_protocol::querier::deduct_tax;
use moneymarket::market::{Cw20HookMsg, EpochStateResponse, ExecuteMsg as AnchorMsg};
use std::ops::{Add, Sub};
use terraswap::querier::query_token_balance;

pub const INITIAL_DEPOSIT_AMOUNT: u128 = 10;
pub const SEQUENCE_DIGITS: u8 = 5;
pub const PRIZE_DISTR_LEN: usize = 6;
pub const MAX_CLAIMS: u8 = 15;
pub const THIRTY_MINUTE_TIME: u64 = 60 * 30;

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

    // Validate prize distribution
    if msg.prize_distribution.len() != PRIZE_DISTR_LEN {
        return Err(ContractError::InvalidPrizeDistribution {});
    }

    let mut sum = Decimal256::zero();
    for item in msg.prize_distribution.iter() {
        sum += *item;
    }

    if sum != Decimal256::one() {
        return Err(ContractError::InvalidPrizeDistribution {});
    }

    // Validate factors
    if msg.reserve_factor > Decimal256::one() {
        return Err(ContractError::InvalidReserveFactor {});
    }
    if msg.split_factor > Decimal256::one() {
        return Err(ContractError::InvalidSplitFactor {});
    }
    if msg.instant_withdrawal_fee > Decimal256::one() {
        return Err(ContractError::InvalidWithdrawalFee {});
    }

    // Validate that epoch_interval is at least 30 minutes
    if msg.epoch_interval < THIRTY_MINUTE_TIME {
        return Err(ContractError::InvalidEpochInterval {});
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
            epoch_interval: Duration::Time(msg.epoch_interval),
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

    // validate first lottery is in the future
    if msg.initial_lottery_execution <= env.block.time.seconds() {
        return Err(ContractError::InvalidFirstLotteryExec {});
    }

    STATE.save(
        deps.storage,
        &State {
            total_tickets: Uint256::zero(),
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(initial_deposit),
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(
                msg.initial_lottery_execution,
            )),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: Duration::Time(msg.epoch_interval).after(&env.block),
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
        } => execute_register_contracts(deps, info, gov_contract, distributor_contract),
        ExecuteMsg::Deposit { combinations } => execute_deposit(deps, env, info, combinations),
        ExecuteMsg::Gift {
            combinations,
            recipient,
        } => execute_gift(deps, env, info, combinations, recipient),
        ExecuteMsg::Sponsor { award } => execute_sponsor(deps, env, info, award),
        ExecuteMsg::SponsorWithdraw {} => execute_sponsor_withdraw(deps, env, info),
        ExecuteMsg::Withdraw { amount, instant } => {
            execute_withdraw(deps, env, info, amount, instant)
        }
        ExecuteMsg::Claim {} => execute_claim_unbonded(deps, env, info),
        ExecuteMsg::ClaimLottery { lottery_ids } => {
            execute_claim_lottery(deps, env, info, lottery_ids)
        }
        ExecuteMsg::ClaimRewards {} => execute_claim_rewards(deps, env, info),
        ExecuteMsg::ExecuteLottery {} => execute_lottery(deps, env, info),
        ExecuteMsg::ExecutePrize { limit } => execute_prize(deps, env, info, limit),
        ExecuteMsg::ExecuteEpochOps {} => execute_epoch_ops(deps, env),
        ExecuteMsg::UpdateConfig {
            owner,
            oracle_addr,
            reserve_factor,
            instant_withdrawal_fee,
            unbonding_period,
            epoch_interval,
        } => execute_update_config(
            deps,
            info,
            owner,
            oracle_addr,
            reserve_factor,
            instant_withdrawal_fee,
            unbonding_period,
            epoch_interval,
        ),
        ExecuteMsg::UpdateLotteryConfig {
            lottery_interval,
            block_time,
            ticket_price,
            prize_distribution,
            round_delta,
        } => execute_update_lottery_config(
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

pub fn execute_register_contracts(
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

pub fn deposit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    recipient: Option<String>,
    combinations: Vec<String>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;

    // get the amount of funds sent in the base stable denom
    let deposit_amount = info
        .funds
        .iter()
        .find(|c| c.denom == config.stable_denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    // validate that the deposit amount is non zero
    if deposit_amount.is_zero() {
        return if let Some(_recipient) = recipient {
            Err(ContractError::InvalidGiftAmount {})
        } else {
            Err(ContractError::InvalidDepositAmount {})
        };
    }

    // validate that all sequence combinations are valid
    for combination in combinations.clone() {
        if !is_valid_sequence(&combination, SEQUENCE_DIGITS) {
            return Err(ContractError::InvalidSequence {});
        }
    }

    // validate that the lottery has not already started
    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    // get the amount of requested tickets
    let mut amount_tickets = combinations.len() as u64;

    // validate that the deposit size is greater than or equal to the corresponding cost of the requested number of tickets
    let required_amount = config.ticket_price * Uint256::from(amount_tickets);
    if deposit_amount < required_amount {
        return if let Some(_recipient) = recipient {
            Err(ContractError::InsufficientGiftDepositAmount(amount_tickets))
        } else {
            Err(ContractError::InsufficientDepositAmount(amount_tickets))
        };
    }

    // get the depositor info
    // depositor being either the message sender
    // or the recipient that will be reciving the deposited funds if specified
    let depositor = if let Some(recipient) = recipient {
        deps.api.addr_validate(recipient.as_str())?
    } else {
        info.sender.clone()
    };
    let mut depositor_info: DepositorInfo = read_depositor_info(deps.storage, &depositor);

    // check if we need to round up number of combinations based on depositor total deposits
    let mut new_combinations = combinations.clone();
    if ((depositor_info.deposit_amount + Decimal256::from_uint256(deposit_amount))
        / config.ticket_price)
        >= (Decimal256::from_uint256(Uint256::from(
            (depositor_info.tickets.len() + combinations.len()) as u128,
        )) + Decimal256::one())
    {
        let current_time = env.block.time.nanos();
        let sequence = pseudo_random_seq(
            info.sender.clone().into_string(),
            depositor_info.tickets.len() as u64,
            current_time,
        );

        new_combinations.push(sequence);
        amount_tickets += 1;
    }

    // update the glow deposit reward index
    compute_reward(&mut state, &pool, env.block.height);
    // update the glow depositor reward for the depositor
    compute_depositor_reward(&state, &mut depositor_info);

    // query exchange_rate from anchor money market
    let epoch_state: EpochStateResponse = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?;

    // deduct tx taxes when calculating the net deposited amount in anchor
    let net_coin_amount = deduct_tax(
        deps.as_ref(),
        coin(deposit_amount.into(), config.stable_denom.clone()),
    )?;
    let post_tax_deposit_amount = net_coin_amount.amount;

    // get the amount of aUST entitled to the user from the deposit
    let minted_amount =
        Decimal256::from_uint256(post_tax_deposit_amount) / epoch_state.exchange_rate;

    // store the post tax deposit amount
    depositor_info.deposit_amount = depositor_info
        .deposit_amount
        .add(Decimal256::from_uint256(post_tax_deposit_amount));

    // update the depositors shares (basically the amount of aUST the depositor is entitled to)
    depositor_info.shares = depositor_info.shares.add(minted_amount);

    for combination in new_combinations {
        // check that the number of holders for any given ticket isn't too high
        if let Some(holders) = TICKETS
            .may_load(deps.storage, combination.as_bytes())
            .unwrap()
        {
            if holders.len() >= config.max_holders as usize {
                return Err(ContractError::InvalidHolderSequence {});
            }
        }

        // update the TICKETS storage
        let add_ticket = |a: Option<Vec<Addr>>| -> StdResult<Vec<Addr>> {
            let mut b = a.unwrap_or_default();
            b.push(depositor.clone());
            Ok(b)
        };
        TICKETS
            .update(deps.storage, combination.as_bytes(), add_ticket)
            .unwrap();
        // add the combination to the depositor_info
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
        .add(Decimal256::from_uint256(post_tax_deposit_amount));
    pool.lottery_deposits = pool
        .lottery_deposits
        .add(Decimal256::from_uint256(post_tax_deposit_amount) * config.split_factor);

    // update depositor and state information
    store_depositor_info(deps.storage, &depositor, &depositor_info)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    // save depositor and state information
    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.anchor_contract.to_string(),
            funds: vec![Coin {
                denom: config.stable_denom,
                amount: post_tax_deposit_amount,
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        })])
        .add_attributes(vec![
            attr("action", "deposit"),
            attr("depositor", info.sender.to_string()),
            attr("recipient", depositor.to_string()),
            attr("deposit_amount", deposit_amount.to_string()),
            attr("tickets", amount_tickets.to_string()),
            attr("shares_minted", minted_amount.to_string()),
        ]))
}

// Deposit UST and get pool shares and tickets in return
pub fn execute_deposit(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    combinations: Vec<String>,
) -> Result<Response, ContractError> {
    deposit(deps.branch(), env, info, None, combinations)
}

// Gift several tickets at once to a given address
pub fn execute_gift(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    combinations: Vec<String>,
    to: String,
) -> Result<Response, ContractError> {
    if to == info.sender {
        return Err(ContractError::InvalidGift {});
    }
    deposit(deps.branch(), env, info, Some(to), combinations)
}

// Make a donation deposit to the lottery pool
pub fn execute_sponsor(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    award: Option<bool>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;

    // get the amount of funds sent in the base stable denom
    let sponsor_amount = info
        .funds
        .iter()
        .find(|c| c.denom == config.stable_denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    // validate that the sponsor amount is non zero
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
        let epoch_state: EpochStateResponse = query_exchange_rate(
            deps.as_ref(),
            config.anchor_contract.to_string(),
            env.block.height,
        )?;

        // Discount tx taxes
        let net_coin_amount = deduct_tax(
            deps.as_ref(),
            coin(sponsor_amount.into(), config.stable_denom.clone()),
        )?;
        let net_sponsor_amount = net_coin_amount.amount;

        // add amount of aUST entitled from the deposit
        let minted_amount =
            Decimal256::from_uint256(net_sponsor_amount) / epoch_state.exchange_rate;

        // fetch sponsor_info
        let mut sponsor_info: SponsorInfo = read_sponsor_info(deps.storage, &info.sender);

        // update sponsor sponsor rewards
        compute_reward(&mut state, &pool, env.block.height);
        compute_sponsor_reward(&state, &mut sponsor_info);

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

pub fn execute_sponsor_withdraw(
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

    // Compute Glow depositor rewards
    compute_reward(&mut state, &pool, env.block.height);
    compute_sponsor_reward(&state, &mut sponsor_info);

    // Calculate aust amount to redeem based on depositor amount
    let contract_a_balance = query_token_balance(
        &deps.querier,
        config.a_terra_contract.clone(),
        env.clone().contract.address,
    )?;
    let rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;
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

    // Discount tx taxes from Anchor to Glow
    let coin_amount = deduct_tax(
        deps.as_ref(),
        coin(
            (sponsor_info.amount * Uint256::one()).into(),
            config.clone().stable_denom,
        ),
    )?
    .amount;

    // Discount tx taxes from Glow to User
    let net_coin_amount = deduct_tax(deps.as_ref(), coin(coin_amount.into(), config.stable_denom))?;

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

pub fn execute_withdraw(
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
    let rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;
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
    let redemed_amount = pooled_deposits * withdraw_ratio;
    // Discount tax Anchor -> Glow
    let mut return_amount = Uint256::from(
        deduct_tax(
            deps.as_ref(),
            coin(redemed_amount.into(), config.clone().stable_denom),
        )?
        .amount,
    );

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
        let net_coin_amount = deduct_tax(
            deps.as_ref(),
            coin(return_amount.into(), config.stable_denom),
        )?;

        msgs.push(CosmosMsg::Bank(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![net_coin_amount],
        }));
    } else {
        // Discount tx taxes
        let net_coin_amount = deduct_tax(
            deps.as_ref(),
            coin(return_amount.into(), config.stable_denom),
        )?;
        // Check max unbonding_info concurrent claims is not bypassed
        if depositor.unbonding_info.len() as u8 >= MAX_CLAIMS {
            return Err(ContractError::MaxUnbondingClaims {});
        }
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

// Send available UST to user from unbonded withdrawals
pub fn execute_claim_unbonded(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    let (to_send, mut depositor) = claim_deposits(deps.storage, &info.sender, &env.block, None)?;
    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    // Compute Glow depositor rewards
    compute_reward(&mut state, &pool, env.block.height);
    compute_depositor_reward(&state, &mut depositor);

    if to_send == Uint128::zero() {
        return Err(ContractError::InsufficientClaimableFunds {});
    }

    // Deduct taxes on the claim
    let net_send = deduct_tax(
        deps.as_ref(),
        coin(to_send.into(), config.stable_denom.clone()),
    )?
    .amount;

    // Double-check if there is enough balance to send in the contract
    let balance = query_balance(
        deps.as_ref(),
        env.contract.address.to_string(),
        config.stable_denom.clone(),
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
            attr("action", "claim_unbonded"),
            attr("depositor", info.sender.to_string()),
            attr("redeemed_amount", net_send),
        ]))
}

// Send available UST to user from prizes won in the given lottery_id
pub fn execute_claim_lottery(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    lottery_ids: Vec<u64>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    let mut to_send = Uint128::zero();
    let mut depositor = read_depositor_info(deps.storage, &info.sender);

    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    // Compute Glow depositor rewards
    compute_reward(&mut state, &pool, env.block.height);
    compute_depositor_reward(&state, &mut depositor);

    for lottery_id in lottery_ids.clone() {
        let lottery = read_lottery_info(deps.storage, lottery_id);
        if !lottery.awarded {
            return Err(ContractError::InvalidClaimLotteryNotAwarded {});
        }
        //Calculate and add to to_send
        let lottery_key: U64Key = U64Key::from(lottery_id);
        let prizes = PRIZES
            .may_load(deps.storage, (&info.sender, lottery_key.clone()))
            .unwrap();
        if let Some(prize) = prizes {
            if prize.claimed {
                return Err(ContractError::InvalidClaimPrizeAlreadyClaimed {});
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

    // Deduct reserve fee
    let reserve_fee = Uint256::from(to_send) * config.reserve_factor;
    to_send -= Uint128::from(reserve_fee);
    state.total_reserve += Decimal256::from_uint256(reserve_fee);

    // Deduct taxes on the claim
    let net_send = deduct_tax(
        deps.as_ref(),
        coin(to_send.into(), config.stable_denom.clone()),
    )?
    .amount;

    // Double-check if there is enough balance to send in the contract
    let balance = query_balance(
        deps.as_ref(),
        env.contract.address.to_string(),
        config.stable_denom.clone(),
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
            attr("action", "claim_lottery"),
            attr("lottery_ids", format!("{:?}", lottery_ids)),
            attr("depositor", info.sender.to_string()),
            attr("redeemed_amount", net_send),
        ]))
}

pub fn execute_epoch_ops(deps: DepsMut, env: Env) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    // Validate that executing epoch will follow rate limiting
    if !state.next_epoch.is_expired(&env.block) {
        return Err(ContractError::InvalidEpochExecution {});
    }

    // Validate that the lottery is not in the process of running
    // This helps avoid delaying the computing of the reward following lottery execution.
    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
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

    // Update next_epoch based on epoch_interval
    state.next_epoch = Expiration::AtTime(env.block.time).add(config.epoch_interval)?;
    // Empty total reserve and store state
    state.total_reserve = Decimal256::zero();
    STATE.save(deps.storage, &state)?;

    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("action", "execute_epoch_operations"),
        attr("total_reserves", total_reserves.to_string()),
        attr("glow_emission_rate", state.glow_emission_rate.to_string()),
    ]))
}

pub fn execute_claim_rewards(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    let depositor_address = info.sender.as_str();
    let mut depositor: DepositorInfo = read_depositor_info(deps.storage, &info.sender);
    let mut sponsor: SponsorInfo = read_sponsor_info(deps.storage, &info.sender);

    // Compute Glow depositor rewards
    compute_reward(&mut state, &pool, env.block.height);
    compute_depositor_reward(&state, &mut depositor);
    compute_sponsor_reward(&state, &mut sponsor);

    let claim_amount = (depositor.pending_rewards + sponsor.pending_rewards) * Uint256::one();
    depositor.pending_rewards = Decimal256::zero();
    sponsor.pending_rewards = Decimal256::zero();

    STATE.save(deps.storage, &state)?;
    store_depositor_info(deps.storage, &info.sender, &depositor)?;
    store_sponsor_info(deps.storage, &info.sender, &sponsor)?;

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
pub fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    oracle_addr: Option<String>,
    reserve_factor: Option<Decimal256>,
    instant_withdrawal_fee: Option<Decimal256>,
    unbonding_period: Option<u64>,
    epoch_interval: Option<u64>,
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
            return Err(ContractError::InvalidWithdrawalFee {});
        }
        config.instant_withdrawal_fee = instant_withdrawal_fee;
    }

    if let Some(unbonding_period) = unbonding_period {
        config.unbonding_period = Duration::Time(unbonding_period);
    }

    if let Some(epoch_interval) = epoch_interval {
        // validate that epoch_interval is at least 30 minutes
        if epoch_interval < THIRTY_MINUTE_TIME {
            return Err(ContractError::InvalidEpochInterval {});
        }

        config.epoch_interval = Duration::Time(epoch_interval);
    }

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![("action", "update_config")]))
}

pub fn execute_update_lottery_config(
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
        if prize_distribution.len() != PRIZE_DISTR_LEN {
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
        QueryMsg::Depositor { address } => to_binary(&query_depositor(deps, env, address)?),
        QueryMsg::Sponsor { address } => to_binary(&query_sponsor(deps, env, address)?),
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
        epoch_interval: config.epoch_interval,
        block_time: config.block_time,
        round_delta: config.round_delta,
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
            rand_round: lottery.rand_round,
            sequence: lottery.sequence,
            awarded: lottery.awarded,
            timestamp: lottery.timestamp,
            total_prizes: lottery.total_prizes,
            number_winners: lottery.number_winners,
            page: lottery.page,
        })
    } else {
        let current_lottery = query_state(deps, env, None)?.current_lottery;
        let lottery = read_lottery_info(deps.storage, current_lottery);
        Ok(LotteryInfoResponse {
            lottery_id: current_lottery,
            rand_round: lottery.rand_round,
            sequence: lottery.sequence,
            awarded: lottery.awarded,
            timestamp: lottery.timestamp,
            total_prizes: lottery.total_prizes,
            number_winners: lottery.number_winners,
            page: lottery.page,
        })
    }
}

pub fn query_depositor(deps: Deps, env: Env, addr: String) -> StdResult<DepositorInfoResponse> {
    let address = deps.api.addr_validate(&addr)?;
    let mut depositor = read_depositor_info(deps.storage, &address);

    let mut state = STATE.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;

    // compute rewards
    compute_reward(&mut state, &pool, env.block.height);
    compute_depositor_reward(&state, &mut depositor);

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

pub fn query_sponsor(deps: Deps, env: Env, addr: String) -> StdResult<SponsorInfoResponse> {
    let address = deps.api.addr_validate(&addr)?;
    let mut sponsor = read_sponsor_info(deps.storage, &address);

    let mut state = STATE.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;

    // compute rewards
    compute_reward(&mut state, &pool, env.block.height);
    compute_sponsor_reward(&state, &mut sponsor);

    Ok(SponsorInfoResponse {
        sponsor: addr,
        amount: sponsor.amount,
        shares: sponsor.shares,
        reward_index: sponsor.reward_index,
        pending_rewards: sponsor.pending_rewards,
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

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use crate::error::ContractError;
use crate::helpers::{
    assert_prize_distribution_not_pending, calculate_value_of_aust_to_be_redeemed_for_lottery,
    claim_unbonded_withdrawals, compute_global_operator_reward, compute_global_sponsor_reward,
    compute_operator_reward, compute_sponsor_reward, decimal_from_ratio_or_one,
    handle_depositor_operator_updates, handle_depositor_ticket_updates,
    old_compute_depositor_reward, old_compute_reward,
};
use crate::querier::{query_balance, query_exchange_rate};
use crate::state::{
    old_read_depositors, old_read_lottery_info, old_read_prize_infos, old_remove_depositor_info,
    read_depositor_info, read_depositor_stats, read_depositors_info, read_depositors_stats,
    read_operator_info, read_sponsor_info, store_depositor_info, store_operator_info,
    store_sponsor_info, Config, OperatorInfo, Pool, SponsorInfo, State, CONFIG, OLDCONFIG, OLDPOOL,
    OLDSTATE, POOL, STATE, TICKETS,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, coin, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo,
    Response, StdError, StdResult, Uint128, WasmMsg,
};
use cw0::{Duration, Expiration};
use cw20::Cw20ExecuteMsg;

use glow_protocol::distributor::ExecuteMsg as FaucetExecuteMsg;
use glow_protocol::lotto::{
    AmountRedeemableForPrizesResponse, BoostConfig, Claim, ConfigResponse, DepositorInfoResponse,
    DepositorStatsResponse, DepositorsInfoResponse, DepositorsStatsResponse, ExecuteMsg,
    InstantiateMsg, MigrateMsg, OldLotteryInfoResponse, OperatorInfoResponse, PoolResponse,
    QueryMsg, RewardEmissionsIndex, SponsorInfoResponse, StateResponse, TicketInfoResponse,
};
use glow_protocol::lotto::{DepositorInfo, OldPrizeInfosResponse};
use glow_protocol::querier::deduct_tax;
use moneymarket::market::{Cw20HookMsg, ExecuteMsg as AnchorMsg};

use std::ops::{Add, Sub};

use terraswap::querier::query_token_balance;

pub const INITIAL_DEPOSIT_AMOUNT: u128 = 10_000_000;
pub const MAX_CLAIMS: u8 = 15;
pub const THIRTY_MINUTE_TIME: u64 = 60 * 30;
pub const MAX_HOLDERS_FLOOR: u8 = 10;
pub const MAX_HOLDERS_CAP: u8 = 100;

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
        return Err(ContractError::InvalidDepositInstantiation(initial_deposit));
    }

    if msg.split_factor > Decimal256::one() {
        return Err(ContractError::InvalidSplitFactor {});
    }
    if msg.instant_withdrawal_fee > Decimal256::one() {
        return Err(ContractError::InvalidWithdrawalFee {});
    }

    // Validate ticket price
    if msg.ticket_price < Uint256::from(10u128) {
        // Ticket price must be at least 10 uusd
        return Err(ContractError::InvalidTicketPrice {});
    }

    // Validate that max_holders is within the bounds
    if msg.max_holders < MAX_HOLDERS_FLOOR || MAX_HOLDERS_CAP < msg.max_holders {
        return Err(ContractError::InvalidMaxHoldersOutsideBounds {});
    }

    // Get and validate the lotto winner boost config
    let _default_lotto_winner_boost_config: BoostConfig = BoostConfig {
        base_multiplier: Decimal256::from_ratio(Uint256::from(40u128), Uint256::from(100u128)),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(150),
    };

    CONFIG.save(
        deps.storage,
        &Config {
            owner: deps.api.addr_validate(msg.owner.as_str())?,
            a_terra_contract: deps.api.addr_validate(msg.aterra_contract.as_str())?,
            gov_contract: Addr::unchecked(""),
            ve_contract: Addr::unchecked(""),
            community_contract: Addr::unchecked(""),
            distributor_contract: Addr::unchecked(""),
            prize_distributor_contract: Addr::unchecked(""),
            oracle_contract: deps.api.addr_validate(msg.oracle_contract.as_str())?,
            stable_denom: msg.stable_denom.clone(),
            anchor_contract: deps.api.addr_validate(msg.anchor_contract.as_str())?,
            ticket_price: msg.ticket_price,
            max_holders: msg.max_holders,
            split_factor: msg.split_factor,
            instant_withdrawal_fee: msg.instant_withdrawal_fee,
            unbonding_period: Duration::Time(msg.unbonding_period),
            max_tickets_per_depositor: msg.max_tickets_per_depositor,
            paused: false,
        },
    )?;

    // Query exchange_rate from anchor money market
    let aust_exchange_rate: Decimal256 = query_exchange_rate(
        deps.as_ref(),
        deps.api
            .addr_validate(msg.anchor_contract.as_str())?
            .to_string(),
        env.block.height,
    )?
    .exchange_rate;

    STATE.save(
        deps.storage,
        &State {
            total_tickets: Uint256::zero(),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: env.block.height,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: msg.initial_operator_glow_emission_rate,
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: env.block.height,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: msg.initial_sponsor_glow_emission_rate,
            },
            last_lottery_execution_aust_exchange_rate: aust_exchange_rate,
        },
    )?;

    POOL.save(
        deps.storage,
        &Pool {
            total_user_aust: Uint256::zero(),
            total_user_shares: Uint256::zero(),
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_operator_shares: Uint256::zero(),
        },
    )?;

    // Deduct taxes that will be payed when transferring to anchor
    let tax_deducted_initial_deposit = Uint256::from(
        deduct_tax(
            deps.as_ref(),
            coin(initial_deposit.into(), msg.stable_denom.clone()),
        )?
        .amount,
    );

    // Convert the initial deposit amount to aust
    let messages: Vec<CosmosMsg> = vec![CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: msg.anchor_contract,
        funds: vec![Coin {
            denom: msg.stable_denom,
            amount: tax_deducted_initial_deposit.into(),
        }],
        msg: to_binary(&AnchorMsg::DepositStable {})?,
    })];

    Ok(Response::default().add_messages(messages))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    if let ExecuteMsg::MigrateOldDepositors { limit } = msg {
        return migrate_old_depositors(deps, env, limit);
    }

    if let ExecuteMsg::UpdateConfig {
        owner,
        oracle_addr,
        instant_withdrawal_fee,
        unbonding_period,
        max_holders,
        max_tickets_per_depositor,
        paused,
        operator_glow_emission_rate,
        sponsor_glow_emission_rate,
    } = msg
    {
        return execute_update_config(
            deps,
            info,
            owner,
            oracle_addr,
            instant_withdrawal_fee,
            unbonding_period,
            max_holders,
            max_tickets_per_depositor,
            paused,
            operator_glow_emission_rate,
            sponsor_glow_emission_rate,
        );
    }

    let config = CONFIG.load(deps.storage)?;
    if config.paused {
        return Err(ContractError::ContractPaused {});
    }

    match msg {
        ExecuteMsg::RegisterContracts {
            gov_contract,
            community_contract,
            distributor_contract,
            ve_contract,
            prize_distributor_contract,
        } => execute_register_contracts(
            deps,
            info,
            gov_contract,
            community_contract,
            distributor_contract,
            ve_contract,
            prize_distributor_contract,
        ),
        ExecuteMsg::Deposit {
            encoded_tickets,
            operator,
        } => execute_deposit(deps, env, info, encoded_tickets, operator),
        ExecuteMsg::ClaimTickets { encoded_tickets } => {
            execute_claim_tickets(deps, env, info, encoded_tickets)
        }
        ExecuteMsg::Gift {
            encoded_tickets,
            recipient,
            operator,
        } => execute_gift(deps, env, info, encoded_tickets, recipient, operator),
        ExecuteMsg::Sponsor {
            award: _,
            prize_distribution: _,
        } => execute_sponsor(deps, env, info),
        ExecuteMsg::SponsorWithdraw {} => execute_sponsor_withdraw(deps, env, info),
        ExecuteMsg::Withdraw { amount, instant } => {
            execute_withdraw(deps, env, info, amount, instant)
        }
        ExecuteMsg::Claim {} => execute_claim_unbonded(deps, env, info),
        ExecuteMsg::ClaimRewards {} => execute_claim_rewards(deps, env, info),
        ExecuteMsg::UpdateLotteryConfig { ticket_price } => {
            execute_update_lottery_config(deps, info, ticket_price)
        }
        ExecuteMsg::SendPrizeFundsToPrizeDistributor {} => {
            execute_send_prize_funds_to_prize_distributor(deps, env)
        }
        ExecuteMsg::UpdateConfig { .. } => unreachable!(),
        ExecuteMsg::MigrateOldDepositors { .. } => unreachable!(),
    }
}

pub fn execute_send_prize_funds_to_prize_distributor(
    deps: DepsMut,
    env: Env,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    // Get the contract's aust balance
    let contract_a_balance = Uint256::from(query_token_balance(
        &deps.querier,
        config.a_terra_contract.clone(),
        env.clone().contract.address,
    )?);

    // Get the aust exchange rate
    let aust_exchange_rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;

    let amount_redeemable_for_prizes = calculate_value_of_aust_to_be_redeemed_for_lottery(
        &state,
        &pool,
        &config,
        contract_a_balance,
        aust_exchange_rate,
    );

    // Update the latest aust_exchange_rate
    state.last_lottery_execution_aust_exchange_rate = aust_exchange_rate;
    // Subtract from the total_user_aust
    pool.total_user_aust = pool.total_user_aust - amount_redeemable_for_prizes.user_aust_to_redeem;

    // Save changes to state
    STATE.save(deps.storage, &state)?;

    // Save changes to pool
    POOL.save(deps.storage, &pool)?;

    // Send message to send funds to prize distributor
    Ok(
        Response::default().add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.a_terra_contract.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: config.prize_distributor_contract.to_string(),
                amount: amount_redeemable_for_prizes.aust_to_redeem.into(),
            })?,
        })),
    )
}

pub fn execute_register_contracts(
    deps: DepsMut,
    info: MessageInfo,
    gov_contract: String,
    community_contract: String,
    distributor_contract: String,
    ve_contract: String,
    prize_distributor_contract: String,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;

    // check permission
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    // can't be registered twice
    if config.contracts_registered() {
        return Err(ContractError::AlreadyRegistered {});
    }

    config.gov_contract = deps.api.addr_validate(&gov_contract)?;
    config.community_contract = deps.api.addr_validate(&community_contract)?;
    config.distributor_contract = deps.api.addr_validate(&distributor_contract)?;
    config.ve_contract = deps.api.addr_validate(&ve_contract)?;
    config.prize_distributor_contract = deps.api.addr_validate(&prize_distributor_contract)?;
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::default())
}

pub fn deposit(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    recipient: Option<String>,
    new_operator_addr: Option<String>,
    encoded_tickets: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;

    // Get the aust exchange rate
    let aust_exchange_rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;

    // Get the amount of funds sent in the base stable denom
    let deposit_amount = info
        .funds
        .iter()
        .find(|c| c.denom == config.stable_denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    // Get the depositor info
    // depositor being either the message sender
    // or the recipient that will be reciving the deposited funds if specified
    let depositor = if let Some(recipient) = recipient.clone() {
        deps.api.addr_validate(recipient.as_str())?
    } else {
        info.sender.clone()
    };
    let mut depositor_info: DepositorInfo = read_depositor_info(deps.storage, &depositor);

    // Validate that the prize distribution isn't pending
    assert_prize_distribution_not_pending(deps.as_ref(), &config.prize_distributor_contract)?;

    // Validate that the deposit amount is non zero
    if deposit_amount.is_zero() {
        return if recipient.is_some() {
            Err(ContractError::ZeroGiftAmount {})
        } else {
            Err(ContractError::ZeroDepositAmount {})
        };
    }

    // Deduct tx taxes when calculating the net deposited amount in anchor
    let net_coin_amount = deduct_tax(
        deps.as_ref(),
        coin(deposit_amount.into(), config.stable_denom.clone()),
    )?;

    let post_tax_deposit_amount = Uint256::from(net_coin_amount.amount);

    // Get the number of minted aust
    let minted_aust = post_tax_deposit_amount / aust_exchange_rate;

    // Get the amount of minted_shares
    // based on the total user shares to total user aust ratio
    let minted_shares =
        minted_aust * decimal_from_ratio_or_one(pool.total_user_shares, pool.total_user_aust);

    // Handle depositor ticket updates
    let number_of_new_tickets = handle_depositor_ticket_updates(
        deps.branch(),
        &env,
        &config,
        &pool,
        &depositor,
        &mut depositor_info,
        encoded_tickets,
        aust_exchange_rate,
        minted_shares,
        minted_aust,
    )?;

    // Update the global reward index
    compute_global_operator_reward(&mut state, &pool, env.block.height);

    // Update operator information
    handle_depositor_operator_updates(
        deps.branch(),
        &mut state,
        &mut pool,
        &depositor,
        &mut depositor_info,
        minted_shares,
        new_operator_addr,
    )?;

    // Increase the depositor's shares by the number of minted shares
    depositor_info.shares = depositor_info.shares.add(minted_shares);

    // Increase total_user_shares by the number of minted shares
    pool.total_user_shares = pool.total_user_shares.add(minted_shares);

    // Increase total_user_aust
    pool.total_user_aust = pool.total_user_aust.add(minted_aust);

    // Update the number of total_tickets
    state.total_tickets = state.total_tickets.add(number_of_new_tickets.into());

    // Save changes to depositor_info, state, and pool
    store_depositor_info(deps.storage, &depositor, depositor_info, env.block.height)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    // Respond
    Ok(Response::new()
        // Add a message to move the deposited UST to anchor
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.anchor_contract.to_string(),
            funds: vec![Coin {
                denom: config.stable_denom,
                amount: post_tax_deposit_amount.into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        })])
        .add_attributes(vec![
            attr("action", "deposit"),
            attr("depositor", info.sender.to_string()),
            attr("recipient", depositor.to_string()),
            attr("deposit_amount", deposit_amount.to_string()),
            attr("tickets", number_of_new_tickets.to_string()),
            attr("aust_minted", minted_aust.to_string()),
        ]))
}

// Deposit UST and get savings aust and tickets in return
pub fn execute_deposit(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    encoded_tickets: String,
    operator_addr: Option<String>,
) -> Result<Response, ContractError> {
    deposit(
        deps.branch(),
        env,
        info,
        None,
        operator_addr,
        encoded_tickets,
    )
}

// Deposit UST and get savings aust and tickets in return
pub fn execute_claim_tickets(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    encoded_tickets: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let _state = STATE.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;

    let depositor = info.sender.clone();
    let mut depositor_info: DepositorInfo = read_depositor_info(deps.storage, &depositor);

    // Get the aust exchange rate
    let aust_exchange_rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;

    // Validate that the prize distribution isn't pending
    assert_prize_distribution_not_pending(deps.as_ref(), &config.prize_distributor_contract)?;

    // Propogate depositor ticket updates
    let number_of_new_tickets = handle_depositor_ticket_updates(
        deps.branch(),
        &env,
        &config,
        &pool,
        &depositor,
        &mut depositor_info,
        encoded_tickets,
        aust_exchange_rate,
        Uint256::zero(),
        Uint256::zero(),
    )?;

    // Save changes to depositor_info
    store_depositor_info(deps.storage, &depositor, depositor_info, env.block.height)?;

    // Save depositor and state information
    Ok(Response::new().add_attributes(vec![
        attr("action", "claim_tickets"),
        attr("depositor", info.sender.to_string()),
        attr("recipient", depositor.to_string()),
        attr("tickets", number_of_new_tickets.to_string()),
    ]))
}

// Gift several tickets at once to a given address
pub fn execute_gift(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    encoded_tickets: String,
    to: String,
    operator_addr: Option<String>,
) -> Result<Response, ContractError> {
    if to == info.sender {
        return Err(ContractError::GiftToSelf {});
    }
    deposit(
        deps.branch(),
        env,
        info,
        Some(to),
        operator_addr,
        encoded_tickets,
    )
}

// Make a donation deposit to the lottery pool
pub fn execute_sponsor(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let mut pool = POOL.load(deps.storage)?;

    // Get the amount of funds sent in the base stable denom
    let sponsor_amount = info
        .funds
        .iter()
        .find(|c| c.denom == config.stable_denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    // Validate that the prize distribution isn't pending
    assert_prize_distribution_not_pending(deps.as_ref(), &config.prize_distributor_contract)?;

    // Validate that the sponsor amount is non zero
    if sponsor_amount.is_zero() {
        return Err(ContractError::ZeroSponsorshipAmount {});
    }

    // Update global sponsor reward index
    compute_global_sponsor_reward(&mut state, &pool, env.block.height);

    let mut msgs: Vec<CosmosMsg> = vec![];

    // Deduct taxes that will be payed when transferring to anchor
    let net_sponsor_amount = Uint256::from(
        deduct_tax(
            deps.as_ref(),
            coin(sponsor_amount.into(), config.stable_denom.clone()),
        )?
        .amount,
    );

    // Query exchange_rate from anchor money market
    let aust_exchange_rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;

    // Add amount of aUST entitled from the deposit
    let minted_aust = net_sponsor_amount / aust_exchange_rate;

    // Get minted_aust_value
    let minted_aust_value = minted_aust * aust_exchange_rate;

    // Fetch sponsor_info
    let mut sponsor_info: SponsorInfo = read_sponsor_info(deps.storage, &info.sender);

    // Update sponsor sponsor rewards
    compute_sponsor_reward(&state, &mut sponsor_info);

    // Add sponsor_amount to depositor
    sponsor_info.lottery_deposit = sponsor_info.lottery_deposit.add(minted_aust_value);
    store_sponsor_info(deps.storage, &info.sender, sponsor_info)?;

    // Update pool
    pool.total_sponsor_lottery_deposits =
        pool.total_sponsor_lottery_deposits.add(minted_aust_value);

    // Push message to deposit stable coins into anchor
    msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.anchor_contract.to_string(),
        funds: vec![Coin {
            denom: config.stable_denom,
            amount: net_sponsor_amount.into(),
        }],
        msg: to_binary(&AnchorMsg::DepositStable {})?,
    }));

    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
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

    // Get the aust exchange rate
    let rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;

    // Validate that the prize distribution isn't pending
    assert_prize_distribution_not_pending(deps.as_ref(), &config.prize_distributor_contract)?;

    // Don't let sponsors withdraw if the aust exchange rate collapses
    if rate < state.last_lottery_execution_aust_exchange_rate {
        return Err(ContractError::AnchorExchangeRateCollapse {});
    }

    let mut sponsor_info: SponsorInfo = read_sponsor_info(deps.storage, &info.sender);

    // Validate that the sponsor has a lottery deposit
    if sponsor_info.lottery_deposit.is_zero() {
        return Err(ContractError::NoSponsorLotteryDeposit {});
    }

    // Update the global sponsor reward index
    compute_global_sponsor_reward(&mut state, &pool, env.block.height);
    // Update the reward index for the sponsor
    compute_sponsor_reward(&state, &mut sponsor_info);

    let aust_to_redeem = sponsor_info.lottery_deposit / rate;
    let aust_to_redeem_value = aust_to_redeem * rate;

    // Update global state

    pool.total_sponsor_lottery_deposits = pool
        .total_sponsor_lottery_deposits
        .sub(sponsor_info.lottery_deposit);

    // Update sponsor info
    sponsor_info.lottery_deposit = Uint256::zero();

    let mut msgs: Vec<CosmosMsg> = vec![];

    // Message for redeem amount operation of aUST
    let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.a_terra_contract.to_string(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Send {
            contract: config.anchor_contract.to_string(),
            amount: aust_to_redeem.into(),
            msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
        })?,
    });
    msgs.push(redeem_msg);

    // Discount tx taxes from Anchor to Glow
    let coin_amount = deduct_tax(
        deps.as_ref(),
        coin(aust_to_redeem_value.into(), config.clone().stable_denom),
    )?
    .amount;

    // Discount tx taxes from Glow to User
    let net_coin_amount = deduct_tax(deps.as_ref(), coin(coin_amount.into(), config.stable_denom))?;

    msgs.push(CosmosMsg::Bank(BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: vec![net_coin_amount],
    }));

    // Save changes to sponsor_info, state, and pool
    store_sponsor_info(deps.storage, &info.sender, sponsor_info)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "withdraw_sponsor"),
        attr("depositor", info.sender.to_string()),
        attr("redeem_amount_anchor", aust_to_redeem.to_string()),
        attr("redeem_stable_amount", aust_to_redeem_value),
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

    let mut depositor_info: DepositorInfo = read_depositor_info(deps.storage, &info.sender);

    // Get the aust exchange rate
    let aust_exchange_rate = query_exchange_rate(
        deps.as_ref(),
        config.anchor_contract.to_string(),
        env.block.height,
    )?
    .exchange_rate;

    // Validate that the prize distribution isn't pending
    assert_prize_distribution_not_pending(deps.as_ref(), &config.prize_distributor_contract)?;

    // Validate that the user has savings aust to withdraw
    if depositor_info.shares.is_zero() {
        return Err(ContractError::NoDepositorSavingsAustToWithdraw {});
    }

    // Validate that the user is withdrawing a non zero amount
    if (amount.is_some()) && (amount.unwrap().is_zero()) {
        return Err(ContractError::SpecifiedWithdrawAmountIsZero {});
    }

    // Get the number of withdrawn shares
    let withdrawn_shares = amount
        .map(|amount| {
            std::cmp::max(
                // Aust to withdraw
                (Uint256::from(amount) / aust_exchange_rate)
                    // Multiply by total shares / total aust to get the shares to withdraw
                    .multiply_ratio(pool.total_user_shares, pool.total_user_aust),
                // Always withdraw at least one share
                Uint256::one(),
            )
        })
        .unwrap_or_else(|| depositor_info.shares);

    // Get the withdrawn amount of aust
    let withdrawn_aust =
        withdrawn_shares.multiply_ratio(pool.total_user_aust, pool.total_user_shares);

    let withdrawn_aust_value = withdrawn_aust * aust_exchange_rate;

    // Calculate the depositor's balance from their aust balance
    let depositor_balance = pool.total_user_aust
    // When withdrawing, depositor_info.shares must be positive and therefore pool.total_user_shares must be positive
        * Decimal256::from_ratio(depositor_info.shares, pool.total_user_shares)
        * aust_exchange_rate;

    if withdrawn_aust_value > depositor_balance {
        return Err(ContractError::SpecifiedWithdrawAmountTooBig {
            amount: Uint128::from(withdrawn_aust_value),
            depositor_balance,
        });
    }

    // Get the depositor's balance post withdraw
    let post_transaction_depositor_balance = (pool.total_user_aust - withdrawn_aust)
        * decimal_from_ratio_or_one(
            depositor_info.shares - withdrawn_shares,
            pool.total_user_shares - withdrawn_shares,
        )
        * aust_exchange_rate;

    let post_transaction_max_depositor_tickets = Uint128::from(
        post_transaction_depositor_balance
            / Decimal256::from_uint256(
                config.ticket_price
            // Subtract 10^-5 in order to offset rounding problems
            // relies on ticket price being at least 10^-5 UST
                - Uint256::from(10u128),
            ),
    )
    .u128();

    // Calculate how many tickets to remove
    let num_depositor_tickets = depositor_info.tickets.len() as u128;

    // Get the number of tickets to withdraw
    let num_withdrawn_tickets: u128 = num_depositor_tickets
        .checked_sub(post_transaction_max_depositor_tickets)
        .unwrap_or_default();

    if num_withdrawn_tickets > num_depositor_tickets {
        return Err(ContractError::WithdrawingTooManyTickets {
            withdrawn_tickets: num_withdrawn_tickets,
            num_depositor_tickets,
        });
    }

    for seq in depositor_info
        .tickets
        .drain(..num_withdrawn_tickets as usize)
    {
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

    // Update operator information
    if depositor_info.operator_registered() {
        let mut operator = read_operator_info(deps.storage, &depositor_info.operator_addr);

        // Update the global operator reward index
        compute_global_operator_reward(&mut state, &pool, env.block.height);
        // Update the reward index for the operator
        compute_operator_reward(&state, &mut operator);

        // Subtract shares from the operator
        operator.shares = operator.shares.sub(withdrawn_shares);

        // Save changes to operator_info
        store_operator_info(deps.storage, &depositor_info.operator_addr, operator)?;

        // Subtract shares from total_operator_shares
        pool.total_operator_shares = pool.total_operator_shares.sub(withdrawn_shares);
    }

    // Update depositor info
    depositor_info.shares = depositor_info.shares.sub(withdrawn_shares);

    // Update pool
    pool.total_user_shares = pool.total_user_shares.sub(withdrawn_shares);
    pool.total_user_aust = pool.total_user_aust.sub(withdrawn_aust);

    // Remove withdrawn_tickets from total_tickets
    state.total_tickets = state
        .total_tickets
        .sub(Uint256::from(num_withdrawn_tickets));

    // Get the value of the returned amount after accounting for taxes.
    let mut return_amount = Uint256::from(
        deduct_tax(
            deps.as_ref(),
            coin(withdrawn_aust_value.into(), config.clone().stable_denom),
        )?
        .amount,
    );

    let mut msgs: Vec<CosmosMsg> = vec![];

    // Message for redeem amount operation of aUST
    let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.a_terra_contract.to_string(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Send {
            contract: config.anchor_contract.to_string(),
            amount: withdrawn_aust.into(),
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

        // TODO Double check that unnecessary
        // // Add the withdrawal fee to the total_reserve
        // state.total_reserve += withdrawal_fee;

        // Get the amount of ust to return after tax
        let net_coin_amount = deduct_tax(
            deps.as_ref(),
            coin(return_amount.into(), config.stable_denom),
        )?;

        // Add message to send UST to the depositor
        msgs.push(CosmosMsg::Bank(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![net_coin_amount],
        }));
    } else {
        // Check max unbonding_info concurrent claims is not bypassed
        if depositor_info.unbonding_info.len() as u8 >= MAX_CLAIMS {
            return Err(ContractError::MaxUnbondingClaims {});
        }
        // Place amount in unbonding state as a claim
        depositor_info.unbonding_info.push(Claim {
            amount: return_amount,
            release_at: config.unbonding_period.after(&env.block),
        });
    }

    // Store changes to depositor_info, state, and pool
    store_depositor_info(deps.storage, &info.sender, depositor_info, env.block.height)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "withdraw_ticket"),
        attr("depositor", info.sender.to_string()),
        attr("tickets_amount", num_withdrawn_tickets.to_string()),
        attr("redeem_amount_anchor", withdrawn_aust.to_string()),
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
    let state = STATE.load(deps.storage)?;

    // Validate that the prize distribution isn't pending
    assert_prize_distribution_not_pending(deps.as_ref(), &config.prize_distributor_contract)?;

    let mut depositor = read_depositor_info(deps.storage, &info.sender);

    let to_send = claim_unbonded_withdrawals(&mut depositor, &env.block, None)?;

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

    if to_send > balance.into() {
        // Should never happen
        return Err(ContractError::InsufficientFunds {
            to_send,
            available_balance: balance,
        });
    }

    // Save changes to depositor_info and state
    store_depositor_info(deps.storage, &info.sender, depositor, env.block.height)?;
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

pub fn execute_claim_rewards(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    let depositor_address = info.sender.as_str();
    let mut sponsor: SponsorInfo = read_sponsor_info(deps.storage, &info.sender);
    let mut operator: OperatorInfo = read_operator_info(deps.storage, &info.sender);

    // Validate distributor contract has already been registered
    if !config.contracts_registered() {
        return Err(ContractError::NotRegistered {});
    }

    // Update global operator reward index
    compute_global_operator_reward(&mut state, &pool, env.block.height);
    // Update global sponsor reward index
    compute_global_sponsor_reward(&mut state, &pool, env.block.height);
    // Update operator reward index
    compute_operator_reward(&state, &mut operator);
    // Update sponsor reward index
    compute_sponsor_reward(&state, &mut sponsor);

    let claim_amount = (operator.pending_rewards + sponsor.pending_rewards) * Uint256::one();
    sponsor.pending_rewards = Decimal256::zero();
    operator.pending_rewards = Decimal256::zero();

    // Save changes to state, sponsor_info, and operator_info
    STATE.save(deps.storage, &state)?;
    store_sponsor_info(deps.storage, &info.sender, sponsor)?;
    store_operator_info(deps.storage, &info.sender, operator)?;

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
    instant_withdrawal_fee: Option<Decimal256>,
    unbonding_period: Option<u64>,
    max_holders: Option<u8>,
    max_tickets_per_depositor: Option<u64>,
    paused: Option<bool>,
    operator_glow_emission_rate: Option<Decimal256>,
    sponsor_glow_emission_rate: Option<Decimal256>,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;

    // Check permission
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    // Change owner of Glow lotto contract
    if let Some(owner) = owner {
        config.owner = deps.api.addr_validate(owner.as_str())?;
    }

    // Change oracle contract addr
    if let Some(oracle_addr) = oracle_addr {
        config.owner = deps.api.addr_validate(oracle_addr.as_str())?;
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

    if let Some(max_holders) = max_holders {
        // Validate that max_holders is within the bounds
        if !(MAX_HOLDERS_FLOOR..=MAX_HOLDERS_CAP).contains(&max_holders) {
            return Err(ContractError::InvalidMaxHoldersOutsideBounds {});
        }

        // Validate that max_holders is increasing
        if max_holders < config.max_holders {
            return Err(ContractError::InvalidMaxHoldersAttemptedDecrease {});
        }

        config.max_holders = max_holders;
    }

    if let Some(max_tickets_per_depositor) = max_tickets_per_depositor {
        config.max_tickets_per_depositor = max_tickets_per_depositor;
    }

    if let Some(paused) = paused {
        if !paused {
            // Make sure that there isn't any old data left if you are unpausing
            let old_depositors = old_read_depositors(deps.as_ref(), None, Some(1))?;
            if !old_depositors.is_empty() {
                return Err(ContractError::Std(StdError::generic_err(
                    "Cannot unpause contract with old depositors",
                )));
            }
        }
        config.paused = paused;
    }

    CONFIG.save(deps.storage, &config)?;

    let mut state = STATE.load(deps.storage)?;

    if let Some(operator_glow_emission_rate) = operator_glow_emission_rate {
        state.operator_reward_emission_index.glow_emission_rate = operator_glow_emission_rate;
    }

    if let Some(sponsor_glow_emission_rate) = sponsor_glow_emission_rate {
        state.sponsor_reward_emission_index.glow_emission_rate = sponsor_glow_emission_rate;
    }

    STATE.save(deps.storage, &state)?;

    Ok(Response::new().add_attributes(vec![("action", "update_config")]))
}

pub fn execute_update_lottery_config(
    deps: DepsMut,
    info: MessageInfo,
    ticket_price: Option<Uint256>,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;

    // Check permission
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    if let Some(ticket_price) = ticket_price {
        config.ticket_price = ticket_price;
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
        QueryMsg::TicketInfo { sequence } => to_binary(&query_ticket_info(deps, sequence)?),
        QueryMsg::DepositorInfo { address } => {
            to_binary(&query_depositor_info(deps, env, address)?)
        }
        QueryMsg::DepositorStatsInfo { address } => {
            to_binary(&query_depositor_stats(deps, env, address)?)
        }
        QueryMsg::DepositorInfos { start_after, limit } => {
            to_binary(&query_depositors_info(deps, start_after, limit)?)
        }
        QueryMsg::DepositorsStatsInfos { start_after, limit } => {
            to_binary(&query_depositors_stats(deps, start_after, limit)?)
        }
        QueryMsg::Sponsor { address } => to_binary(&query_sponsor(deps, env, address)?),
        QueryMsg::Operator { address } => to_binary(&query_operator(deps, env, address)?),
        QueryMsg::AmountRedeemableForPrizes {} => {
            to_binary(&query_amount_redeemable_for_prizes(deps, env)?)
        }
        QueryMsg::OldLotteryInfo { lottery_id } => {
            to_binary(&query_old_lottery_info(deps, env, lottery_id)?)
        }
        QueryMsg::OldPrizeInfos { start_after, limit } => {
            to_binary(&query_old_prize_infos(deps, env, start_after, limit)?)
        }
    }
}

pub fn query_ticket_info(deps: Deps, ticket: String) -> StdResult<TicketInfoResponse> {
    let holders = TICKETS
        .may_load(deps.storage, ticket.as_ref())?
        .unwrap_or_default();
    Ok(TicketInfoResponse { holders })
}

pub fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;

    Ok(ConfigResponse {
        owner: config.owner.to_string(),
        stable_denom: config.stable_denom,
        a_terra_contract: config.a_terra_contract.to_string(),
        anchor_contract: config.anchor_contract.to_string(),
        gov_contract: config.gov_contract.to_string(),
        ve_contract: config.ve_contract.to_string(),
        community_contract: config.community_contract.to_string(),
        distributor_contract: config.distributor_contract.to_string(),
        ticket_price: config.ticket_price,
        max_holders: config.max_holders,
        split_factor: config.split_factor,
        instant_withdrawal_fee: config.instant_withdrawal_fee,
        unbonding_period: config.unbonding_period,
        max_tickets_per_depositor: config.max_tickets_per_depositor,
        paused: config.paused,
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

    if block_height < state.operator_reward_emission_index.last_reward_updated
        || block_height < state.sponsor_reward_emission_index.last_reward_updated
    {
        return Err(StdError::generic_err(
            "Block_height must be greater than both operator and sponsor last_reward_updated",
        ));
    }

    // Compute global operator reward index
    compute_global_operator_reward(&mut state, &pool, block_height);
    // Compute global sponsor reward index
    compute_global_sponsor_reward(&mut state, &pool, block_height);

    Ok(StateResponse {
        total_tickets: state.total_tickets,
        operator_reward_emission_index: state.operator_reward_emission_index,
        sponsor_reward_emission_index: state.sponsor_reward_emission_index,
        last_lottery_execution_aust_exchange_rate: state.last_lottery_execution_aust_exchange_rate,
    })
}

pub fn query_pool(deps: Deps) -> StdResult<PoolResponse> {
    let pool = POOL.load(deps.storage)?;

    Ok(PoolResponse {
        total_user_shares: pool.total_user_shares,
        total_user_aust: pool.total_user_aust,
        total_sponsor_lottery_deposits: pool.total_sponsor_lottery_deposits,
        total_operator_shares: pool.total_operator_shares,
    })
}

pub fn query_old_lottery_info(
    deps: Deps,
    _env: Env,
    lottery_id: Option<u64>,
) -> StdResult<OldLotteryInfoResponse> {
    let (lottery_id, lottery) = if let Some(lottery_id) = lottery_id {
        (
            lottery_id,
            old_read_lottery_info(deps.storage, lottery_id)?.unwrap(),
        )
    } else {
        let lottery_id = OLDSTATE.load(deps.storage)?.current_lottery;
        (
            lottery_id,
            old_read_lottery_info(deps.storage, lottery_id)?.unwrap(),
        )
    };
    Ok(OldLotteryInfoResponse {
        lottery_id,
        rand_round: lottery.rand_round,
        sequence: lottery.sequence,
        awarded: lottery.awarded,
        prize_buckets: lottery.prize_buckets,
        number_winners: lottery.number_winners,
        page: lottery.page,
    })
}

pub fn query_old_prize_infos(
    deps: Deps,
    _env: Env,
    start_after: Option<(String, u64)>,
    limit: Option<u32>,
) -> StdResult<OldPrizeInfosResponse> {
    let old_prizes = old_read_prize_infos(deps, start_after, limit)?;

    Ok(OldPrizeInfosResponse { prizes: old_prizes })
}

pub fn query_depositor_info(
    deps: Deps,
    _env: Env,
    addr: String,
) -> StdResult<DepositorInfoResponse> {
    let address = deps.api.addr_validate(&addr)?;
    let depositor = read_depositor_info(deps.storage, &address);

    Ok(DepositorInfoResponse {
        depositor: addr,
        shares: depositor.shares,
        tickets: depositor.tickets,
        unbonding_info: depositor.unbonding_info,
    })
}

pub fn query_depositor_stats(
    deps: Deps,
    _env: Env,
    addr: String,
) -> StdResult<DepositorStatsResponse> {
    let address = deps.api.addr_validate(&addr)?;
    let depositor_stats_info = read_depositor_stats(deps.storage, &address);

    Ok(DepositorStatsResponse {
        depositor: addr,
        shares: depositor_stats_info.shares,
        num_tickets: depositor_stats_info.num_tickets,
    })
}

pub fn query_sponsor(deps: Deps, env: Env, addr: String) -> StdResult<SponsorInfoResponse> {
    let address = deps.api.addr_validate(&addr)?;
    let mut sponsor = read_sponsor_info(deps.storage, &address);

    let mut state = STATE.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;

    // compute rewards
    compute_global_sponsor_reward(&mut state, &pool, env.block.height);
    compute_sponsor_reward(&state, &mut sponsor);

    Ok(SponsorInfoResponse {
        sponsor: addr,
        lottery_deposit: sponsor.lottery_deposit,
        reward_index: sponsor.reward_index,
        pending_rewards: sponsor.pending_rewards,
    })
}

pub fn query_operator(deps: Deps, env: Env, addr: String) -> StdResult<OperatorInfoResponse> {
    let address = deps.api.addr_validate(&addr)?;
    let mut operator = read_operator_info(deps.storage, &address);

    let mut state = STATE.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;

    // compute rewards
    compute_global_operator_reward(&mut state, &pool, env.block.height);
    compute_operator_reward(&state, &mut operator);

    Ok(OperatorInfoResponse {
        operator: addr,
        shares: operator.shares,
        reward_index: operator.reward_index,
        pending_rewards: operator.pending_rewards,
    })
}

pub fn query_depositors_info(
    deps: Deps,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<DepositorsInfoResponse> {
    let start_after = if let Some(start_after) = start_after {
        Some(deps.api.addr_validate(&start_after)?)
    } else {
        None
    };

    let depositors = read_depositors_info(deps, start_after, limit)?;
    Ok(DepositorsInfoResponse { depositors })
}

pub fn query_depositors_stats(
    deps: Deps,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<DepositorsStatsResponse> {
    let start_after = if let Some(start_after) = start_after {
        Some(deps.api.addr_validate(&start_after)?)
    } else {
        None
    };

    let depositors = read_depositors_stats(deps, start_after, limit)?;
    Ok(DepositorsStatsResponse { depositors })
}

pub fn query_amount_redeemable_for_prizes(
    deps: Deps,
    env: Env,
) -> StdResult<AmountRedeemableForPrizesResponse> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let state = STATE.load(deps.storage)?;

    // Get the contract's aust balance
    let contract_a_balance = Uint256::from(query_token_balance(
        &deps.querier,
        config.a_terra_contract.clone(),
        env.clone().contract.address,
    )?);

    // Get the aust exchange rate
    let aust_exchange_rate =
        query_exchange_rate(deps, config.anchor_contract.to_string(), env.block.height)?
            .exchange_rate;

    let amount_redeemable_for_prizes = calculate_value_of_aust_to_be_redeemed_for_lottery(
        &state,
        &pool,
        &config,
        contract_a_balance,
        aust_exchange_rate,
    );

    Ok(AmountRedeemableForPrizesResponse {
        amount_redeemable_for_prizes,
    })
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, env: Env, msg: MigrateMsg) -> Result<Response, ContractError> {
    // Migration Notes
    // The changes to storage:
    // - CONFIG (reuses storage key)
    // - LOTTERIES (new storage key)
    // - PRIZES (new storage key)
    // - DEPOSITORS (new storage key, paginated migration)
    // - STATE (reuses storage key)
    // - POOL (reuses storage key)

    // Read old storage
    let old_config = OLDCONFIG.load(deps.as_ref().storage)?;
    let mut old_state = OLDSTATE.load(deps.storage)?;
    let old_pool = OLDPOOL.load(deps.as_ref().storage)?;

    let default_lotto_winner_boost_config: BoostConfig = BoostConfig {
        base_multiplier: Decimal256::from_ratio(40u64, 100u64),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(150),
    };

    let _lotto_winner_boost_config =
        if let Some(msg_lotto_winner_boost_config) = msg.lotto_winner_boost_config {
            if msg_lotto_winner_boost_config.base_multiplier
                > msg_lotto_winner_boost_config.max_multiplier
                || msg_lotto_winner_boost_config.total_voting_power_weight == Decimal256::zero()
            {
                return Err(ContractError::InvalidBoostConfig {});
            }
            msg_lotto_winner_boost_config
        } else {
            default_lotto_winner_boost_config
        };

    let _lottery_interval_seconds = if let Duration::Time(time) = old_config.lottery_interval {
        time
    } else {
        return Err(ContractError::Std(StdError::generic_err(
            "Invalid lottery interval",
        )));
    };

    let new_config = Config {
        owner: old_config.owner,
        a_terra_contract: old_config.a_terra_contract,
        gov_contract: old_config.gov_contract,
        ve_contract: deps.api.addr_validate(msg.ve_contract.as_str())?,
        community_contract: deps.api.addr_validate(msg.community_contract.as_str())?,
        distributor_contract: old_config.distributor_contract,
        prize_distributor_contract: deps
            .api
            .addr_validate(msg.prize_distributor_contract.as_str())?,
        oracle_contract: old_config.oracle_contract,
        stable_denom: old_config.stable_denom,
        anchor_contract: old_config.anchor_contract,
        ticket_price: old_config.ticket_price,
        max_holders: old_config.max_holders,
        split_factor: old_config.split_factor,
        instant_withdrawal_fee: old_config.instant_withdrawal_fee,
        unbonding_period: old_config.unbonding_period,
        max_tickets_per_depositor: msg.max_tickets_per_depositor,
        paused: true,
    };

    CONFIG.save(deps.storage, &new_config)?;

    // Query exchange_rate from anchor money market
    let aust_exchange_rate: Decimal256 = query_exchange_rate(
        deps.as_ref(),
        deps.api
            .addr_validate(new_config.anchor_contract.as_str())?
            .to_string(),
        env.block.height,
    )?
    .exchange_rate;

    old_compute_reward(&mut old_state, &old_pool, env.block.height);

    let _next_lottery_time =
        if let Expiration::AtTime(next_lottery_time) = old_state.next_lottery_time {
            next_lottery_time
        } else {
            return Err(ContractError::Std(StdError::generic_err(
                "invalid lottery next time",
            )));
        };

    let state = State {
        total_tickets: old_state.total_tickets,
        operator_reward_emission_index: RewardEmissionsIndex {
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: msg.operator_glow_emission_rate,
            last_reward_updated: env.block.height,
        },
        sponsor_reward_emission_index: RewardEmissionsIndex {
            global_reward_index: old_state.global_reward_index,
            glow_emission_rate: msg.sponsor_glow_emission_rate,
            last_reward_updated: old_state.last_reward_updated,
        },
        last_lottery_execution_aust_exchange_rate: aust_exchange_rate,
    };

    STATE.save(deps.storage, &state)?;

    // Migrate pool
    // Initially total_user_aust and total_user_shares are set to 0
    // But they are updated in the migrate_old_depositors section of the loop
    let new_pool = Pool {
        total_user_aust: Uint256::zero(),
        total_user_shares: Uint256::zero(),
        total_sponsor_lottery_deposits: old_pool.total_sponsor_lottery_deposits,
        total_operator_shares: Uint256::zero(),
    };

    POOL.save(deps.storage, &new_pool)?;

    Ok(Response::default())
}

pub fn migrate_old_depositors(
    deps: DepsMut,
    env: Env,
    limit: Option<u32>,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    let state = STATE.load(deps.storage)?;

    let aust_exchange_rate: Decimal256 = query_exchange_rate(
        deps.as_ref(),
        deps.api
            .addr_validate(config.anchor_contract.as_str())?
            .to_string(),
        env.block.height,
    )?
    .exchange_rate;

    let old_depositors = old_read_depositors(deps.as_ref(), None, limit)?;

    let mut num_migrated_entries: u32 = 0;

    let mut pool = POOL.load(deps.storage)?;

    let mut msgs: Vec<CosmosMsg> = vec![];

    for (addr, mut old_depositor_info) in old_depositors {
        // Update depositor reward and append message to send pending rewards
        old_compute_depositor_reward(
            state.sponsor_reward_emission_index.global_reward_index,
            &mut old_depositor_info,
        );
        let claim_amount = old_depositor_info.pending_rewards * Uint256::one();
        if !claim_amount.is_zero() {
            msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.distributor_contract.to_string(),
                funds: vec![],
                msg: to_binary(&FaucetExecuteMsg::Spend {
                    recipient: addr.to_string(),
                    amount: claim_amount.into(),
                })?,
            }));
        }

        // Delete old depositor
        old_remove_depositor_info(deps.storage, &addr);

        // Get the depositors balance, add the value of the savings aust with the lottery_deposit
        // Then at the end there will be some left over aust.
        // This will be captured by the sponsors.
        let depositor_aust_balance = old_depositor_info.savings_aust
            + old_depositor_info.lottery_deposit / aust_exchange_rate;

        let new_depositor_info = DepositorInfo {
            shares: depositor_aust_balance,
            tickets: old_depositor_info.tickets,
            unbonding_info: old_depositor_info.unbonding_info,
            operator_addr: Addr::unchecked(""),
        };

        pool.total_user_shares += depositor_aust_balance;
        pool.total_user_aust += depositor_aust_balance;

        // Store new depositor
        store_depositor_info(deps.storage, &addr, new_depositor_info, env.block.height)?;

        // Increment num_migrates_entries
        num_migrated_entries += 1;
    }

    let old_depositors = old_read_depositors(deps.as_ref(), None, Some(1))?;
    if old_depositors.is_empty() {
        // Migrate lottery info

        // TODO Handle migration to prize distributor contract
        // // Don't need to include state.current_lottery
        // // because nothing has been saved with id state.current_lottery yet
        // for i in 0..state.current_lottery {
        //     let old_lottery_info = old_read_lottery_info(deps.storage, i)?;

        //     if let Some(old_lottery_info) = old_lottery_info {
        //         let new_lottery_info = LotteryInfo {
        //             rand_round: old_lottery_info.rand_round,
        //             sequence: old_lottery_info.sequence,
        //             awarded: old_lottery_info.awarded,
        //             timestamp: Timestamp::from_seconds(0),
        //             prize_buckets: old_lottery_info.prize_buckets,
        //             number_winners: old_lottery_info.number_winners,
        //             page: old_lottery_info.page,
        //             glow_prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
        //             block_height: old_lottery_info.timestamp,
        //             total_user_shares: pool.total_user_shares,
        //         };

        //         store_lottery_info(deps.storage, i, &new_lottery_info)?;

        //         old_remove_lottery_info(deps.storage, i);
        //     } else {
        //         return Err(ContractError::Std(StdError::generic_err(
        //             "Already migrated depositors and lotteries",
        //         )));
        //     }
        // }

        // Set paused to false and save
        config.paused = false;
        CONFIG.save(deps.storage, &config)?;
    }

    POOL.save(deps.storage, &pool)?;

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "migrate_old_depositors"),
        attr("num_migrated_entries", num_migrated_entries.to_string()),
    ]))
}

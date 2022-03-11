#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use crate::error::ContractError;
use crate::helpers::{
    base64_encoded_tickets_to_vec_string_tickets,
    calculate_value_of_aust_to_be_redeemed_for_lottery, calculate_winner_prize,
    claim_unbonded_withdrawals, compute_global_operator_reward, compute_global_sponsor_reward,
    compute_operator_reward, compute_sponsor_reward, decimal_from_ratio_or_one,
    handle_depositor_operator_updates, is_valid_sequence, pseudo_random_seq,
    ExecuteLotteryRedeemedAustInfo,
};
use crate::prize_strategy::{execute_lottery, execute_prize};
use crate::querier::{query_balance, query_exchange_rate};
use crate::state::{
    old_read_depositors, old_read_lottery_info, old_remove_depositor_info, old_remove_lottery_info,
    parse_length, read_depositor_info, read_depositor_stats, read_depositor_stats_at_height,
    read_depositors_info, read_depositors_stats, read_lottery_info, read_lottery_prizes,
    read_operator_info, read_sponsor_info, store_depositor_info, store_lottery_info,
    store_operator_info, store_sponsor_info, Config, DepositorInfo, LotteryInfo, OperatorInfo,
    Pool, PrizeInfo, SponsorInfo, State, CONFIG, OLDCONFIG, OLDPOOL, OLDSTATE, OLD_PRIZES, POOL,
    PRIZES, STATE, TICKETS,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, coin, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo,
    Order, Response, StdError, StdResult, Timestamp, Uint128, WasmMsg,
};
use cw0::{Duration, Expiration};
use cw20::Cw20ExecuteMsg;
use cw_storage_plus::U64Key;
use glow_protocol::distributor::ExecuteMsg as FaucetExecuteMsg;
use glow_protocol::lotto::{
    BoostConfig, Claim, ConfigResponse, DepositorInfoResponse, DepositorStatsResponse,
    DepositorsInfoResponse, DepositorsStatsResponse, ExecuteMsg, InstantiateMsg,
    LotteryBalanceResponse, LotteryInfoResponse, MigrateMsg, OperatorInfoResponse, PoolResponse,
    PrizeInfoResponse, PrizeInfosResponse, QueryMsg, RewardEmissionsIndex, SponsorInfoResponse,
    StateResponse, TicketInfoResponse,
};
use glow_protocol::lotto::{NUM_PRIZE_BUCKETS, TICKET_LENGTH};
use glow_protocol::querier::deduct_tax;
use moneymarket::market::{Cw20HookMsg, EpochStateResponse, ExecuteMsg as AnchorMsg};
use std::ops::{Add, Sub};
use std::str::from_utf8;
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

    // Validate prize distribution
    if msg.prize_distribution.len() != NUM_PRIZE_BUCKETS {
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

    // Validate that max_holders is within the bounds
    if msg.max_holders < MAX_HOLDERS_FLOOR || MAX_HOLDERS_CAP < msg.max_holders {
        return Err(ContractError::InvalidMaxHoldersOutsideBounds {});
    }

    let default_lotto_winner_boost_config: BoostConfig = BoostConfig {
        base_multiplier: Decimal256::from_ratio(Uint256::from(40u128), Uint256::from(100u128)),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(150),
    };

    let lotto_winner_boost_config =
        if let Some(msg_lotto_winner_boost_config) = msg.lotto_winner_boost_config {
            if msg_lotto_winner_boost_config.base_multiplier
                > msg_lotto_winner_boost_config.max_multiplier
            {
                return Err(ContractError::InvalidBoostConfig {});
            }
            msg_lotto_winner_boost_config
        } else {
            default_lotto_winner_boost_config
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
            max_tickets_per_depositor: msg.max_tickets_per_depositor,
            glow_prize_buckets: msg.glow_prize_buckets,
            paused: false,
            lotto_winner_boost_config,
        },
    )?;

    // Validate first lottery is in the future
    if msg.initial_lottery_execution <= env.block.time.seconds() {
        return Err(ContractError::InvalidFirstLotteryExec {});
    }

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
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(
                msg.initial_lottery_execution,
            )),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: Duration::Time(msg.epoch_interval).after(&env.block),
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
        reserve_factor,
        instant_withdrawal_fee,
        unbonding_period,
        epoch_interval,
        max_holders,
        max_tickets_per_depositor,
        paused,
        lotto_winner_boost_config,
        operator_glow_emission_rate,
        sponsor_glow_emission_rate,
    } = msg
    {
        return execute_update_config(
            deps,
            info,
            owner,
            oracle_addr,
            reserve_factor,
            instant_withdrawal_fee,
            unbonding_period,
            epoch_interval,
            max_holders,
            max_tickets_per_depositor,
            paused,
            lotto_winner_boost_config,
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
        } => execute_register_contracts(
            deps,
            info,
            gov_contract,
            community_contract,
            distributor_contract,
            ve_contract,
        ),
        ExecuteMsg::Deposit {
            encoded_tickets,
            operator,
        } => execute_deposit(deps, env, info, encoded_tickets, operator),
        ExecuteMsg::Gift {
            encoded_tickets,
            recipient,
            operator,
        } => execute_gift(deps, env, info, encoded_tickets, recipient, operator),
        ExecuteMsg::Sponsor {
            award,
            prize_distribution,
        } => execute_sponsor(deps, env, info, award, prize_distribution),
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
            max_holders,
            max_tickets_per_depositor,
            paused,
            lotto_winner_boost_config,
            operator_glow_emission_rate,
            sponsor_glow_emission_rate,
        } => execute_update_config(
            deps,
            info,
            owner,
            oracle_addr,
            reserve_factor,
            instant_withdrawal_fee,
            unbonding_period,
            epoch_interval,
            max_holders,
            max_tickets_per_depositor,
            paused,
            lotto_winner_boost_config,
            operator_glow_emission_rate,
            sponsor_glow_emission_rate,
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
        ExecuteMsg::MigrateOldDepositors { .. } => Err(ContractError::Std(StdError::generic_err(
            "Cannot call MigrateLoop when unpaused.",
        ))),
    }
}

pub fn execute_register_contracts(
    deps: DepsMut,
    info: MessageInfo,
    gov_contract: String,
    community_contract: String,
    distributor_contract: String,
    ve_contract: String,
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

    // Get combinations from encoded tickets
    let combinations = base64_encoded_tickets_to_vec_string_tickets(encoded_tickets)?;

    // Get the depositor info
    // depositor being either the message sender
    // or the recipient that will be reciving the deposited funds if specified
    let depositor = if let Some(recipient) = recipient.clone() {
        deps.api.addr_validate(recipient.as_str())?
    } else {
        info.sender.clone()
    };
    let mut depositor_info: DepositorInfo = read_depositor_info(deps.storage, &depositor);

    // Get the amount of requested tickets
    let mut number_of_new_tickets = combinations.len() as u64;

    // Validate that the deposit amount is non zero
    if deposit_amount.is_zero() {
        return if recipient.is_some() {
            Err(ContractError::ZeroGiftAmount {})
        } else {
            Err(ContractError::ZeroDepositAmount {})
        };
    }

    // Validate that all sequence combinations are valid
    for combination in combinations.clone() {
        if !is_valid_sequence(&combination, TICKET_LENGTH) {
            return Err(ContractError::InvalidSequence(combination));
        }
    }

    // Validate that the lottery has not already started
    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    // deduct tx taxes when calculating the net deposited amount in anchor
    let net_coin_amount = deduct_tax(
        deps.as_ref(),
        coin(deposit_amount.into(), config.stable_denom.clone()),
    )?;

    let post_tax_deposit_amount = Uint256::from(net_coin_amount.amount);

    // Get the number of minted aust
    let minted_aust = post_tax_deposit_amount / aust_exchange_rate;

    // Get the amount of minted_shares
    let minted_shares =
        minted_aust * decimal_from_ratio_or_one(pool.total_user_shares, pool.total_user_aust);

    let post_transaction_depositor_shares = depositor_info.shares + minted_shares;

    let post_transaction_depositor_balance = (pool.total_user_aust + minted_aust)
        * Decimal256::from_ratio(
            post_transaction_depositor_shares,
            pool.total_user_shares + minted_shares,
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
    .u128() as u64;

    // Get the number of tickets the user would have post transaction (without accounting for round up)
    let mut post_transaction_num_depositor_tickets =
        (depositor_info.tickets.len() + number_of_new_tickets as usize) as u64;

    // Check if we need to round up the number of combinations based on the depositor's mixed_tax_post_transaction_lottery_deposit
    let mut new_combinations = combinations;
    for _ in 0..100 {
        if post_transaction_max_depositor_tickets <= post_transaction_num_depositor_tickets {
            break;
        }

        let current_time = env.block.time.nanos();
        let sequence = pseudo_random_seq(
            info.sender.clone().into_string(),
            post_transaction_num_depositor_tickets,
            current_time,
        );

        // Add the randomly generated sequence to new_combinations
        new_combinations.push(sequence);
        // Increment number_of_new_tickets and post_transaction_num_depositor_tickets
        number_of_new_tickets += 1;
        post_transaction_num_depositor_tickets += 1;
    }

    // Validate that the post_transaction_max_depositor_tickets is less than or equal to the post_transaction_num_depositor_tickets
    if post_transaction_num_depositor_tickets > post_transaction_max_depositor_tickets {
        return Err(ContractError::InsufficientPostTransactionDepositorBalance {
            post_transaction_depositor_balance,
            post_transaction_num_depositor_tickets,
            post_transaction_max_depositor_tickets,
        });
    }

    // Validate that the depositor won't go over max_tickets_per_depositor
    if post_transaction_num_depositor_tickets > config.max_tickets_per_depositor {
        return Err(ContractError::MaxTicketsPerDepositorExceeded {
            max_tickets_per_depositor: config.max_tickets_per_depositor,
            post_transaction_num_depositor_tickets,
        });
    }

    for combination in new_combinations {
        // check that the number of holders for any given ticket isn't too high
        if let Some(holders) = TICKETS
            .may_load(deps.storage, combination.as_bytes())
            .unwrap()
        {
            if holders.len() >= config.max_holders as usize {
                return Err(ContractError::InvalidHolderSequence(combination));
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

    // update depositor and state information
    store_depositor_info(deps.storage, &depositor, depositor_info, env.block.height)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    // save depositor and state information
    Ok(Response::new()
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
    award: Option<bool>,
    prize_distribution: Option<[Decimal256; NUM_PRIZE_BUCKETS]>,
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
        return Err(ContractError::ZeroSponsorshipAmount {});
    }

    compute_global_sponsor_reward(&mut state, &pool, env.block.height);

    let mut msgs: Vec<CosmosMsg> = vec![];

    if let None | Some(false) = award {
        // Deduct taxes that will be payed when transferring to anchor
        let net_sponsor_amount = Uint256::from(
            deduct_tax(
                deps.as_ref(),
                coin(sponsor_amount.into(), config.stable_denom.clone()),
            )?
            .amount,
        );

        // query exchange_rate from anchor money market
        let epoch_state: EpochStateResponse = query_exchange_rate(
            deps.as_ref(),
            config.anchor_contract.to_string(),
            env.block.height,
        )?;

        // add amount of aUST entitled from the deposit
        let minted_aust = net_sponsor_amount / epoch_state.exchange_rate;

        // Get minted_aust_value
        let minted_aust_value = minted_aust * epoch_state.exchange_rate;

        // fetch sponsor_info
        let mut sponsor_info: SponsorInfo = read_sponsor_info(deps.storage, &info.sender);

        // update sponsor sponsor rewards
        compute_sponsor_reward(&state, &mut sponsor_info);

        // add sponsor_amount to depositor
        sponsor_info.lottery_deposit = sponsor_info.lottery_deposit.add(minted_aust_value);
        store_sponsor_info(deps.storage, &info.sender, sponsor_info)?;

        // update pool
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
    } else {
        // Award is instant

        // Get the prize_distribution or the prize_distribution in the config
        let prize_distribution = prize_distribution.unwrap_or(config.prize_distribution);

        // Validate that the prize_distribution is of length NUM_PRIZE_BUCKETS
        if prize_distribution.len() != NUM_PRIZE_BUCKETS {
            return Err(ContractError::InvalidPrizeDistribution {});
        }

        // Validate that the prize_distributions sums to 1
        let mut sum = Decimal256::zero();
        for item in prize_distribution.iter() {
            sum += *item;
        }

        if sum != Decimal256::one() {
            return Err(ContractError::InvalidPrizeDistribution {});
        }

        // Distribute the sponsorship to the prize buckets according to the prize distribution
        for (index, fraction_of_prize) in prize_distribution.iter().enumerate() {
            // Add the proportional amount of the net redeemed amount to the relevant award bucket.
            state.prize_buckets[index] += sponsor_amount * *fraction_of_prize
        }
    }

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

    let mut sponsor_info: SponsorInfo = read_sponsor_info(deps.storage, &info.sender);

    // Validate that the sponsor has a lottery deposit
    if sponsor_info.lottery_deposit.is_zero() {
        return Err(ContractError::NoSponsorLotteryDeposit {});
    }

    // Validate that there isn't a lottery in progress
    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    // Compute Glow depositor rewards
    compute_global_sponsor_reward(&mut state, &pool, env.block.height);
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

    // Validate that the user has savings aust to withdraw
    if depositor_info.shares.is_zero() {
        return Err(ContractError::NoDepositorSavingsAustToWithdraw {});
    }

    // Validate that the user is withdrawing a non zero amount
    if (amount.is_some()) && (amount.unwrap().is_zero()) {
        return Err(ContractError::SpecifiedWithdrawAmountIsZero {});
    }

    // Validate that there isn't a lottery in progress already
    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    // Calculate the depositor's balance from their aust balance
    let depositor_balance = pool.total_user_aust
        * Decimal256::from_ratio(depositor_info.shares, pool.total_user_shares)
        * aust_exchange_rate;

    let aust_amount = amount.map(|v| Uint256::from(v) / aust_exchange_rate);

    // Get the number of withdrawn shares
    let withdrawn_shares = aust_amount
        .map(|v| {
            std::cmp::max(
                v.multiply_ratio(pool.total_user_shares, pool.total_user_aust),
                Uint256::one(),
            )
        })
        .unwrap_or_else(|| depositor_info.shares);

    // Get the withdrawn amount
    let withdrawn_aust =
        withdrawn_shares.multiply_ratio(pool.total_user_aust, pool.total_user_shares);

    let withdrawn_aust_value = withdrawn_aust * aust_exchange_rate;

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
        post_transaction_depositor_balance / Decimal256::from_uint256(config.ticket_price),
    )
    .u128();

    // Calculate how many tickets to remove
    let num_depositor_tickets = depositor_info.tickets.len() as u128;

    // Get the number of tickets to withdraw
    let withdrawn_tickets: u128 = num_depositor_tickets
        .checked_sub(post_transaction_max_depositor_tickets)
        .unwrap_or_default();

    if withdrawn_tickets > num_depositor_tickets {
        return Err(ContractError::WithdrawingTooManyTickets {
            withdrawn_tickets,
            num_depositor_tickets,
        });
    }

    for seq in depositor_info.tickets.drain(..withdrawn_tickets as usize) {
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

        // update the glow reward index
        compute_global_operator_reward(&mut state, &pool, env.block.height);
        // update the glow depositor reward for the depositor
        compute_operator_reward(&state, &mut operator);

        // Add new deposit amount
        operator.shares = operator.shares.sub(withdrawn_shares);

        // Store new operator info
        store_operator_info(deps.storage, &depositor_info.operator_addr, operator)?;

        pool.total_operator_shares = pool.total_operator_shares.sub(withdrawn_shares);
    }

    // Update depositor info

    depositor_info.shares = depositor_info.shares.sub(withdrawn_shares);

    // Update pool

    pool.total_user_shares = pool.total_user_shares.sub(withdrawn_shares);
    pool.total_user_aust = pool.total_user_aust.sub(withdrawn_aust);

    // Remove withdrawn_tickets from total_tickets
    state.total_tickets = state.total_tickets.sub(Uint256::from(withdrawn_tickets));

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

        // Add the withdrawal fee to the total_reserve
        state.total_reserve += withdrawal_fee;

        // Get the amount of ust to return after tax
        let net_coin_amount = deduct_tax(
            deps.as_ref(),
            coin(return_amount.into(), config.stable_denom),
        )?;

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

    store_depositor_info(deps.storage, &info.sender, depositor_info, env.block.height)?;
    STATE.save(deps.storage, &state)?;
    POOL.save(deps.storage, &pool)?;

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "withdraw_ticket"),
        attr("depositor", info.sender.to_string()),
        attr("tickets_amount", withdrawn_tickets.to_string()),
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

    let mut depositor = read_depositor_info(deps.storage, &info.sender);

    let to_send = claim_unbonded_withdrawals(&mut depositor, &env.block, None)?;

    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

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

    let reserved_for_prizes = state
        .prize_buckets
        .iter()
        .fold(Uint256::zero(), |sum, val| sum + *val);

    if to_send > (balance - reserved_for_prizes).into() {
        return Err(ContractError::InsufficientFunds {
            to_send,
            available_balance: balance - reserved_for_prizes,
        });
    }

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

// Send available UST to user from prizes won in the given lottery_id
pub fn execute_claim_lottery(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    lottery_ids: Vec<u64>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let state = STATE.load(deps.storage)?;

    let mut ust_to_send = Uint128::zero();
    let mut glow_to_send = Uint128::zero();

    let current_lottery = read_lottery_info(deps.storage, state.current_lottery);
    if current_lottery.rand_round != 0 {
        return Err(ContractError::LotteryAlreadyStarted {});
    }

    for lottery_id in lottery_ids.clone() {
        let lottery_info = read_lottery_info(deps.storage, lottery_id);
        if !lottery_info.awarded {
            return Err(ContractError::InvalidClaimLotteryNotAwarded(lottery_id));
        }
        //Calculate and add to to_send
        let lottery_key: U64Key = U64Key::from(lottery_id);
        let prize = PRIZES
            .may_load(deps.storage, (lottery_key.clone(), &info.sender))
            .unwrap();
        if let Some(prize) = prize {
            if prize.claimed {
                return Err(ContractError::InvalidClaimPrizeAlreadyClaimed(lottery_id));
            }

            let snapshotted_depositor_stats_info = read_depositor_stats_at_height(
                deps.storage,
                &info.sender,
                lottery_info.block_height,
            );

            let (local_ust_to_send, local_glow_to_send): (Uint128, Uint128) =
                calculate_winner_prize(
                    &deps.querier,
                    &config,
                    &prize,
                    &lottery_info,
                    &snapshotted_depositor_stats_info,
                    &info.sender,
                )?;

            ust_to_send += local_ust_to_send;
            glow_to_send += local_glow_to_send;

            PRIZES.save(
                deps.storage,
                (lottery_key, &info.sender),
                &PrizeInfo {
                    claimed: true,
                    ..prize
                },
            )?;
        }
    }

    // If ust_to_send is zero, don't send anything even if glow_to_send is positive.
    // It should never be the case that ust_to_send is 0 and glow_to_send is positive.
    if ust_to_send == Uint128::zero() {
        return Err(ContractError::InsufficientClaimableFunds {});
    }

    let mut msgs: Vec<CosmosMsg> = vec![];

    // ust_to_send calculations

    // Deduct taxes on the claim
    let net_send = deduct_tax(
        deps.as_ref(),
        coin(ust_to_send.into(), config.stable_denom.clone()),
    )?
    .amount;

    // Double-check if there is enough balance to send in the contract
    let balance = query_balance(
        deps.as_ref(),
        env.contract.address.to_string(),
        config.stable_denom.clone(),
    )?;

    if ust_to_send > balance.into() {
        return Err(ContractError::InsufficientFunds {
            to_send: ust_to_send,
            available_balance: balance,
        });
    }

    msgs.push(CosmosMsg::Bank(BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: vec![Coin {
            denom: config.stable_denom,
            amount: net_send,
        }],
    }));

    // glow_to_send calculations

    if glow_to_send != Uint128::zero() {
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.distributor_contract.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: info.sender.to_string(),
                amount: glow_to_send,
            })?,
        }));
    }

    // Update storage
    STATE.save(deps.storage, &state)?;

    // Send response

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "claim_lottery"),
        attr("lottery_ids", format!("{:?}", lottery_ids)),
        attr("depositor", info.sender.to_string()),
        attr("redeemed_ust", net_send),
        attr("redeemed_glow", glow_to_send),
    ]))
}

pub fn execute_epoch_ops(deps: DepsMut, env: Env) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let pool = POOL.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    // Validate distributor contract has already been registered
    if !config.contracts_registered() {
        return Err(ContractError::NotRegistered {});
    }

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
    compute_global_operator_reward(&mut state, &pool, env.block.height);
    compute_global_sponsor_reward(&mut state, &pool, env.block.height);

    // Compute total_reserves to fund gov contract
    let total_reserves = state.total_reserve;
    let messages: Vec<CosmosMsg> = if !total_reserves.is_zero() {
        vec![CosmosMsg::Bank(BankMsg::Send {
            to_address: config.community_contract.to_string(),
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
    state.total_reserve = Uint256::zero();
    STATE.save(deps.storage, &state)?;

    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("action", "execute_epoch_operations"),
        attr("total_reserves", total_reserves.to_string()),
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

    // Compute Glow depositor rewards
    compute_global_operator_reward(&mut state, &pool, env.block.height);
    compute_global_sponsor_reward(&mut state, &pool, env.block.height);
    compute_operator_reward(&state, &mut operator);
    compute_sponsor_reward(&state, &mut sponsor);

    let claim_amount = (operator.pending_rewards + sponsor.pending_rewards) * Uint256::one();
    sponsor.pending_rewards = Decimal256::zero();
    operator.pending_rewards = Decimal256::zero();
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
    reserve_factor: Option<Decimal256>,
    instant_withdrawal_fee: Option<Decimal256>,
    unbonding_period: Option<u64>,
    epoch_interval: Option<u64>,
    max_holders: Option<u8>,
    max_tickets_per_depositor: Option<u64>,
    paused: Option<bool>,
    lotto_winner_boost_config: Option<BoostConfig>,
    operator_glow_emission_rate: Option<Decimal256>,
    sponsor_glow_emission_rate: Option<Decimal256>,
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

    if let Some(max_holders) = max_holders {
        // Validate that max_holders is within the bounds
        if max_holders < MAX_HOLDERS_FLOOR || MAX_HOLDERS_CAP < max_holders {
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

    if let Some(lotto_winner_boost_config) = lotto_winner_boost_config {
        if lotto_winner_boost_config.base_multiplier > lotto_winner_boost_config.max_multiplier {
            return Err(ContractError::InvalidBoostConfig {});
        }
        config.lotto_winner_boost_config = lotto_winner_boost_config
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
    lottery_interval: Option<u64>,
    block_time: Option<u64>,
    ticket_price: Option<Uint256>,
    prize_distribution: Option<[Decimal256; NUM_PRIZE_BUCKETS]>,
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
        if prize_distribution.len() != NUM_PRIZE_BUCKETS {
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
        QueryMsg::LotteryPrizeInfos {
            lottery_id,
            start_after,
            limit,
        } => to_binary(&query_lottery_prizes(deps, lottery_id, start_after, limit)?),
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
        QueryMsg::LotteryBalance {} => to_binary(&query_lottery_balance(deps, env)?),
    }
}

pub fn query_ticket_info(deps: Deps, ticket: String) -> StdResult<TicketInfoResponse> {
    let holders = TICKETS
        .may_load(deps.storage, ticket.as_ref())?
        .unwrap_or_default();
    Ok(TicketInfoResponse { holders })
}

pub fn query_prizes(deps: Deps, address: String, lottery_id: u64) -> StdResult<PrizeInfoResponse> {
    // Get config
    let config = CONFIG.load(deps.storage)?;

    // Get lottery info
    let lottery_info = read_lottery_info(deps.storage, lottery_id);

    // Get prize info
    let lottery_key = U64Key::from(lottery_id);
    let addr = deps.api.addr_validate(&address)?;
    let prize_info =
        if let Some(prize_info) = PRIZES.may_load(deps.storage, (lottery_key, &addr))? {
            prize_info
        } else {
            return Err(StdError::generic_err(
                "No prize with the specified address and lottery id.",
            ));
        };

    // Get ust and glow to send
    let snapshotted_depositor_stats_info =
        read_depositor_stats_at_height(deps.storage, &addr, lottery_info.block_height);

    let (local_ust_to_send, local_glow_to_send): (Uint128, Uint128) = calculate_winner_prize(
        &deps.querier,
        &config,
        &prize_info,
        &lottery_info,
        &snapshotted_depositor_stats_info,
        &addr,
    )?;

    Ok(PrizeInfoResponse {
        holder: addr,
        lottery_id,
        claimed: prize_info.claimed,
        matches: prize_info.matches,
        won_ust: local_ust_to_send,
        won_glow: local_glow_to_send,
    })
}

pub fn query_lottery_prizes(
    deps: Deps,
    lottery_id: u64,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<PrizeInfosResponse> {
    let config = CONFIG.load(deps.storage)?;

    let addr = if let Some(s) = start_after {
        Some(deps.api.addr_validate(&s)?)
    } else {
        None
    };

    let lottery_info = read_lottery_info(deps.storage, lottery_id);

    let prize_infos = read_lottery_prizes(deps, lottery_id, addr, limit)?;

    let prize_info_responses = prize_infos
        .into_iter()
        .map(|(addr, prize_info)| {
            let snapshotted_depositor_stats_info =
                read_depositor_stats_at_height(deps.storage, &addr, lottery_info.block_height);

            let (local_ust_to_send, local_glow_to_send): (Uint128, Uint128) =
                calculate_winner_prize(
                    &deps.querier,
                    &config,
                    &prize_info,
                    &lottery_info,
                    &snapshotted_depositor_stats_info,
                    &addr,
                )?;

            Ok(PrizeInfoResponse {
                holder: addr,
                lottery_id,
                claimed: prize_info.claimed,
                matches: prize_info.matches,
                won_ust: local_ust_to_send,
                won_glow: local_glow_to_send,
            })
        })
        .collect::<StdResult<Vec<_>>>()?;

    Ok(PrizeInfosResponse {
        prize_infos: prize_info_responses,
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
        ve_contract: config.ve_contract.to_string(),
        community_contract: config.community_contract.to_string(),
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

    // Compute reward rate with given block height
    compute_global_operator_reward(&mut state, &pool, block_height);
    compute_global_sponsor_reward(&mut state, &pool, block_height);

    Ok(StateResponse {
        total_tickets: state.total_tickets,
        total_reserve: state.total_reserve,
        prize_buckets: state.prize_buckets,
        current_lottery: state.current_lottery,
        next_lottery_time: state.next_lottery_time,
        next_lottery_exec_time: state.next_lottery_exec_time,
        next_epoch: state.next_epoch,
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

pub fn query_lottery_info(
    deps: Deps,
    env: Env,
    lottery_id: Option<u64>,
) -> StdResult<LotteryInfoResponse> {
    let (lottery_id, lottery) = if let Some(lottery_id) = lottery_id {
        (lottery_id, read_lottery_info(deps.storage, lottery_id))
    } else {
        let lottery_id = query_state(deps, env, None)?.current_lottery;
        (lottery_id, read_lottery_info(deps.storage, lottery_id))
    };
    Ok(LotteryInfoResponse {
        lottery_id,
        rand_round: lottery.rand_round,
        sequence: lottery.sequence,
        awarded: lottery.awarded,
        timestamp: lottery.timestamp,
        block_height: lottery.block_height,
        glow_prize_buckets: lottery.glow_prize_buckets,
        prize_buckets: lottery.prize_buckets,
        number_winners: lottery.number_winners,
        page: lottery.page,
        total_user_lottery_deposits: lottery.total_user_shares,
    })
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

pub fn query_lottery_balance(deps: Deps, env: Env) -> StdResult<LotteryBalanceResponse> {
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

    let ExecuteLotteryRedeemedAustInfo {
        value_of_user_aust_to_be_redeemed_for_lottery,
        user_aust_to_redeem,
        value_of_sponsor_aust_to_be_redeemed_for_lottery,
        sponsor_aust_to_redeem,
        aust_to_redeem,
        aust_to_redeem_value,
    } = calculate_value_of_aust_to_be_redeemed_for_lottery(
        &state,
        &pool,
        &config,
        contract_a_balance,
        aust_exchange_rate,
    );

    Ok(LotteryBalanceResponse {
        value_of_user_aust_to_be_redeemed_for_lottery,
        user_aust_to_redeem,
        value_of_sponsor_aust_to_be_redeemed_for_lottery,
        sponsor_aust_to_redeem,
        aust_to_redeem,
        aust_to_redeem_value,
        prize_buckets: state.prize_buckets,
    })
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, env: Env, msg: MigrateMsg) -> StdResult<Response> {
    // Migration Notes
    // The changes to storage:
    // - CONFIG (reuses storage key)
    // - LOTTERIES (new storage key)
    // - PRIZES (new storage key)
    // - DEPOSITORS (new storage key, paginated migration)
    // - STATE (reuses storage key)
    // - POOL (reuses storage key)

    let default_lotto_winner_boost_config: BoostConfig = BoostConfig {
        base_multiplier: Decimal256::from_ratio(40u64, 100u64),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(150),
    };

    let lotto_winner_boost_config =
        if let Some(msg_lotto_winner_boost_config) = msg.lotto_winner_boost_config {
            if msg_lotto_winner_boost_config.base_multiplier
                > msg_lotto_winner_boost_config.max_multiplier
            {
                return Err(StdError::generic_err(
                    "boost config base multiplier must be less than max multiplier",
                ));
            }
            msg_lotto_winner_boost_config
        } else {
            default_lotto_winner_boost_config
        };

    // migrate config
    let old_config = OLDCONFIG.load(deps.as_ref().storage)?;
    let new_config = Config {
        owner: old_config.owner,
        a_terra_contract: old_config.a_terra_contract,
        gov_contract: old_config.gov_contract,
        ve_contract: deps.api.addr_validate(msg.ve_contract.as_str())?,
        community_contract: deps.api.addr_validate(msg.community_contract.as_str())?,
        distributor_contract: old_config.distributor_contract,
        oracle_contract: old_config.oracle_contract,
        stable_denom: old_config.stable_denom,
        anchor_contract: old_config.anchor_contract,
        lottery_interval: old_config.lottery_interval,
        epoch_interval: old_config.epoch_interval,
        block_time: old_config.block_time,
        round_delta: old_config.round_delta,
        ticket_price: old_config.ticket_price,
        max_holders: old_config.max_holders,
        prize_distribution: old_config.prize_distribution,
        target_award: old_config.target_award,
        reserve_factor: old_config.reserve_factor,
        split_factor: old_config.split_factor,
        instant_withdrawal_fee: old_config.instant_withdrawal_fee,
        unbonding_period: old_config.unbonding_period,
        max_tickets_per_depositor: msg.max_tickets_per_depositor,
        glow_prize_buckets: msg.glow_prize_buckets,
        paused: true,
        lotto_winner_boost_config,
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

    let old_state = OLDSTATE.load(deps.storage)?;

    let state = State {
        total_tickets: old_state.total_tickets,
        total_reserve: old_state.total_reserve,
        prize_buckets: old_state.prize_buckets,
        current_lottery: old_state.current_lottery,
        next_lottery_time: old_state.next_lottery_time,
        next_lottery_exec_time: old_state.next_lottery_exec_time,
        next_epoch: old_state.next_epoch,
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
    let old_pool = OLDPOOL.load(deps.as_ref().storage)?;
    let new_pool = Pool {
        total_user_aust: Uint256::zero(),
        total_user_shares: Uint256::zero(),
        total_sponsor_lottery_deposits: old_pool.total_sponsor_lottery_deposits,
        total_operator_shares: Uint256::zero(),
    };

    POOL.save(deps.storage, &new_pool)?;

    // Migrate prize info
    let old_prizes = OLD_PRIZES
        .range(deps.storage, None, None, Order::Ascending)
        .map(|item| {
            let (mut k, v) = item?;

            // https://github.com/CosmWasm/cw-plus/issues/466

            // Gets the length prefix from the composite key
            let mut tu = k.split_off(2);

            // Calculate the size of the first key in the composite key
            // using the length prefix
            let t_len = parse_length(&k)?;

            // Split tu into the first and second key.
            // u is the second key, and tu is the first key
            let u = tu.split_off(t_len);

            // Extract address from the first key
            let addr = Addr::unchecked(from_utf8(&tu)?);

            // Extract the lottery id from the second key
            let lottery_id = U64Key::from(u);

            // Return
            Ok((lottery_id, addr, v))
        })
        .collect::<StdResult<Vec<_>>>()?;

    for old_prize in old_prizes {
        let (lottery_id, addr, prize_info) = old_prize;
        OLD_PRIZES.remove(deps.storage, (&addr, lottery_id.clone()));

        PRIZES.save(deps.storage, (lottery_id, &addr), &prize_info)?;
    }

    Ok(Response::default())
}

pub fn migrate_old_depositors(
    deps: DepsMut,
    env: Env,
    limit: Option<u32>,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

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

    for (addr, old_depositor_info) in old_depositors {
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

        let state = STATE.load(deps.storage)?;

        // Don't need to include state.current_lottery
        // because nothing has been saved with id state.current_lottery yet
        for i in 0..state.current_lottery {
            let old_lottery_info = old_read_lottery_info(deps.storage, i);

            let new_lottery_info = LotteryInfo {
                rand_round: old_lottery_info.rand_round,
                sequence: old_lottery_info.sequence,
                awarded: old_lottery_info.awarded,
                timestamp: Timestamp::from_seconds(0),
                prize_buckets: old_lottery_info.prize_buckets,
                number_winners: old_lottery_info.number_winners,
                page: old_lottery_info.page,
                glow_prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
                block_height: old_lottery_info.timestamp,
                total_user_shares: pool.total_user_shares,
            };

            store_lottery_info(deps.storage, i, &new_lottery_info)?;

            old_remove_lottery_info(deps.storage, i);
        }

        // Set paused to false and save
        config.paused = false;
        CONFIG.save(deps.storage, &config)?;
    }

    POOL.save(deps.storage, &pool)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "migrate_old_depositors"),
        attr("num_migrated_entries", num_migrated_entries.to_string()),
    ]))
}

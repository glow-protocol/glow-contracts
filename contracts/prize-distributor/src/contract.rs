#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use glow_protocol::lotto::AmountRedeemableForPrizesInfo;

use crate::error::ContractError;
use crate::helpers::calculate_winner_prize;
use crate::prize_strategy::{
    execute_initiate_prize_distribution, execute_prize_distribution, execute_update_prize_buckets,
};
use crate::querier::{
    query_balance, query_exchange_rate, query_redeemable_funds_info, read_depositor_stats_at_height,
};
use crate::state::{read_lottery_info, read_lottery_prizes, Config, State, CONFIG, PRIZES, STATE};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, coin, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo,
    Reply, Response, StdError, StdResult, Timestamp, Uint128, WasmMsg,
};
use cw0::{Duration, Expiration};

use cw_storage_plus::U64Key;
use glow_protocol::distributor::ExecuteMsg as FaucetExecuteMsg;
use glow_protocol::prize_distributor::{PrizeInfo, NUM_PRIZE_BUCKETS};

use glow_protocol::prize_distributor::{
    BoostConfig, ConfigResponse, ExecuteMsg, InstantiateMsg, LotteryBalanceResponse,
    LotteryInfoResponse, MigrateMsg, PrizeInfoResponse, PrizeInfosResponse, QueryMsg,
    StateResponse,
};
use glow_protocol::querier::deduct_tax;
use moneymarket::market::ExecuteMsg as AnchorMsg;

use terraswap::querier::query_token_balance;

pub const INITIAL_DEPOSIT_AMOUNT: u128 = 10_000_000;
pub const MAX_CLAIMS: u8 = 15;
pub const THIRTY_MINUTE_TIME: u64 = 60 * 30;
pub const MAX_HOLDERS_FLOOR: u8 = 10;
pub const MAX_HOLDERS_CAP: u8 = 100;
pub const SEND_PRIZE_FUNDS_TO_PRIZE_DISTRIBUTOR_REPLY: u64 = 1;

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

    // Validate that epoch_interval is at least 30 minutes
    if msg.epoch_interval < THIRTY_MINUTE_TIME {
        return Err(ContractError::InvalidEpochInterval {});
    }

    // Get and validate the lotto winner boost config
    let default_lotto_winner_boost_config: BoostConfig = BoostConfig {
        base_multiplier: Decimal256::from_ratio(Uint256::from(40u128), Uint256::from(100u128)),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(150),
    };

    let lotto_winner_boost_config =
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

    CONFIG.save(
        deps.storage,
        &Config {
            owner: deps.api.addr_validate(msg.owner.as_str())?,
            a_terra_contract: deps.api.addr_validate(msg.aterra_contract.as_str())?,
            gov_contract: Addr::unchecked(""),
            ve_contract: Addr::unchecked(""),
            community_contract: Addr::unchecked(""),
            distributor_contract: Addr::unchecked(""),
            savings_contract: Addr::unchecked(""),
            oracle_contract: deps.api.addr_validate(msg.oracle_contract.as_str())?,
            stable_denom: msg.stable_denom.clone(),
            anchor_contract: deps.api.addr_validate(msg.anchor_contract.as_str())?,
            lottery_interval: msg.lottery_interval,
            epoch_interval: Duration::Time(msg.epoch_interval),
            block_time: Duration::Time(msg.block_time),
            round_delta: msg.round_delta,
            prize_distribution: msg.prize_distribution,
            reserve_factor: msg.reserve_factor,
            glow_prize_buckets: msg.glow_prize_buckets,
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
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Timestamp::from_seconds(msg.initial_lottery_execution),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: Duration::Time(msg.epoch_interval).after(&env.block),
            last_lottery_execution_aust_exchange_rate: aust_exchange_rate,
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
    match msg {
        ExecuteMsg::RegisterContracts {
            gov_contract,
            community_contract,
            distributor_contract,
            ve_contract,
            savings_contract,
        } => execute_register_contracts(
            deps,
            info,
            gov_contract,
            community_contract,
            distributor_contract,
            ve_contract,
            savings_contract,
        ),
        ExecuteMsg::ClaimLottery { lottery_ids } => {
            execute_claim_lottery(deps, env, info, lottery_ids)
        }
        ExecuteMsg::ExecuteLottery {} => execute_initiate_prize_distribution(deps, env, info),
        ExecuteMsg::ExecutePrize { limit } => execute_prize_distribution(deps, env, info, limit),
        ExecuteMsg::ExecuteEpochOps {} => execute_epoch_ops(deps, env),
        ExecuteMsg::UpdateConfig {
            owner,
            oracle_addr,
            reserve_factor,
            epoch_interval,
            paused,
            lotto_winner_boost_config,
        } => execute_update_config(
            deps,
            info,
            owner,
            oracle_addr,
            reserve_factor,
            epoch_interval,
            paused,
            lotto_winner_boost_config,
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
    community_contract: String,
    distributor_contract: String,
    ve_contract: String,
    savings_contract: String,
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
    config.savings_contract = deps.api.addr_validate(&savings_contract)?;

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::default())
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

        // Calculate lottery prize and add to to_send
        let lottery_key: U64Key = U64Key::from(lottery_id);
        let prize = PRIZES
            .may_load(deps.storage, (lottery_key.clone(), &info.sender))
            .unwrap();

        if let Some(prize) = prize {
            if prize.claimed {
                return Err(ContractError::InvalidClaimPrizeAlreadyClaimed(lottery_id));
            }

            let snapshotted_depositor_stats_info = read_depositor_stats_at_height(
                deps.as_ref(),
                info.sender.as_str(),
                lottery_info.block_height,
            )?;

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
        // Should never happen
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

    // Compute total_reserves to fund community contract
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

#[allow(clippy::too_many_arguments)]
pub fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    oracle_addr: Option<String>,
    reserve_factor: Option<Decimal256>,
    epoch_interval: Option<u64>,
    _paused: Option<bool>,
    lotto_winner_boost_config: Option<BoostConfig>,
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

    if let Some(reserve_factor) = reserve_factor {
        if reserve_factor > Decimal256::one() {
            return Err(ContractError::InvalidReserveFactor {});
        }

        config.reserve_factor = reserve_factor;
    }

    if let Some(epoch_interval) = epoch_interval {
        // Validate that epoch_interval is at least 30 minutes
        if epoch_interval < THIRTY_MINUTE_TIME {
            return Err(ContractError::InvalidEpochInterval {});
        }

        config.epoch_interval = Duration::Time(epoch_interval);
    }

    if let Some(lotto_winner_boost_config) = lotto_winner_boost_config {
        if lotto_winner_boost_config.base_multiplier > lotto_winner_boost_config.max_multiplier
            || lotto_winner_boost_config.total_voting_power_weight == Decimal256::zero()
        {
            return Err(ContractError::InvalidBoostConfig {});
        }
        config.lotto_winner_boost_config = lotto_winner_boost_config
    }

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![("action", "update_config")]))
}

pub fn execute_update_lottery_config(
    deps: DepsMut,
    info: MessageInfo,
    lottery_interval: Option<u64>,
    block_time: Option<u64>,
    _ticket_price: Option<Uint256>,
    prize_distribution: Option<[Decimal256; NUM_PRIZE_BUCKETS]>,
    round_delta: Option<u64>,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;

    // Check permission
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    if let Some(lottery_interval) = lottery_interval {
        config.lottery_interval = lottery_interval;
    }

    if let Some(block_time) = block_time {
        config.block_time = Duration::Time(block_time);
    }

    if let Some(round_delta) = round_delta {
        config.round_delta = round_delta;
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
        QueryMsg::LotteryInfo { lottery_id } => {
            to_binary(&query_lottery_info(deps, env, lottery_id)?)
        }
        QueryMsg::PrizeInfo {
            address,
            lottery_id,
        } => to_binary(&query_prizes(deps, address, lottery_id)?),
        QueryMsg::LotteryPrizeInfos {
            lottery_id,
            start_after,
            limit,
        } => to_binary(&query_lottery_prizes(deps, lottery_id, start_after, limit)?),
        QueryMsg::LotteryBalance {} => to_binary(&query_lottery_balance(deps, env)?),
    }
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
        read_depositor_stats_at_height(deps, addr.as_str(), lottery_info.block_height)?;

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
                read_depositor_stats_at_height(deps, addr.as_str(), lottery_info.block_height)?;

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
        savings_contract: config.savings_contract.to_string(),
        lottery_interval: config.lottery_interval,
        epoch_interval: config.epoch_interval,
        block_time: config.block_time,
        round_delta: config.round_delta,
        prize_distribution: config.prize_distribution,
        reserve_factor: config.reserve_factor,
    })
}

pub fn query_state(deps: Deps, _env: Env, _block_height: Option<u64>) -> StdResult<StateResponse> {
    let state = STATE.load(deps.storage)?;

    Ok(StateResponse {
        total_reserve: state.total_reserve,
        prize_buckets: state.prize_buckets,
        current_lottery: state.current_lottery,
        next_lottery_time: state.next_lottery_time,
        next_lottery_exec_time: state.next_lottery_exec_time,
        next_epoch: state.next_epoch,
        last_lottery_execution_aust_exchange_rate: state.last_lottery_execution_aust_exchange_rate,
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
        total_user_shares: lottery.total_user_shares,
    })
}

pub fn query_lottery_balance(deps: Deps, env: Env) -> StdResult<LotteryBalanceResponse> {
    let config = CONFIG.load(deps.storage)?;
    let state = STATE.load(deps.storage)?;

    // Get the contract's aust balance
    let _contract_a_balance = Uint256::from(query_token_balance(
        &deps.querier,
        config.a_terra_contract.clone(),
        env.clone().contract.address,
    )?);

    // Get the aust exchange rate
    let _aust_exchange_rate =
        query_exchange_rate(deps, config.anchor_contract.to_string(), env.block.height)?
            .exchange_rate;

    let AmountRedeemableForPrizesInfo {
        value_of_user_aust_to_be_redeemed_for_lottery,
        user_aust_to_redeem,
        value_of_sponsor_aust_to_be_redeemed_for_lottery,
        sponsor_aust_to_redeem,
        aust_to_redeem,
        aust_to_redeem_value,
    } = query_redeemable_funds_info(deps)?;

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
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    Ok(Response::default())
}

// Reply callback triggered from cw721 contract instantiation
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> Result<Response, ContractError> {
    match msg.id {
        SEND_PRIZE_FUNDS_TO_PRIZE_DISTRIBUTOR_REPLY => execute_update_prize_buckets(deps, env),
        _id => Err(ContractError::InvalidTokenReplyId {}),
    }
}

use crate::contract::{
    execute, instantiate, migrate, query, query_config, query_pool, query_state, query_ticket_info,
    INITIAL_DEPOSIT_AMOUNT,
};
use crate::helpers::{
    base64_encoded_tickets_to_vec_string_tickets, calculate_boost_multiplier, calculate_max_bound,
    calculate_value_of_aust_to_be_redeemed_for_lottery, calculate_winner_prize,
    get_minimum_matches_for_winning_ticket, uint256_times_decimal256_ceil,
    ExecuteLotteryRedeemedAustInfo,
};
use crate::mock_querier::{
    mock_dependencies, mock_env, mock_info, WasmMockQuerier, MOCK_CONTRACT_ADDR,
};
use crate::state::{
    old_read_depositor_info, old_read_lottery_info, old_remove_depositor_info, read_depositor_info,
    read_depositor_stats, read_depositor_stats_at_height, read_lottery_info, read_lottery_prizes,
    read_prize, read_sponsor_info, store_depositor_info, store_depositor_stats, Config,
    DepositorInfo, DepositorStatsInfo, LotteryInfo, OldConfig, OldDepositorInfo, OldPool, OldState,
    Pool, PrizeInfo, State, CONFIG, OLDCONFIG, OLDPOOL, OLDSTATE, OLD_PRIZES, POOL, PRIZES, STATE,
};
use crate::test_helpers::{
    calculate_lottery_prize_buckets, calculate_prize_buckets,
    calculate_remaining_state_prize_buckets, generate_sequential_ticket_combinations,
    old_store_depositor_info, old_store_lottery_info, vec_string_tickets_to_encoded_tickets,
};
use cosmwasm_storage::bucket;
use cw_storage_plus::U64Key;
use glow_protocol::lotto::{
    BoostConfig, MigrateMsg, OperatorInfoResponse, PrizeInfoResponse, RewardEmissionsIndex,
    NUM_PRIZE_BUCKETS, TICKET_LENGTH,
};
use lazy_static::lazy_static;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::testing::MockApi;
use cosmwasm_std::{
    attr, from_binary, to_binary, Addr, Api, BankMsg, Coin, CosmosMsg, Decimal, DepsMut, Env,
    MemoryStorage, OwnedDeps, Response, StdError, SubMsg, Timestamp, Uint128, WasmMsg,
};
use cw20::Cw20ExecuteMsg;
use glow_protocol::distributor::ExecuteMsg as FaucetExecuteMsg;
use glow_protocol::lotto::{
    Claim, ConfigResponse, ExecuteMsg, InstantiateMsg, PoolResponse, QueryMsg, SponsorInfoResponse,
    StateResponse,
};

use crate::error::ContractError;
use cw0::{Duration, Expiration, HOUR, WEEK};
use glow_protocol::querier::{deduct_tax, query_token_balance};
use moneymarket::market::{Cw20HookMsg, ExecuteMsg as AnchorMsg};
use std::ops::{Add, Mul, Sub};
use std::str::FromStr;

pub const TEST_CREATOR: &str = "creator";
pub const ANCHOR: &str = "anchor";
pub const A_UST: &str = "aterra-ust";
pub const DENOM: &str = "uusd";
pub const GOV_ADDR: &str = "gov";
pub const COMMUNITY_ADDR: &str = "community";
pub const DISTRIBUTOR_ADDR: &str = "distributor";
pub const VE_ADDR: &str = "ve_addr";
pub const ORACLE_ADDR: &str = "oracle";

pub const RATE: u64 = 1023; // as a permille
const SMALL_TICKET_PRICE: u64 = 10;
const TICKET_PRICE: u64 = 10_000_000; // 10 * 10^6

const SPLIT_FACTOR: u64 = 75; // as a %
const INSTANT_WITHDRAWAL_FEE: u64 = 10; // as a %
pub const RESERVE_FACTOR: u64 = 5; // as a %
const MAX_HOLDERS: u8 = 10;
const WEEK_TIME: u64 = 604800; // in seconds
const HOUR_TIME: u64 = 3600; // in seconds
const ROUND_DELTA: u64 = 10;
const FIRST_LOTTO_TIME: u64 = 1595961494; // timestamp between deployment and 1 week after
const MAX_TICKETS_PER_DEPOSITOR: u64 = 12000;

const SIX_MATCH_SEQUENCE: &str = "be1ce9";
const FOUR_MATCH_SEQUENCE: &str = "be1c79";
const FOUR_MATCH_SEQUENCE_2: &str = "be1c89";
const FOUR_MATCH_SEQUENCE_3: &str = "be1c99";
const THREE_MATCH_SEQUENCE: &str = "be18e9";
const TWO_MATCH_SEQUENCE: &str = "be0ce9";
const ONE_MATCH_SEQUENCE: &str = "b81ce9";
const ZERO_MATCH_SEQUENCE: &str = "6e1ce9";
const ZERO_MATCH_SEQUENCE_2: &str = "7e1ce9";
const ZERO_MATCH_SEQUENCE_3: &str = "8e1ce9";
const ZERO_MATCH_SEQUENCE_4: &str = "9e1ce9";
// const INVALID_TICKET_TOO_LONG: &str = "2b02cabf";
// const INVALID_TICKET_TOO_SHORT: &str = "2b02";
// const INVALID_TICKET_NOT_HEX: &str = "2b02cg";

lazy_static! {
    static ref PRIZE_DISTRIBUTION: [Decimal256; NUM_PRIZE_BUCKETS] = [
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::percent(5),
        Decimal256::percent(15),
        Decimal256::percent(25),
        Decimal256::percent(35),
        Decimal256::percent(20),
    ];
    static ref GLOW_PRIZE_BUCKETS: [Uint256; NUM_PRIZE_BUCKETS] = [
        Uint256::from(0u128),
        Uint256::from(0u128),
        Uint256::from(10 * u128::pow(10, 6)),
        Uint256::from(50 * u128::pow(10, 6)),
        Uint256::from(100 * u128::pow(10, 6)),
        Uint256::from(1000 * u128::pow(10, 6)),
        Uint256::from(100000 * u128::pow(10, 6)),
    ];
}

pub(crate) fn instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        owner: TEST_CREATOR.to_string(),
        stable_denom: DENOM.to_string(),
        anchor_contract: ANCHOR.to_string(),
        aterra_contract: A_UST.to_string(),
        oracle_contract: ORACLE_ADDR.to_string(),
        lottery_interval: WEEK_TIME,
        epoch_interval: 3 * HOUR_TIME,
        block_time: HOUR_TIME,
        round_delta: ROUND_DELTA,
        ticket_price: Uint256::from(TICKET_PRICE),
        max_holders: MAX_HOLDERS,
        prize_distribution: *PRIZE_DISTRIBUTION,
        target_award: Uint256::zero(),
        reserve_factor: Decimal256::percent(RESERVE_FACTOR),
        split_factor: Decimal256::percent(SPLIT_FACTOR),
        instant_withdrawal_fee: Decimal256::percent(INSTANT_WITHDRAWAL_FEE),
        unbonding_period: WEEK_TIME,
        initial_sponsor_glow_emission_rate: Decimal256::zero(),
        initial_operator_glow_emission_rate: Decimal256::zero(),
        initial_lottery_execution: FIRST_LOTTO_TIME,
        max_tickets_per_depositor: MAX_TICKETS_PER_DEPOSITOR,
        glow_prize_buckets: *GLOW_PRIZE_BUCKETS,
        lotto_winner_boost_config: None,
    }
}

pub(crate) fn instantiate_msg_small_ticket_price() -> InstantiateMsg {
    InstantiateMsg {
        owner: TEST_CREATOR.to_string(),
        stable_denom: DENOM.to_string(),
        anchor_contract: ANCHOR.to_string(),
        aterra_contract: A_UST.to_string(),
        oracle_contract: ORACLE_ADDR.to_string(),
        lottery_interval: WEEK_TIME,
        epoch_interval: 3 * HOUR_TIME,
        block_time: HOUR_TIME,
        round_delta: ROUND_DELTA,
        ticket_price: Uint256::from(SMALL_TICKET_PRICE),
        max_holders: MAX_HOLDERS,
        prize_distribution: *PRIZE_DISTRIBUTION,
        target_award: Uint256::zero(),
        reserve_factor: Decimal256::percent(RESERVE_FACTOR),
        split_factor: Decimal256::percent(SPLIT_FACTOR),
        instant_withdrawal_fee: Decimal256::percent(INSTANT_WITHDRAWAL_FEE),
        unbonding_period: WEEK_TIME,
        initial_sponsor_glow_emission_rate: Decimal256::zero(),
        initial_operator_glow_emission_rate: Decimal256::zero(),
        initial_lottery_execution: FIRST_LOTTO_TIME,
        max_tickets_per_depositor: MAX_TICKETS_PER_DEPOSITOR,
        glow_prize_buckets: *GLOW_PRIZE_BUCKETS,
        lotto_winner_boost_config: None,
    }
}

fn mock_instantiate(deps: &mut OwnedDeps<MemoryStorage, MockApi, WasmMockQuerier>) {
    let msg = instantiate_msg();

    let info = mock_info(
        TEST_CREATOR,
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    instantiate(deps.as_mut(), mock_env(), info, msg)
        .expect("contract successfully executes InstantiateMsg");

    let net_amount = Uint256::from(
        deduct_tax(
            deps.as_ref(),
            Coin {
                denom: String::from("uusd"),
                amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
            },
        )
        .unwrap()
        .amount,
    );

    // withdraw sponsor
    let app_aust = net_amount / Decimal256::permille(RATE);

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &app_aust.into())],
    )]);
}

fn mock_instantiate_small_ticket_price(deps: DepsMut) -> Response {
    let msg = instantiate_msg_small_ticket_price();

    let info = mock_info(
        TEST_CREATOR,
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    instantiate(deps, mock_env(), info, msg).expect("contract successfully executes InstantiateMsg")
}

fn mock_register_contracts(deps: DepsMut) {
    let info = mock_info(TEST_CREATOR, &[]);
    let msg = ExecuteMsg::RegisterContracts {
        gov_contract: GOV_ADDR.to_string(),
        community_contract: COMMUNITY_ADDR.to_string(),
        distributor_contract: DISTRIBUTOR_ADDR.to_string(),
        ve_contract: VE_ADDR.to_string(),
    };
    let _res = execute(deps, mock_env(), info, msg)
        .expect("contract successfully executes RegisterContracts");
}

#[allow(dead_code)]
fn mock_env_height(height: u64, time: u64) -> Env {
    let mut env = mock_env();
    env.block.height = height;
    env.block.time = Timestamp::from_seconds(time);
    env
}

#[test]
fn proper_initialization() {
    let mut deps = mock_dependencies(&[Coin {
        denom: DENOM.to_string(),
        amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
    }]);

    let msg = instantiate_msg();
    let info = mock_info(
        TEST_CREATOR,
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let env = mock_env();

    let res = instantiate(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();
    assert_eq!(1, res.messages.len());

    let config = query_config(deps.as_ref()).unwrap();

    assert_eq!(
        config,
        ConfigResponse {
            owner: TEST_CREATOR.to_string(),
            a_terra_contract: A_UST.to_string(),
            gov_contract: "".to_string(),
            ve_contract: "".to_string(),
            community_contract: "".to_string(),
            distributor_contract: "".to_string(),
            anchor_contract: ANCHOR.to_string(),
            stable_denom: DENOM.to_string(),
            lottery_interval: WEEK,
            epoch_interval: HOUR.mul(3),
            block_time: HOUR,
            round_delta: ROUND_DELTA,
            ticket_price: Uint256::from(TICKET_PRICE),
            max_holders: MAX_HOLDERS,
            prize_distribution: *PRIZE_DISTRIBUTION,
            target_award: Uint256::zero(),
            reserve_factor: Decimal256::percent(RESERVE_FACTOR),
            split_factor: Decimal256::percent(SPLIT_FACTOR),
            instant_withdrawal_fee: Decimal256::percent(INSTANT_WITHDRAWAL_FEE),
            unbonding_period: WEEK,
            max_tickets_per_depositor: MAX_TICKETS_PER_DEPOSITOR,
            paused: false
        }
    );

    // Check that the glow_emission_rate and last_block_updated are set correctly
    let state = STATE.load(deps.as_ref().storage).unwrap();
    assert_eq!(
        state.operator_reward_emission_index.glow_emission_rate,
        Decimal256::zero()
    );
    assert_eq!(
        state.sponsor_reward_emission_index.glow_emission_rate,
        Decimal256::zero()
    );
    assert_eq!(
        state.operator_reward_emission_index.last_reward_updated,
        mock_env().block.height
    );
    assert_eq!(
        state.sponsor_reward_emission_index.last_reward_updated,
        mock_env().block.height
    );

    // Register contracts
    let msg = ExecuteMsg::RegisterContracts {
        gov_contract: GOV_ADDR.to_string(),
        community_contract: COMMUNITY_ADDR.to_string(),
        distributor_contract: DISTRIBUTOR_ADDR.to_string(),
        ve_contract: VE_ADDR.to_string(),
    };

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();
    let config = query_config(deps.as_ref()).unwrap();
    assert_eq!(config.gov_contract, GOV_ADDR.to_string());
    assert_eq!(config.community_contract, COMMUNITY_ADDR.to_string());
    assert_eq!(config.distributor_contract, DISTRIBUTOR_ADDR.to_string());

    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();
    assert_eq!(
        state,
        StateResponse {
            total_tickets: Uint256::zero(),
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: HOUR.mul(3).after(&mock_env().block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            last_lottery_execution_aust_exchange_rate: Decimal256::permille(RATE)
        }
    );

    let pool = query_pool(deps.as_ref()).unwrap();
    assert_eq!(
        pool,
        PoolResponse {
            total_user_shares: Uint256::zero(),
            total_user_aust: Uint256::zero(),
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_operator_shares: Uint256::zero(),
        }
    );

    // Cannot register contracts again
    let res = execute(deps.as_mut(), env, info, msg);

    match res {
        Err(ContractError::AlreadyRegistered {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

#[test]
fn update_config() {
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // update owner
    let info = mock_info(TEST_CREATOR, &[]);

    let msg = ExecuteMsg::UpdateConfig {
        owner: Some("owner1".to_string()),
        oracle_addr: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        reserve_factor: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: None,
        lotto_winner_boost_config: None,
    };
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // Check owner has changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();

    assert_eq!("owner1".to_string(), config_response.owner);

    // update lottery interval to 30 minutes
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateLotteryConfig {
        lottery_interval: Some(1800),
        block_time: None,
        round_delta: None,
        ticket_price: None,
        prize_distribution: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check lottery_interval has changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.lottery_interval, Duration::Time(1800));

    // update reserve_factor to 1%
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: Some(Decimal256::percent(1)),
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: None,
        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check reserve_factor has changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.reserve_factor, Decimal256::percent(1));

    // update epoch_interval to 5 hours
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: Some(HOUR_TIME * 5),
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: None,

        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check that epoch_interval changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.epoch_interval, HOUR.mul(5));

    // check that you can't set epoch_interval to a value
    // less than 30 minutes
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: Some(HOUR_TIME / 3),
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: None,

        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidEpochInterval {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Check updating max_owners --------

    // Try decreasing max_holders below floor

    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: Some(8),
        max_tickets_per_depositor: None,
        paused: None,

        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidMaxHoldersOutsideBounds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Updating max_holders to 15
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: Some(15),
        max_tickets_per_depositor: None,
        paused: None,

        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check that max_holders changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.max_holders, 15);

    // try decreasing max_holders
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: Some(14),
        max_tickets_per_depositor: None,
        paused: None,

        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidMaxHoldersAttemptedDecrease {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // try increasing above max_holders_cap
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: Some(101),
        max_tickets_per_depositor: None,
        paused: None,

        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidMaxHoldersOutsideBounds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Update the max_tickets_per_depositor
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: Some(100),
        paused: None,

        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check max_tickets_per_depositor has changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.max_tickets_per_depositor, 100);

    // Try updating paused
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: Some(true),
        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check paused has changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert!(config_response.paused);

    // check only owner can update config
    let info = mock_info("owner2", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        oracle_addr: None,
        owner: Some(String::from("new_owner")),
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: None,

        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::Unauthorized {}) => {}
        _ => panic!("Must return unauthorized error"),
    }
}

#[test]
fn test_max_tickets_per_depositor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Invalid deposit - exceeds max_tickets_per_depositor
    let info = mock_info(
        "addr1000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from((MAX_TICKETS_PER_DEPOSITOR + 1) * TICKET_PRICE).into(),
        }],
    );
    let too_many_combinations =
        generate_sequential_ticket_combinations(MAX_TICKETS_PER_DEPOSITOR + 1);

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(too_many_combinations),
        operator: None,
    };
    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::MaxTicketsPerDepositorExceeded {
            max_tickets_per_depositor,
            post_transaction_num_depositor_tickets,
        }) if max_tickets_per_depositor == MAX_TICKETS_PER_DEPOSITOR
            && post_transaction_num_depositor_tickets == MAX_TICKETS_PER_DEPOSITOR + 1 => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Deposit at the limit successfully
    let info = mock_info(
        "addr1000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from((MAX_TICKETS_PER_DEPOSITOR) * TICKET_PRICE).into(),
        }],
    );
    let too_many_combinations = generate_sequential_ticket_combinations(MAX_TICKETS_PER_DEPOSITOR);

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(too_many_combinations),
        operator: None,
    };
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Depositing one more ticket fails because it goes over the limit

    let info = mock_info(
        "addr1000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );
    let too_many_combinations = generate_sequential_ticket_combinations(1);

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(too_many_combinations),
        operator: None,
    };
    let res = execute(deps.as_mut(), mock_env(), info, msg);

    match res {
        Err(ContractError::MaxTicketsPerDepositorExceeded {
            max_tickets_per_depositor,
            post_transaction_num_depositor_tickets,
        }) if max_tickets_per_depositor == MAX_TICKETS_PER_DEPOSITOR
            && post_transaction_num_depositor_tickets == MAX_TICKETS_PER_DEPOSITOR + 1 => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // If we increase the limit than we can deposit again

    // Update the max_tickets_per_depositor
    let info = mock_info(TEST_CREATOR, &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: Some(MAX_TICKETS_PER_DEPOSITOR + 1),
        paused: None,

        lotto_winner_boost_config: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Depositing one more ticket now succeeds

    let info = mock_info(
        "addr1000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );
    let too_many_combinations = generate_sequential_ticket_combinations(1);

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(too_many_combinations),
        operator: None,
    };
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
}

#[test]
fn deposit() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Must deposit stable_denom coins
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(THREE_MATCH_SEQUENCE),
            String::from(ZERO_MATCH_SEQUENCE),
        ]),
        operator: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "ukrw".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg.clone());
    match res {
        Err(ContractError::ZeroDepositAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // correct base denom, zero deposit
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint128::zero(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::ZeroDepositAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // These tests don't really make sense anymore because the encoding process will throw errors

    // // Invalid ticket sequence - more number of digits
    // let msg = ExecuteMsg::Deposit {
    //     encoded_tickets: combinations_to_encoded_tickets(vec![
    //         String::from(INVALID_TICKET_TOO_LONG),
    //         String::from(ZERO_MATCH_SEQUENCE),
    //     ]),
    // };
    // let info = mock_info(
    //     "addr0000",
    //     &[Coin {
    //         denom: DENOM.to_string(),
    //         amount: Uint256::from(2 * TICKET_PRICE).into(),
    //     }],
    // );
    // let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    // match res {
    //     Err(ContractError::InvalidSequence(sequence)) if sequence == INVALID_TICKET_TOO_LONG => {}
    //     _ => panic!("DO NOT ENTER HERE"),
    // }

    // // Invalid ticket sequence - less number of digits
    // let msg = ExecuteMsg::Deposit {
    //     encoded_tickets: combinations_to_encoded_tickets(vec![
    //         String::from(ZERO_MATCH_SEQUENCE),
    //         String::from(INVALID_TICKET_TOO_SHORT),
    //     ]),
    // };

    // let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    // match res {
    //     Err(ContractError::InvalidSequence(sequence)) if sequence == INVALID_TICKET_TOO_SHORT => {}
    //     _ => panic!("DO NOT ENTER HERE"),
    // }

    // // Invalid ticket sequence - only hex values allowed
    // let msg = ExecuteMsg::Deposit {
    //     encoded_tickets: combinations_to_encoded_tickets(vec![
    //         String::from(INVALID_TICKET_NOT_HEX),
    //         String::from(ZERO_MATCH_SEQUENCE),
    //     ]),
    // };
    // let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    // match res {
    //     Err(ContractError::InvalidSequence(sequence)) if sequence == INVALID_TICKET_NOT_HEX => {}
    //     _ => panic!("DO NOT ENTER HERE"),
    // }

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );

    // Correct deposit - buys two tickets
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
        operator: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the amount of minted_shares
    let minted_shares = minted_aust;

    // Check address of sender was stored correctly in both sequence buckets
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from(ZERO_MATCH_SEQUENCE))
            .unwrap()
            .holders,
        vec![Addr::unchecked("addr0000")]
    );
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from(ONE_MATCH_SEQUENCE))
            .unwrap()
            .holders,
        vec![Addr::unchecked("addr0000")]
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0000").unwrap()
        ),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![
                String::from(ZERO_MATCH_SEQUENCE),
                String::from(ONE_MATCH_SEQUENCE)
            ],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(2u64),
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: (HOUR.mul(3)).after(&mock_env().block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },

            last_lottery_execution_aust_exchange_rate: Decimal256::permille(RATE)
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_shares: minted_shares,
            total_user_aust: minted_shares,
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_operator_shares: Uint256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: String::from("uusd"),
                amount: Uint256::from(2 * TICKET_PRICE).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "deposit"),
            attr("depositor", "addr0000"),
            attr("recipient", "addr0000"),
            attr(
                "deposit_amount",
                Uint256::from(2 * TICKET_PRICE).to_string()
            ),
            attr("tickets", 2u64.to_string()),
            attr(
                "aust_minted",
                (Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE)).to_string()
            ),
        ]
    );

    // test round-up tickets
    let deposit_amount = Uint256::from(TICKET_PRICE) * Decimal256::from_ratio(16, 10);

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: deposit_amount.into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            TWO_MATCH_SEQUENCE,
        )]),
        operator: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 3);

    // deposit again
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            THREE_MATCH_SEQUENCE,
        )]),
        operator: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 5);

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ZERO_MATCH_SEQUENCE_2,
        )]),
        operator: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 6);

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ZERO_MATCH_SEQUENCE_3,
        )]),
        operator: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 8);

    // Test sequential buys of the same ticket by the same address
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            FOUR_MATCH_SEQUENCE,
        )]),
        operator: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            FOUR_MATCH_SEQUENCE,
        )]),
        operator: None,
    };

    // We let users have a repeated ticket
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Ticket is already owner by 10 holders
    let addresses_count = 10u64;
    let addresses_range = 0..addresses_count;
    let addresses = addresses_range
        .map(|c| format!("addr{:0>4}", c))
        .collect::<Vec<String>>();

    for (_index, address) in addresses.iter().enumerate() {
        // Users buys winning ticket
        let msg = ExecuteMsg::Deposit {
            encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
                ZERO_MATCH_SEQUENCE_4,
            )]),
            operator: None,
        };
        let info = mock_info(
            address.as_str(),
            &[Coin {
                denom: "uusd".to_string(),
                amount: Uint256::from(TICKET_PRICE).into(),
            }],
        );

        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    }

    let holders = query_ticket_info(deps.as_ref(), String::from(ZERO_MATCH_SEQUENCE_4))
        .unwrap()
        .holders;
    println!("holders: {:?}", holders);
    println!("len: {:?}", holders.len());

    // 11th holder with same sequence, should fail
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ZERO_MATCH_SEQUENCE_4,
        )]),
        operator: None,
    };
    let info = mock_info(
        "addr1111",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidHolderSequence(sequence))
            if sequence == ZERO_MATCH_SEQUENCE_4 => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

#[test]
fn gift_tickets() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Must deposit stable_denom coins
    let msg = ExecuteMsg::Gift {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
        recipient: "addr1111".to_string(),
        operator: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "ukrw".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg.clone());
    match res {
        Err(ContractError::ZeroGiftAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // correct base denom, zero deposit
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::zero(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg.clone());
    match res {
        Err(ContractError::ZeroGiftAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let wrong_amount = Uint256::from(TICKET_PRICE);

    // correct base denom, deposit different to TICKET_PRICE
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: wrong_amount.into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg);

    let expected_tickets_attempted = 2;
    match res {
        Err(ContractError::InsufficientGiftDepositAmount(amount_required)) => {
            assert_eq!(expected_tickets_attempted, amount_required)
        }
        _ => panic!("DO NOT ENTER HERE"),
    }
    // Invalid recipient - you cannot make a gift to yourself
    let msg = ExecuteMsg::Gift {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE_3),
            String::from(ZERO_MATCH_SEQUENCE_4),
        ]),
        recipient: "addr0000".to_string(),
        operator: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );
    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::GiftToSelf {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // These tests don't really make sense anymore because they will throw errors during the encoding process
    // // Invalid ticket sequence - more number of digits
    // let msg = ExecuteMsg::Gift {
    //     encoded_tickets: combinations_to_encoded_tickets(vec![
    //         String::from(INVALID_TICKET_TOO_LONG),
    //         String::from(ZERO_MATCH_SEQUENCE),
    //     ]),
    //     recipient: "addr1111".to_string(),
    // };
    // let info = mock_info(
    //     "addr0000",
    //     &[Coin {
    //         denom: DENOM.to_string(),
    //         amount: Uint256::from(2 * TICKET_PRICE).into(),
    //     }],
    // );
    // let res = execute(deps.as_mut(), mock_env(), info, msg);
    // match res {
    //     Err(ContractError::InvalidSequence(sequence)) if sequence == INVALID_TICKET_TOO_LONG => {}
    //     _ => panic!("DO NOT ENTER HERE"),
    // }

    // // Invalid ticket sequence - less number of digits
    // let msg = ExecuteMsg::Gift {
    //     encoded_tickets: combinations_to_encoded_tickets(vec![
    //         String::from(ZERO_MATCH_SEQUENCE),
    //         String::from(INVALID_TICKET_TOO_SHORT),
    //     ]),
    //     recipient: "addr1111".to_string(),
    // };
    // let info = mock_info(
    //     "addr0000",
    //     &[Coin {
    //         denom: "uusd".to_string(),
    //         amount: Uint256::from(2 * TICKET_PRICE).into(),
    //     }],
    // );
    // let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    // match res {
    //     Err(ContractError::InvalidSequence(sequence)) if sequence == INVALID_TICKET_TOO_SHORT => {}
    //     _ => panic!("DO NOT ENTER HERE"),
    // }

    // // Invalid ticket sequence - only numbers allowed
    // let msg = ExecuteMsg::Gift {
    //     encoded_tickets: combinations_to_encoded_tickets(vec![
    //         String::from(INVALID_TICKET_NOT_HEX),
    //         String::from(ZERO_MATCH_SEQUENCE),
    //     ]),
    //     recipient: "addr1111".to_string(),
    // };

    // let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    // match res {
    //     Err(ContractError::InvalidSequence(sequence)) if sequence == INVALID_TICKET_NOT_HEX => {}
    //     _ => panic!("DO NOT ENTER HERE"),
    // }

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );

    // Correct gift - gifts two tickets
    let msg = ExecuteMsg::Gift {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
        recipient: "addr1111".to_string(),
        operator: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the amount of minted_shares
    let minted_shares = minted_aust;

    // Check address of sender was stored correctly in both sequence buckets
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from(ZERO_MATCH_SEQUENCE))
            .unwrap()
            .holders,
        vec![deps.api.addr_validate("addr1111").unwrap()]
    );
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from(ONE_MATCH_SEQUENCE))
            .unwrap()
            .holders,
        vec![deps.api.addr_validate("addr1111").unwrap()]
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr1111").unwrap()
        ),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![
                String::from(ZERO_MATCH_SEQUENCE),
                String::from(ONE_MATCH_SEQUENCE)
            ],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(2u64),
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: HOUR.mul(3).after(&mock_env().block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            last_lottery_execution_aust_exchange_rate: Decimal256::permille(RATE)
        }
    );

    // They should be equal after a first deposit
    assert_eq!(minted_shares, minted_aust);

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_shares: minted_shares,
            total_user_aust: minted_aust,
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_operator_shares: Uint256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: DENOM.to_string(),
                amount: Uint256::from(2 * TICKET_PRICE).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "deposit"),
            attr("depositor", "addr0000"),
            attr("recipient", "addr1111"),
            attr(
                "deposit_amount",
                Uint256::from(2 * TICKET_PRICE).to_string()
            ),
            attr("tickets", 2u64.to_string()),
            attr("aust_minted", minted_aust.to_string()),
        ]
    );
}

#[test]
fn sponsor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let sponsor_amount = 100_000_000u128;

    deps.querier.with_tax(
        Decimal::percent(1),
        &[(&"uusd".to_string(), &Uint128::from(1_000_000u128))],
    );

    // Address sponsor
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(sponsor_amount),
        }],
    );

    let msg = ExecuteMsg::Sponsor {
        award: None,
        prize_distribution: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info, msg);
    println!("{:?}", _res);

    let net_amount = Uint256::from(
        deduct_tax(
            deps.as_ref(),
            Coin {
                denom: String::from("uusd"),
                amount: Uint128::from(sponsor_amount),
            },
        )
        .unwrap()
        .amount,
    );

    let minted_aust = net_amount / Decimal256::permille(RATE);

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::SponsorWithdraw {};
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let sponsor_info = read_sponsor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0001").unwrap(),
    );

    let pool = query_pool(deps.as_ref()).unwrap();

    assert_eq!(sponsor_info.lottery_deposit, Uint256::zero());
    assert_eq!(pool.total_sponsor_lottery_deposits, Uint256::zero());
}

#[test]
fn instant_sponsor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let sponsor_amount = 100_000_000u128;

    deps.querier.with_tax(
        Decimal::percent(1),
        &[(&"uusd".to_string(), &Uint128::from(1_000_000u128))],
    );

    // Address sponsor
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(sponsor_amount),
        }],
    );

    // Test sponsoring with the default prize distribution

    let msg = ExecuteMsg::Sponsor {
        award: Some(true),
        prize_distribution: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    println!("{:?}", _res);

    // Check that the prize buckets were updated

    let mut prize_buckets = [Uint256::zero(); NUM_PRIZE_BUCKETS];

    // Distribute the sponsorship to the prize buckets according to the prize distribution
    for (index, fraction_of_prize) in PRIZE_DISTRIBUTION.iter().enumerate() {
        // Add the proportional amount of the net redeemed amount to the relevant award bucket.
        prize_buckets[index] += Uint256::from(sponsor_amount) * *fraction_of_prize
    }

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, prize_buckets);

    // Check that the sponsor doesn't exist in the db

    let sponsor_info = read_sponsor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0001").unwrap(),
    );

    assert_eq!(sponsor_info.lottery_deposit, Uint256::zero());

    // Check that the pool sponsor deposits are zero

    let pool = query_pool(deps.as_ref()).unwrap();
    assert_eq!(pool.total_sponsor_lottery_deposits, Uint256::zero());

    // Test sponsoring with a custom prize distribution

    let custom_prize_distribution = [
        Decimal256::zero(),
        Decimal256::percent(5),
        Decimal256::percent(5),
        Decimal256::percent(15),
        Decimal256::percent(25),
        Decimal256::percent(30),
        Decimal256::percent(20),
    ];
    let msg = ExecuteMsg::Sponsor {
        award: Some(true),
        prize_distribution: Some(custom_prize_distribution),
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    println!("{:?}", _res);

    // Check that the prize buckets were updated

    // Distribute the sponsorship to the prize buckets according to the prize distribution
    for (index, fraction_of_prize) in custom_prize_distribution.iter().enumerate() {
        // Add the proportional amount of the net redeemed amount to the relevant award bucket.
        prize_buckets[index] += Uint256::from(sponsor_amount) * *fraction_of_prize
    }

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, prize_buckets);

    // Check that the sponsor doesn't exist in the db

    let sponsor_info = read_sponsor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0001").unwrap(),
    );

    assert_eq!(sponsor_info.lottery_deposit, Uint256::zero());

    // Check that the pool sponsor deposits are zero

    let pool = query_pool(deps.as_ref()).unwrap();
    assert_eq!(pool.total_sponsor_lottery_deposits, Uint256::zero());

    // Test sponsoring with a prize distribution that doesn't sum to 1

    let custom_prize_distribution = [
        Decimal256::zero(),
        Decimal256::percent(10),
        Decimal256::percent(10),
        Decimal256::percent(15),
        Decimal256::percent(25),
        Decimal256::percent(30),
        Decimal256::percent(20),
    ];
    let msg = ExecuteMsg::Sponsor {
        award: Some(true),
        prize_distribution: Some(custom_prize_distribution),
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidPrizeDistribution {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

#[test]
fn withdraw() {
    // Initialize contract
    let mut deps = mock_dependencies(&[Coin {
        denom: DENOM.to_string(),
        amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
    }]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let deposit_amount = Uint256::from(TICKET_PRICE).into();

    // Address buys one ticket
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: deposit_amount,
        }],
    );

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ONE_MATCH_SEQUENCE,
        )]),
        operator: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    let info = mock_info("addr0001", &[]);

    let msg = ExecuteMsg::Withdraw {
        amount: None,
        instant: None,
    };

    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR.to_string(),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: deposit_amount,
        }],
    );

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    // Correct withdraw, user has 1 ticket to be withdrawn
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Get the sent_amount
    let sent_amount = if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
        let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
        if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
            amount
        } else {
            panic!("DO NOT ENTER HERE")
        }
    } else {
        panic!("DO NOT ENTER HERE");
    };

    println!("Shares vs sent_amount: {}, {}", minted_aust, sent_amount);

    let empty_addr: Vec<Addr> = vec![];
    // Check address of sender was removed correctly in the sequence bucket
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("234567"))
            .unwrap()
            .holders,
        empty_addr
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0001").unwrap()
        ),
        DepositorInfo {
            shares: Uint256::zero(),
            tickets: vec![],
            unbonding_info: vec![Claim {
                amount: Uint256::from(sent_amount) * Decimal256::permille(RATE),
                release_at: WEEK.after(&mock_env().block),
            }],
            operator_addr: Addr::unchecked("")
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::zero(),
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: HOUR.mul(3).after(&mock_env().block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            last_lottery_execution_aust_exchange_rate: Decimal256::permille(RATE)
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_shares: Uint256::zero(),
            total_user_aust: Uint256::zero(),
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_operator_shares: Uint256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: A_UST.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: ANCHOR.to_string(),
                amount: sent_amount,
                msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
            })
            .unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "withdraw_ticket"),
            attr("depositor", "addr0001"),
            attr("tickets_amount", 1u64.to_string()),
            attr("redeem_amount_anchor", sent_amount.to_string()),
            attr(
                "redeem_stable_amount",
                (Uint256::from(sent_amount) * Decimal256::permille(RATE)).to_string()
            ),
            attr("instant_withdrawal_fee", Uint256::zero().to_string())
        ]
    );

    // Not counting tax
    // TODO Separate test with taxes
    // deps.querier.with_tax(
    //     Decimal::percent(1),
    //     &[(&"uusd".to_string(), &Uint128::from(1000000u128))],
    // );

    // Deposit one ticket 10 times
    for index in 0..10 {
        let msg = ExecuteMsg::Deposit {
            encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![format!(
                "{:0length$}",
                index,
                length = TICKET_LENGTH
            )]),
            operator: None,
        };
        let info = mock_info(
            "addr2222",
            &[Coin {
                denom: "uusd".to_string(),
                amount: Uint256::from(TICKET_PRICE).into(),
            }],
        );

        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    }

    let dep = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr2222").unwrap(),
    );

    println!("depositor: {:?}", dep);
    let minted_aust = Uint256::from(10 * TICKET_PRICE) / Decimal256::permille(RATE);

    let info = mock_info("addr2222", &[]);

    // Withdraws half of its tickets
    let msg = ExecuteMsg::Withdraw {
        // Withdraw amount - 1 to avoid rounding issues
        amount: Some(Uint256::from(5 * TICKET_PRICE - 1).into()),
        instant: None,
    };

    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR.to_string(),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: deposit_amount,
        }],
    );

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    // Correct withdraw, user withdraws 5 tickets
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let dep = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr2222").unwrap(),
    );

    println!("depositor: {:?}", dep);

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr2222").unwrap()
        )
        .tickets,
        vec![
            format!("{:0length$}", 5, length = TICKET_LENGTH),
            format!("{:0length$}", 6, length = TICKET_LENGTH),
            format!("{:0length$}", 7, length = TICKET_LENGTH),
            format!("{:0length$}", 8, length = TICKET_LENGTH),
            format!("{:0length$}", 9, length = TICKET_LENGTH)
        ]
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None)
            .unwrap()
            .total_tickets,
        Uint256::from(5u64)
    );

    // Check ticket map is updated correctly
    assert_eq!(
        query_ticket_info(
            deps.as_ref(),
            format!("{:0length$}", 2, length = TICKET_LENGTH)
        )
        .unwrap()
        .holders,
        empty_addr
    );

    assert_eq!(
        query_ticket_info(
            deps.as_ref(),
            format!("{:0length$}", 6, length = TICKET_LENGTH)
        )
        .unwrap()
        .holders,
        vec![Addr::unchecked("addr2222")]
    );

    // Withdraws a very small amount, burns a ticket as rounding
    let msg = ExecuteMsg::Withdraw {
        amount: Some(Uint128::from(1u128)),
        instant: None,
    };

    // Correct withdraw, one ticket gets withdrawn
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr2222").unwrap()
        )
        .tickets,
        vec![
            format!("{:0length$}", 6, length = TICKET_LENGTH),
            format!("{:0length$}", 7, length = TICKET_LENGTH),
            format!("{:0length$}", 8, length = TICKET_LENGTH),
            format!("{:0length$}", 9, length = TICKET_LENGTH)
        ]
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None)
            .unwrap()
            .total_tickets,
        // TODO Don't hardcode
        Uint256::from(4u64)
    );
    // Check ticket map is updated correctly
    assert_eq!(
        query_ticket_info(
            deps.as_ref(),
            format!("{:0length$}", 5, length = TICKET_LENGTH)
        )
        .unwrap()
        .holders,
        empty_addr
    );
}

#[test]
fn instant_withdraw() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let deposit_amount = Uint256::from(TICKET_PRICE).into();

    // Address buys one ticket
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: DENOM.to_string(),
            amount: deposit_amount,
        }],
    );

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ZERO_MATCH_SEQUENCE,
        )]),
        operator: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let info = mock_info("addr0001", &[]);

    let msg = ExecuteMsg::Withdraw {
        amount: None,
        instant: Some(true),
    };

    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR.to_string(),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: deposit_amount,
        }],
    );

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Shares equals aust in this case
    let aust_to_redeem = minted_aust;
    let mut return_amount = aust_to_redeem * Decimal256::permille(RATE);

    let withdrawal_fee = return_amount * Decimal256::percent(INSTANT_WITHDRAWAL_FEE);
    return_amount = return_amount.sub(withdrawal_fee);

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    // Correct withdraw, user has 1 ticket to be withdrawn
    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let empty_addr: Vec<Addr> = vec![];

    // Check address of sender was removed correctly in the sequence bucket
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("234567"))
            .unwrap()
            .holders,
        empty_addr
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap()),
        DepositorInfo {
            shares: Uint256::zero(),
            tickets: vec![],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::zero(),
            total_reserve: withdrawal_fee,
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: HOUR.mul(3).after(&mock_env().block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            last_lottery_execution_aust_exchange_rate: Decimal256::permille(RATE)
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_shares: Uint256::zero(),
            total_user_aust: Uint256::zero(),
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_operator_shares: Uint256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![
            SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: A_UST.to_string(),
                funds: vec![],
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: ANCHOR.to_string(),
                    amount: aust_to_redeem.into(),
                    msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
                })
                .unwrap(),
            })),
            SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                to_address: info.sender.to_string(),
                amount: vec![Coin {
                    denom: "uusd".to_string(),
                    amount: return_amount.into()
                }],
            }))
        ]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "withdraw_ticket"),
            attr("depositor", "addr0001"),
            attr("tickets_amount", 1u64.to_string()),
            attr("redeem_amount_anchor", aust_to_redeem.to_string()),
            attr("redeem_stable_amount", return_amount.to_string()),
            attr("instant_withdrawal_fee", withdrawal_fee.to_string())
        ]
    )
}

#[test]
fn claim() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Address buys one ticket
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ZERO_MATCH_SEQUENCE,
        )]),
        operator: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Address withdraws one ticket
    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: None,
        instant: None,
    };

    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Calculate the depositor's aust balance
    let depositor_aust_balance = minted_aust;

    // Calculate the depositor's balance from their aust balance
    let depositor_balance = depositor_aust_balance * Decimal256::permille(RATE);

    let redeemed_amount = depositor_balance;

    // Correct withdraw, user has 1 ticket to be withdrawn
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Get the amount to redeem and it's corresponding value
    let sent_amount = if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
        let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
        if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
            amount
        } else {
            panic!("DO NOT ENTER HERE")
        }
    } else {
        panic!("DO NOT ENTER HERE");
    };

    // Claim amount that you don't have, should fail
    let info = mock_info("addr0002", &[]);
    let msg = ExecuteMsg::Claim {};

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InsufficientClaimableFunds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Claim amount that you have, but still in unbonding state, should fail
    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Claim {};

    let mut env = mock_env();

    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone());
    match res {
        Err(ContractError::InsufficientClaimableFunds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    println!("Block time 1: {}", env.block.time);

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time * 2);
    }
    println!("Block time 2: {}", env.block.time);

    // Read the depositor info
    let dep = read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap());

    // Fail because not enough funds in the contract
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone());
    match res {
        Err(ContractError::InsufficientFunds {
            to_send,
            available_balance,
        }) => {
            if available_balance == Uint256::zero() && Uint256::from(to_send) == redeemed_amount {}
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Revert the state of the depositor
    store_depositor_info(
        &mut deps.storage,
        &deps.api.addr_validate("addr0001").unwrap(),
        dep,
        env.block.height,
    )
    .unwrap();

    // Update the contract balance to include the withdrawn funds
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: DENOM.to_string(),
            amount: Uint128::from(Uint256::from(sent_amount) * Decimal256::permille(RATE)),
        }],
    );

    // Claim amount is already unbonded, so claim execution should work
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap()),
        DepositorInfo {
            shares: Uint256::zero(),
            tickets: vec![],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
            to_address: "addr0001".to_string(),
            amount: vec![Coin {
                denom: String::from("uusd"),
                amount: redeemed_amount.into()
            }],
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "claim_unbonded"),
            attr("depositor", "addr0001"),
            attr("redeemed_amount", redeemed_amount.to_string()),
        ]
    );
}

#[test]
fn claim_lottery_single_winner() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Users buys winning ticket
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            SIX_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw = deps.api.addr_validate("addr0000").unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the amount of minted_shares
    let minted_shares = minted_aust;

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from(SIX_MATCH_SEQUENCE)],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    // Run lottery, one winner (5 hits) - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);

    //Advance time one week
    let mut env = mock_env();
    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(20_000_000u128),
        )],
    )]);

    let state_prize_buckets = calculate_prize_buckets(deps.as_ref());

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};

    let execute_lottery_block = env.block.clone();
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Check that state equals calculated prize
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, state_prize_buckets);

    // Get the amount of aust that is being redeemed
    let sent_amount = if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
        let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
        if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
            amount
        } else {
            panic!("DO NOT ENTER HERE")
        }
    } else {
        panic!("DO NOT ENTER HERE");
    };

    // Update contract_balance based on the amount of redeemed aust --

    // Increase the uusd balance by the value of the aust

    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(Uint256::from(sent_amount) * Decimal256::permille(RATE)),
        }],
    );

    // Decrease the aust balance by the amount of redeemed aust
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &(Uint128::from(20_000_000u128) - sent_amount),
        )],
    )]);

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let number_winners = [0, 0, 0, 0, 0, 0, 1];
    let (lottery_prize_buckets, total_reserve) =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners, RESERVE_FACTOR);
    let (glow_prize_buckets, _) =
        calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners, 0);

    let lottery = read_lottery_info(deps.as_ref().storage, 0u64);
    assert_eq!(
        lottery,
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: execute_lottery_block.time,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            block_height: execute_lottery_block.height,
            total_user_shares: minted_shares
        }
    );

    let prize_info = read_prize(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(
        prize_info,
        PrizeInfo {
            claimed: false,
            matches: number_winners,
        }
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, total_reserve);

    let remaining_state_prize_buckets =
        calculate_remaining_state_prize_buckets(state_prize_buckets, number_winners);

    // From the initialization of the contract
    assert_eq!(state.prize_buckets, remaining_state_prize_buckets);

    let info = mock_info("addr0000", &[]);
    let msg = ExecuteMsg::ClaimLottery {
        lottery_ids: Vec::from([0u64]),
    };

    // Claim lottery should work, even if there are no unbonded claims
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    let config = CONFIG.load(deps.as_ref().storage).unwrap();
    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);
    let snapshotted_depositor_stats_info = read_depositor_stats_at_height(
        deps.as_ref().storage,
        &info.sender,
        lottery_info.block_height,
    );

    let winner_address = info.sender;

    let (ust_to_send, glow_to_send): (Uint128, Uint128) = calculate_winner_prize(
        &deps.as_mut().querier,
        &config,
        &prize_info,
        &lottery_info,
        &snapshotted_depositor_stats_info,
        &winner_address,
    )
    .unwrap();

    let prizes = read_prize(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(
        prizes,
        PrizeInfo {
            claimed: true,
            matches: [0, 0, 0, 0, 0, 0, 1],
        }
    );

    let prize_response: PrizeInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            env,
            QueryMsg::PrizeInfo {
                address: "addr0000".to_string(),
                lottery_id: 0,
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(prize_response.won_ust, ust_to_send);
    assert_eq!(prize_response.won_glow, glow_to_send);

    //check total_reserve
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.total_reserve, total_reserve);

    assert_eq!(
        res.messages,
        vec![
            SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                to_address: "addr0000".to_string(),
                amount: vec![Coin {
                    denom: String::from("uusd"),
                    amount: ust_to_send,
                }],
            })),
            SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.distributor_contract.to_string(),
                funds: vec![],
                msg: to_binary(&FaucetExecuteMsg::Spend {
                    recipient: "addr0000".to_string(),
                    amount: glow_to_send,
                })
                .unwrap(),
            }))
        ]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "claim_lottery"),
            attr("lottery_ids", "[0]"),
            attr("depositor", "addr0000"),
            attr("redeemed_ust", ust_to_send.to_string()),
            attr("redeemed_glow", glow_to_send.to_string()),
        ]
    );
}

#[test]
fn execute_lottery() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::zero())],
    )]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(1u64),
        }],
    );

    let msg = ExecuteMsg::ExecuteLottery {};

    let res = execute(deps.as_mut(), mock_env(), info, msg.clone());

    match res {
        Err(ContractError::InvalidLotteryExecutionFunds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let mut env = mock_env();
    let info = mock_info("addr0001", &[]);
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone());

    match res {
        Err(ContractError::LotteryNotReady { next_lottery_time })
            if next_lottery_time
                == Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let res = execute(deps.as_mut(), env.clone(), info, msg);

    // Lottery cannot be run with 0 tickets participating
    match res {
        Err(ContractError::InvalidLotteryExecutionTickets {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Correct deposit - buys two tickets
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Calculate the number of minted_aust
    let minted_aust = Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE);

    // Add minted_aust to our contract balance
    deps.querier.increment_token_balance(
        A_UST.to_string(),
        MOCK_CONTRACT_ADDR.to_string(),
        minted_aust.into(),
    );

    // Execute lottery, now with tickets
    let lottery_msg = ExecuteMsg::ExecuteLottery {};
    let info = mock_info("addr0001", &[]);

    let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

    // Get the sent_amount
    let sent_amount = if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
        let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
        if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
            amount
        } else {
            panic!("DO NOT ENTER HERE")
        }
    } else {
        panic!("DO NOT ENTER HERE");
    };

    // Messages is empty because aust hasn't appreciated so there is nothing to redeem
    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_lottery"),
            attr("redeemed_amount", sent_amount.to_string()),
        ]
    );

    deps.querier.decrement_token_balance(
        A_UST.to_string(),
        MOCK_CONTRACT_ADDR.to_string(),
        sent_amount,
    );

    // Directly check next_lottery_exec_time has been set up
    let next_lottery_exec_time = query_state(deps.as_ref(), mock_env(), None)
        .unwrap()
        .next_lottery_exec_time;

    assert_eq!(
        next_lottery_exec_time,
        Expiration::AtTime(env.block.time).add(HOUR).unwrap()
    );

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    // Execute prize
    let execute_prize_msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg.clone()).unwrap();

    // Directly check next_lottery_time has been set up for next week
    let next_lottery_time = query_state(deps.as_ref(), mock_env(), None)
        .unwrap()
        .next_lottery_time;

    assert_eq!(
        next_lottery_time,
        Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME))
            .add(WEEK)
            .unwrap()
    );

    // Directly check next_lottery_exec_time has been set up to Never
    let next_lottery_exec_time = query_state(deps.as_ref(), mock_env(), None)
        .unwrap()
        .next_lottery_exec_time;

    assert_eq!(next_lottery_exec_time, Expiration::Never {});

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr("total_awarded_prize", "0"),
        ]
    );

    // Increase the aust exchange rate
    let new_rate = Decimal256::permille(RATE * 2);
    deps.querier.with_exchange_rate(new_rate);

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    // Execute 2nd lottery
    let lottery_msg = ExecuteMsg::ExecuteLottery {};
    let info = mock_info("addr0001", &[]);

    let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

    // Amount of aust to redeem

    // Get this contracts aust balance
    let contract_a_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    let config = CONFIG.load(deps.as_ref().storage).unwrap();
    let pool = POOL.load(deps.as_ref().storage).unwrap();
    let state = STATE.load(deps.as_ref().storage).unwrap();
    let ExecuteLotteryRedeemedAustInfo { aust_to_redeem, .. } =
        calculate_value_of_aust_to_be_redeemed_for_lottery(
            &state,
            &pool,
            &config,
            contract_a_balance,
            new_rate,
        );

    // Verify amount to redeem for the lottery
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: A_UST.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: ANCHOR.to_string(),
                amount: Uint128::from(aust_to_redeem),
                msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
            })
            .unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_lottery"),
            attr("redeemed_amount", aust_to_redeem.to_string()),
        ]
    );

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    // Execute prize
    let _res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg.clone()).unwrap();

    // Directly check next_lottery_time has been set up for next week
    let next_lottery_time = query_state(deps.as_ref(), mock_env(), None)
        .unwrap()
        .next_lottery_time;

    assert_eq!(
        next_lottery_time,
        Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME))
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    println!("state: {:?}", state);

    // Advance three weeks in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time * 3);
    }

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(20_000_000u128),
        )],
    )]);

    // Get this contracts aust balance
    let contract_a_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    // Execute 3rd lottery
    let lottery_msg = ExecuteMsg::ExecuteLottery {};
    let info = mock_info("addr0001", &[]);

    let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

    let config = CONFIG.load(deps.as_ref().storage).unwrap();
    let pool = POOL.load(deps.as_ref().storage).unwrap();
    let state = STATE.load(deps.as_ref().storage).unwrap();
    let ExecuteLotteryRedeemedAustInfo { aust_to_redeem, .. } =
        calculate_value_of_aust_to_be_redeemed_for_lottery(
            &state,
            &pool,
            &config,
            contract_a_balance,
            new_rate,
        );

    // Check the attributes
    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_lottery"),
            attr("redeemed_amount", aust_to_redeem.to_string()),
        ]
    );

    // Verify amount to redeem for the lottery
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: A_UST.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: ANCHOR.to_string(),
                amount: Uint128::from(aust_to_redeem),
                msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
            })
            .unwrap(),
        }))]
    );

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    // Execute prize
    let _res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg.clone()).unwrap();

    // Directly check next_lottery_time has been set up for three weeks from the last lottery
    // This checks the functionality of ensuring that the next_lottery_time is always
    // set to a time in the future
    let next_lottery_time = query_state(deps.as_ref(), mock_env(), None)
        .unwrap()
        .next_lottery_time;

    assert_eq!(
        next_lottery_time,
        Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME))
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
    );

    // Advance to the next lottery time
    if let Expiration::AtTime(next_lottery_time_seconds) = next_lottery_time {
        env.block.time = next_lottery_time_seconds;
    };

    // Execute 4th lottery
    // Confirm that you can run the lottery right at the next execution time
    let lottery_msg = ExecuteMsg::ExecuteLottery {};
    let info = mock_info("addr0001", &[]);

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    // Execute prize
    let _res = execute(deps.as_mut(), env, info, execute_prize_msg).unwrap();

    // Directly check next_lottery_time has been set up one week from the last execution time
    let next_lottery_time = query_state(deps.as_ref(), mock_env(), None)
        .unwrap()
        .next_lottery_time;

    assert_eq!(
        next_lottery_time,
        Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME))
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
            .add(WEEK)
            .unwrap()
    );
}

#[test]
fn execute_lottery_no_tickets() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &Uint128::zero())],
    )]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let info = mock_info("addr0001", &[]);

    let mut env = mock_env();
    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};

    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg);

    println!("res: {:?}", res);
    match res {
        Err(ContractError::InvalidLotteryExecutionTickets {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    // Run lottery, no winners - should run correctly
    let res = execute(deps.as_mut(), env, info, msg);
    match res {
        Err(ContractError::InvalidLotteryPrizeExecution {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

#[test]
fn execute_prize_no_winners() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Users buys a non-winning ticket
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ZERO_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw = deps.api.addr_validate("addr0000").unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the amount of minted_shares
    let minted_shares = minted_aust;

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from(ZERO_MATCH_SEQUENCE)],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    //Advance time one week
    let mut env = mock_env();
    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(20_000_000u128),
        )],
    )]);

    // Calculate the prize buckets
    let state_prize_buckets = calculate_prize_buckets(deps.as_ref());

    // Execute lottery - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);
    let msg = ExecuteMsg::ExecuteLottery {};

    let execute_lottery_block = env.block.clone();
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Check that state equals calculated prize
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, state_prize_buckets);

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Check lottery info was updated correctly
    let awarded_prize = Uint256::zero();
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            number_winners: [0; NUM_PRIZE_BUCKETS],
            page: "".to_string(),
            glow_prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            timestamp: execute_lottery_block.time,
            block_height: execute_lottery_block.height,
            total_user_shares: minted_shares
        }
    );

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Uint256::zero());

    // After executing the lottery, the prize buckets remain unchanged because there were no winning tickets
    assert_eq!(state.prize_buckets, state_prize_buckets);

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr("total_awarded_prize", awarded_prize.to_string()),
        ]
    );
}

#[test]
fn execute_prize_one_winner() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Users buys winning ticket
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            SIX_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw = deps.api.addr_validate("addr0000").unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the amount of minted_shares
    let minted_shares = minted_aust;

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from(SIX_MATCH_SEQUENCE)],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    // Run lottery, one winner (5 hits) - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);

    //Advance time one week
    let mut env = mock_env();

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(20_000_000u128),
        )],
    )]);

    // Check lottery info was updated correctly
    let state_prize_buckets = calculate_prize_buckets(deps.as_ref());

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};

    let execute_lottery_block = env.block.clone();
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }
    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let number_winners = [0, 0, 0, 0, 0, 0, 1];
    let (lottery_prize_buckets, total_reserve) =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners, RESERVE_FACTOR);

    let (glow_prize_buckets, _) =
        calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners, 0);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            timestamp: execute_lottery_block.time,
            block_height: execute_lottery_block.height,
            total_user_shares: minted_shares,
        }
    );

    let prizes = read_prize(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(prizes.matches, number_winners);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, total_reserve);

    let remaining_state_prize_buckets =
        calculate_remaining_state_prize_buckets(state_prize_buckets, number_winners);

    // From the initialization of the contract
    assert_eq!(state.prize_buckets, remaining_state_prize_buckets);

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr(
                "total_awarded_prize",
                lottery_prize_buckets[NUM_PRIZE_BUCKETS - 1].to_string()
            ),
        ]
    );
}

#[test]
fn execute_prize_winners_diff_ranks() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Users buys winning ticket - 5 hits
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            SIX_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_0 = deps.api.addr_validate("addr0000").unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the amount of minted_shares
    let minted_shares = minted_aust;

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_0),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from(SIX_MATCH_SEQUENCE)],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    // Users buys winning ticket - 2 hits
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            TWO_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_1 = deps.api.addr_validate("addr0001").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_1),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from(TWO_MATCH_SEQUENCE)],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    // Run lottery, one winner (6 hits), one winner (2 hits) - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);
    //Advance time one week
    let mut env = mock_env();
    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(30_000_000u128),
        )],
    )]);

    // Get the prize buckets
    let state_prize_buckets = calculate_prize_buckets(deps.as_ref());

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};

    let execute_lottery_block = env.block.clone();
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let number_winners = [0, 0, 1, 0, 0, 0, 1];
    let (lottery_prize_buckets, _total_reserve) =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners, RESERVE_FACTOR);
    let (glow_prize_buckets, _) =
        calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners, 0);

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    let each_shares_amount = minted_aust;

    // calculate the total minted_aust_value
    let total_minted_shares = Uint256::from(2u128) * each_shares_amount;

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            timestamp: execute_lottery_block.time,
            block_height: execute_lottery_block.height,
            total_user_shares: total_minted_shares,
        }
    );

    let prizes = read_prize(deps.as_ref(), &address_raw_0, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 0, 0, 0, 0, 1]);

    let prizes = read_prize(deps.as_ref(), &address_raw_1, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 1, 0, 0, 0, 0]);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(state.current_lottery, 1u64);

    let remaining_state_prize_buckets =
        calculate_remaining_state_prize_buckets(state_prize_buckets, number_winners);

    // From the initialization of the contract
    assert_eq!(state.prize_buckets, remaining_state_prize_buckets);

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr(
                "total_awarded_prize",
                lottery_prize_buckets
                    .iter()
                    .fold(Uint256::zero(), |sum, val| sum + *val)
                    .to_string()
            ),
        ]
    );
}

#[test]
fn execute_prize_winners_same_rank() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Users buys winning ticket - 4 hits
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            FOUR_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_0 = deps.api.addr_validate("addr0000").unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the amount of minted_shares
    let minted_shares = minted_aust;

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_0),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from(FOUR_MATCH_SEQUENCE)],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    // Users buys winning ticket - 4 hits
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            FOUR_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_1 = deps.api.addr_validate("addr0001").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_1),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from(FOUR_MATCH_SEQUENCE)],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    // Run lottery, 2 winners (4 hits) - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);

    let mut env = mock_env();
    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(30_000_000u128),
        )],
    )]);

    let state_prize_buckets = calculate_prize_buckets(deps.as_ref());

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};

    let execute_lottery_block = env.block.clone();
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Check that state equals calculated prize
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, state_prize_buckets);

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let number_winners = [0, 0, 0, 0, 2, 0, 0];
    let (lottery_prize_buckets, total_reserve) =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners, RESERVE_FACTOR);

    let (glow_prize_buckets, _) =
        calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners, 0);

    // calculate the value of each deposit accounting for rounding errors
    let each_minted_shares = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // calculate the total minted_aust_value
    let total_minted_shares = Uint256::from(2u128) * each_minted_shares;

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: execute_lottery_block.time,
            block_height: execute_lottery_block.height,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            total_user_shares: total_minted_shares
        }
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, total_reserve);

    let remaining_state_prize_buckets =
        calculate_remaining_state_prize_buckets(state_prize_buckets, number_winners);

    // Check award_available
    assert_eq!(state.prize_buckets, remaining_state_prize_buckets);

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr(
                "total_awarded_prize",
                lottery_prize_buckets
                    .iter()
                    .fold(Uint256::zero(), |sum, val| sum + *val)
                    .to_string()
            ),
        ]
    );
}

#[test]
fn execute_prize_one_winner_multiple_ranks() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Users buys winning ticket - 6 hits
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            SIX_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ONE_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            FOUR_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            FOUR_MATCH_SEQUENCE_2,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            FOUR_MATCH_SEQUENCE_3,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw = deps.api.addr_validate("addr0000").unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the amount of minted_shares
    let minted_shares = minted_aust * Uint256::from(5u128);

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![
                String::from(SIX_MATCH_SEQUENCE),
                String::from(ONE_MATCH_SEQUENCE),
                String::from(FOUR_MATCH_SEQUENCE),
                String::from(FOUR_MATCH_SEQUENCE_2),
                String::from(FOUR_MATCH_SEQUENCE_3),
            ],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    let mut env = mock_env();
    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(55_000_000u128),
        )],
    )]);

    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);

    // Get state prize buckets
    let state_prize_buckets = calculate_prize_buckets(deps.as_ref());

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let execute_lottery_block = env.block.clone();
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Check that state equals calculated prize
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, state_prize_buckets);

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let number_winners = [0, 0, 0, 0, 3, 0, 1];
    let (lottery_prize_buckets, total_reserve) =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners, RESERVE_FACTOR);

    let (glow_prize_buckets, _) =
        calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners, 0);

    println!(
        "lottery_info: {:x?}",
        read_lottery_info(deps.as_ref().storage, 0u64)
    );

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: execute_lottery_block.time,
            block_height: execute_lottery_block.height,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            total_user_shares: minted_shares
        }
    );

    let prizes = read_prize(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(prizes.matches, number_winners);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, total_reserve);

    let remaining_state_prize_buckets =
        calculate_remaining_state_prize_buckets(state_prize_buckets, number_winners);

    // From the initialization of the contract
    assert_eq!(state.prize_buckets, remaining_state_prize_buckets);

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr(
                "total_awarded_prize",
                lottery_prize_buckets
                    .iter()
                    .fold(Uint256::zero(), |sum, val| sum + *val)
                    .to_string()
            ),
        ]
    );
}

#[test]
fn execute_prize_multiple_winners_one_ticket() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            SIX_MATCH_SEQUENCE,
        )]),
        operator: None,
    };

    // User 0 buys winning ticket - 5 hits
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap();

    // User 1 buys winning ticket - 5 hits
    let info = mock_info(
        "addr1111",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap();

    // User 2 buys winning ticket - 5 hits
    let info = mock_info(
        "addr2222",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // calculate the value of each deposit accounting for rounding errors
    let each_lottery_shares = minted_aust;

    // calculate the total minted_aust_value
    let total_minted_shares = Uint256::from(3u128) * each_lottery_shares;

    let address_0 = deps.api.addr_validate("addr0000").unwrap();
    let address_1 = deps.api.addr_validate("addr1111").unwrap();
    let address_2 = deps.api.addr_validate("addr2222").unwrap();

    let ticket = query_ticket_info(deps.as_ref(), String::from(SIX_MATCH_SEQUENCE)).unwrap();

    assert_eq!(
        ticket.holders,
        vec![address_0.clone(), address_1, address_2]
    );

    let mut env = mock_env();
    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(31_000_000u128),
        )],
    )]);

    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);

    // Get state prize buckets
    let state_prize_buckets = calculate_prize_buckets(deps.as_ref());

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let execute_lottery_block = env.block.clone();
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Check that state equals calculated prize
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, state_prize_buckets);

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let number_winners = [0, 0, 0, 0, 0, 0, 3];
    let (lottery_prize_buckets, total_reserve) =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners, RESERVE_FACTOR);
    let (glow_prize_buckets, _) =
        calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners, 0);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: execute_lottery_block.time,
            block_height: execute_lottery_block.height,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            total_user_shares: total_minted_shares
        }
    );

    let prizes = read_prize(deps.as_ref(), &address_0, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 0, 0, 0, 0, 1]);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, total_reserve);

    let remaining_state_prize_buckets =
        calculate_remaining_state_prize_buckets(state_prize_buckets, number_winners);

    // From the initialization of the contract
    assert_eq!(state.prize_buckets, remaining_state_prize_buckets);

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr(
                "total_awarded_prize",
                lottery_prize_buckets
                    .iter()
                    .fold(Uint256::zero(), |sum, val| sum + *val)
                    .to_string()
            ),
        ]
    );
}

#[test]
fn execute_prize_pagination() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let addresses_count = 480u64;
    let addresses_range = 0..addresses_count;
    let addresses = addresses_range
        .map(|c| format!("addr{:0>4}", c))
        .collect::<Vec<String>>();

    for (index, address) in addresses.iter().enumerate() {
        // Users buys winning ticket
        let msg = ExecuteMsg::Deposit {
            encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![format!(
                "be{:0length$}",
                100 + index,
                length = TICKET_LENGTH - 2
            )]),
            operator: None,
        };
        let info = mock_info(
            address.as_str(),
            &[Coin {
                denom: "uusd".to_string(),
                amount: Uint256::from(TICKET_PRICE).into(),
            }],
        );

        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    }

    // Run lottery - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);

    let mut env = mock_env();
    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    //Add aterra balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(16_000_000_000u128),
        )],
    )]);

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize {
        limit: Some(100u32),
    };
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);

    // println!("lottery_info: {:x?}", lottery_info);
    assert!(!lottery_info.awarded);

    // Second pagination round
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    // let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);

    // println!("lottery_info: {:x?}", lottery_info);
    // Third pagination round
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly
    // let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);
    // println!("lottery_info: {:x?}", lottery_info);

    // Fourth pagination round
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    // let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);
    // println!("lottery_info: {:x?}", lottery_info);

    // Fifth pagination round
    let _res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Check lottery info was updated correctly

    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);

    // println!("lottery_info: {:x?}", lottery_info);

    assert!(lottery_info.awarded);
}

#[test]
fn test_premature_emissions() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    let mut env = mock_env();

    mock_instantiate(&mut deps);
    // don't mock_register_contracts

    let info = mock_info("addr0000", &[]);

    // Try running epoch_operations, it should because contracts aren't registered

    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }
    let msg = ExecuteMsg::ExecuteEpochOps {};
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg);
    match res {
        Err(ContractError::NotRegistered {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Contracts not registered, so claiming rewards is an error
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), env.clone(), info, msg);

    match res {
        Err(ContractError::NotRegistered {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Deposit of 20_000_000 uusd
    let msg = ExecuteMsg::Sponsor {
        award: None,
        prize_distribution: None,
    };

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg);

    // Get the number of minted aust
    let minted_aust = Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    // After 100 blocks
    env.block.height += 100;

    // Assert that the global_reward_index is still 0
    let state = query_state(deps.as_ref(), env.clone(), Some(env.block.height)).unwrap();
    assert_eq!(
        state.operator_reward_emission_index.glow_emission_rate,
        Decimal256::zero()
    );
    assert_eq!(
        state.sponsor_reward_emission_index.glow_emission_rate,
        Decimal256::zero()
    );

    // Register contracts
    mock_register_contracts(deps.as_mut());

    // Execute epoch ops

    let msg = ExecuteMsg::ExecuteEpochOps {};
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();
    assert_eq!(
        state.operator_reward_emission_index.last_reward_updated,
        env.block.height
    );
    assert_eq!(
        state.sponsor_reward_emission_index.last_reward_updated,
        env.block.height
    );

    let state = query_state(deps.as_ref(), env.clone(), Some(env.block.height)).unwrap();
    assert_eq!(
        state.operator_reward_emission_index.glow_emission_rate,
        Decimal256::zero()
    );
    assert_eq!(
        state.sponsor_reward_emission_index.glow_emission_rate,
        Decimal256::zero()
    );

    // Increase glow emission rate
    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.operator_reward_emission_index.glow_emission_rate = Decimal256::one();
    state.sponsor_reward_emission_index.glow_emission_rate = Decimal256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    // User has deposits but zero blocks have passed, so no rewards accrued
    let info = mock_info("addr0000", &[]);
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();
    assert_eq!(res.messages.len(), 0);

    // After 100 blocks from this point, the user earns some emissions
    env.block.height += 100;
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "addr0000".to_string(),
                amount: (Decimal256::from_str("100").unwrap()
                    / Decimal256::from_uint256(minted_lottery_aust_value)
                    * Decimal256::from_uint256(minted_lottery_aust_value)
                    * Uint256::one())
                .into(),
            })
            .unwrap(),
        }))]
    );

    let res: SponsorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            env,
            QueryMsg::Sponsor {
                address: "addr0000".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(res.pending_rewards, Decimal256::zero());
    assert_eq!(
        res.reward_index,
        (Decimal256::from_str("100").unwrap()
            / Decimal256::from_uint256(minted_lottery_aust_value))
    );
}

#[test]
fn claim_rewards_one_sponsor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let info = mock_info("addr0000", &[]);

    /*
    STATE.update(deps.as_mut().storage,  |mut state| {
        state.glow_emission_rate = Decimal256::one();
        Ok(state.unwrap())
    }).unwrap();
     */
    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.operator_reward_emission_index.glow_emission_rate = Decimal256::one();
    state.sponsor_reward_emission_index.glow_emission_rate = Decimal256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    // User has no deposits, so no claimable rewards and empty msg returned
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(res.messages.len(), 0);

    // Deposit of 20_000_000 uusd
    let msg = ExecuteMsg::Sponsor {
        award: None,
        prize_distribution: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );

    let mut env = mock_env();

    let _res = execute(deps.as_mut(), env.clone(), info, msg);

    // User has deposits but zero blocks have passed, so no rewards accrued
    let info = mock_info("addr0000", &[]);
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();
    assert_eq!(res.messages.len(), 0);

    // After 100 blocks
    env.block.height += 100;
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "addr0000".to_string(),
                amount: (Decimal256::from_str("100").unwrap()
                    / Decimal256::from_uint256(minted_lottery_aust_value)
                    * Decimal256::from_uint256(minted_lottery_aust_value)
                    * Uint256::one())
                .into(),
            })
            .unwrap(),
        }))]
    );

    let res: SponsorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Sponsor {
                address: "addr0000".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(res.pending_rewards, Decimal256::zero());
    assert_eq!(
        res.reward_index,
        (Decimal256::from_str("100").unwrap()
            / Decimal256::from_uint256(minted_lottery_aust_value))
    );
}

#[test]
fn claim_rewards_one_referrer() {
    // Initialize contract
    let mut deps = mock_dependencies(&[Coin {
        denom: DENOM.to_string(),
        amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
    }]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let info = mock_info("operator", &[]);

    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.operator_reward_emission_index.glow_emission_rate = Decimal256::one();
    state.sponsor_reward_emission_index.glow_emission_rate = Decimal256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    // User has no deposits, so no claimable rewards and empty msg returned
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(res.messages.len(), 0);

    // Deposit of 20_000_000 uusd
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(THREE_MATCH_SEQUENCE),
            String::from(ZERO_MATCH_SEQUENCE),
        ]),
        operator: Some(String::from("operator")),
    };

    let deposit_amount = Uint256::from(2 * TICKET_PRICE).into();

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );

    let mut env = mock_env();

    let _res = execute(deps.as_mut(), env.clone(), info, msg);

    // User has deposits but zero blocks have passed, so no rewards accrued
    let info = mock_info("operator", &[]);
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
    assert_eq!(res.messages.len(), 0);

    // After 100 blocks
    env.block.height += 100;

    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR.to_string(),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: deposit_amount,
        }],
    );

    let minted_aust = Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE);

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    // User withdraws all its deposits
    let msg = ExecuteMsg::Withdraw {
        amount: None,
        instant: None,
    };
    let info = mock_info("addr0000", &[]);

    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Another 100 blocks pass
    env.block.height += 100;

    let info = mock_info("operator", &[]);
    let msg = ExecuteMsg::ClaimRewards {};

    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE);

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "operator".to_string(),
                amount: (Decimal256::from_str("100").unwrap()
                    / Decimal256::from_uint256(minted_aust)
                    * Decimal256::from_uint256(minted_aust)
                    * Uint256::one())
                .into(),
            })
            .unwrap(),
        }))]
    );

    let res: OperatorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Operator {
                address: "operator".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(res.pending_rewards, Decimal256::zero());
    assert_eq!(
        res.reward_index,
        (Decimal256::from_str("100").unwrap() / Decimal256::from_uint256(minted_aust))
    );
}

#[test]
fn execute_epoch_operations() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    deps.querier.with_tax(
        Decimal::percent(1),
        &[(&"uusd".to_string(), &Uint128::from(1000000u128))],
    );

    let info = mock_info("addr0000", &[]);
    let mut env = mock_env();

    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.total_reserve = Uint256::from(500u128);
    STATE.save(deps.as_mut().storage, &state).unwrap();
    env.block.height += 100;

    // fails, next epoch time not expired
    let msg = ExecuteMsg::ExecuteEpochOps {};
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone());
    match res {
        Err(ContractError::InvalidEpochExecution {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    //Advance to next epoch
    if let Duration::Time(time) = (WEEK + HOUR).unwrap() {
        env.block.time = env.block.time.plus_seconds(time);
    }
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
            to_address: COMMUNITY_ADDR.to_string(),
            amount: vec![Coin {
                denom: DENOM.to_string(),
                amount: Uint128::from(495u128), // 1% tax
            }],
        }))]
    );

    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();
    // Glow Emission rate must be 1 as hard-coded in mock querier
    assert_eq!(
        state,
        StateResponse {
            total_tickets: Uint256::zero(),
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: HOUR.mul(3).after(&env.block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12445,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12445,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            last_lottery_execution_aust_exchange_rate: Decimal256::permille(RATE)
        }
    );
}

#[test]
fn small_withdraw() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // get env
    let env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ONE_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Add the funds to the contract address -------------------

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the amount of minted_shares
    let minted_shares = minted_aust;

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    // Compare total user savings aust with contract_a_balance -----------

    let pool = query_pool(deps.as_ref()).unwrap();
    let contract_a_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    // Total user savings aust should equal contract_a_balance minus contract_a_balance times split_factor
    assert_eq!(pool.total_user_shares, contract_a_balance);

    // Check that the depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0001").unwrap()
        ),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from(ONE_MATCH_SEQUENCE)],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(1u64),
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: HOUR.mul(3).after(&mock_env().block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            last_lottery_execution_aust_exchange_rate: Decimal256::permille(RATE)
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_shares: minted_shares,
            total_user_aust: minted_shares,
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_operator_shares: Uint256::zero(),
        }
    );

    // Address withdraws a small amount of money ----------------

    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: Some(10u128.into()),
        instant: None,
    };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Get the withdrawn shares and aust
    let withdrawn_shares = Uint256::from(10u128) / Decimal256::permille(RATE);
    let withdrawn_aust = withdrawn_shares;

    // Message for redeem amount operation of aUST

    // Get the sent_amount
    let sent_amount = if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
        let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
        if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
            amount
        } else {
            panic!("DO NOT ENTER HERE")
        }
    } else {
        panic!("DO NOT ENTER HERE");
    };

    assert_eq!(Uint256::from(sent_amount), withdrawn_aust);

    // Update contract_balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &(contract_a_balance - sent_amount.into()).into(),
        )],
    )]);

    // Check that the depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0001").unwrap()
        ),
        DepositorInfo {
            shares: minted_shares - withdrawn_shares,
            tickets: vec![],
            unbonding_info: vec![Claim {
                amount: Uint256::from(sent_amount) * Decimal256::permille(RATE),
                release_at: WEEK.after(&env.block),
            }],
            operator_addr: Addr::unchecked("")
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(0u64),
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: HOUR.mul(3).after(&mock_env().block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            last_lottery_execution_aust_exchange_rate: Decimal256::permille(RATE)
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_shares: minted_shares - withdrawn_shares,
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_user_aust: minted_aust - withdrawn_aust,
            total_operator_shares: Uint256::zero(),
        }
    );
}

#[test]
pub fn lottery_deposit_floor_edge_case() {
    let small_ticket_price = 9590u128;

    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // get env
    let env = mock_env();

    // mock instantiate the contracts
    mock_instantiate_small_ticket_price(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(small_ticket_price).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ONE_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // User deposits and buys one ticket again-------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(small_ticket_price).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            TWO_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Add AUST to the contract

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &(10_000_000u128).into())],
    )]);

    // Address withdraws all their money ----------------
    // this would fail with an underflow error
    // with the previous implementation

    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: None,
        instant: Some(true),
    };
    let _res = execute(deps.as_mut(), env, info, msg).unwrap();
}

#[test]
pub fn lottery_pool_solvency_edge_case() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    let special_rate = Decimal256::from_str(".05234").unwrap();

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(special_rate);

    // get env
    let env = mock_env();

    // mock instantiate the contracts
    mock_instantiate_small_ticket_price(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(SMALL_TICKET_PRICE).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            ONE_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Add the funds to the contract address -------------------

    // Get the number of minted aust
    let minted_aust = Uint256::from(SMALL_TICKET_PRICE) / special_rate;

    let minted_shares = minted_aust;

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    // Compare total_user_savings_aust with contract_a_balance -----------

    let pool = query_pool(deps.as_ref()).unwrap();
    let contract_a_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    assert_eq!(pool.total_user_shares, minted_shares,);
    assert_eq!(pool.total_user_aust, contract_a_balance,);
    assert_eq!(minted_aust, contract_a_balance,);

    // Check that the depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0001").unwrap()
        ),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from(ONE_MATCH_SEQUENCE)],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(1u64),
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: HOUR.mul(3).after(&mock_env().block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            last_lottery_execution_aust_exchange_rate: special_rate
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_shares: minted_shares,
            total_user_aust: minted_shares,
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_operator_shares: Uint256::zero(),
        }
    );

    // Address withdraws a quarter of their money ----------------

    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: Some((SMALL_TICKET_PRICE / 4).into()),
        instant: None,
    };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Message for redeem amount operation of aUST

    // Get the sent_amount
    let sent_amount = if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
        let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
        if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
            amount
        } else {
            panic!("DO NOT ENTER HERE")
        }
    } else {
        panic!("DO NOT ENTER HERE");
    };

    // Update contract_balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &(contract_a_balance - sent_amount.into()).into(),
        )],
    )]);

    // Verify that Anchor Pool is solvent

    let contract_a_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    // Assert that Lotto pool is solvent
    // No easy way of doing this unfortunately
    assert!(contract_a_balance * special_rate >= Uint256::from(SMALL_TICKET_PRICE * 3 / 4));
}

#[test]
pub fn simulate_many_lotteries_with_one_depositor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    let mut exchange_rate = Decimal256::from_str("1").unwrap();
    deps.querier.with_exchange_rate(exchange_rate);

    let num_weeks = 52;

    let weekly_rate_multiplier =
        Decimal256::from_str(&(1.2f64).powf(1.0 / (num_weeks as f64)).to_string()).unwrap();

    // Mock aUST-UST exchange rate

    // get env
    let mut env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            TWO_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Calculate the number of minted_aust
    let minted_aust = Uint256::from(TICKET_PRICE) / exchange_rate;

    let mut contract_balance = minted_aust;

    let mut amount_distributed_through_lottery = Uint256::zero();

    // Add the funds to the contract address -------------------
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
    )]);

    for i in 0..num_weeks {
        // Get the pool size appreciation
        let pool_size_appreciation = contract_balance * (exchange_rate * weekly_rate_multiplier)
            - contract_balance * exchange_rate;

        // Update the exchange rate
        exchange_rate = exchange_rate * weekly_rate_multiplier;

        // Mock aUST-UST exchange rate
        deps.querier.with_exchange_rate(exchange_rate);

        // Advance one week in time
        if let Duration::Time(time) = WEEK {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute the lottery

        let lottery_msg = ExecuteMsg::ExecuteLottery {};
        let info = mock_info("addr0001", &[]);
        let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

        // Check how much aust was redeemed
        let sent_amount =
            if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
                let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
                if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
                    amount
                } else {
                    panic!("DO NOT ENTER HERE")
                }
            } else {
                panic!("DO NOT ENTER HERE");
            };

        // Update the contract balance
        contract_balance = contract_balance - Uint256::from(sent_amount);

        // Add the funds to the contract address -------------------
        deps.querier.with_token_balances(&[(
            &A_UST.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
        )]);

        // Advance block_time in time
        if let Duration::Time(time) = HOUR {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute prize
        let execute_prize_msg = ExecuteMsg::ExecutePrize { limit: None };
        let _res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg).unwrap();

        amount_distributed_through_lottery += Uint256::from(sent_amount) * exchange_rate;

        let percent_appreciation_towards_lottery =
            Decimal256::from_uint256(Uint256::from(sent_amount)) * exchange_rate
                / Decimal256::from_uint256(pool_size_appreciation);

        assert!(
            percent_appreciation_towards_lottery
                >= Decimal256::percent(SPLIT_FACTOR) - Decimal256::permille(100)
                && percent_appreciation_towards_lottery <= Decimal256::percent(SPLIT_FACTOR)
        );

        if i % 5 == 0 {
            println!(
                "Percent appreciation towards lottery after week {}: {}",
                i, percent_appreciation_towards_lottery
            );
        }
    }

    println!("Initial pool size value: {}", minted_aust);
    println!(
        "Final pool size value: {}",
        contract_balance * exchange_rate
    );
    println!(
        "Total appreciation: {}",
        Decimal256::from_uint256(contract_balance) * exchange_rate
            / Decimal256::from_uint256(minted_aust)
    );
    println!(
        "Total spent on lottery: {}",
        amount_distributed_through_lottery
    );
    println!(
        "Percent of total appreciation towards lottery: {}",
        Decimal256::from_uint256(amount_distributed_through_lottery)
            / Decimal256::from_uint256(
                contract_balance * exchange_rate - minted_aust + amount_distributed_through_lottery
            )
    );

    println!("Withdrawing half of user deposits");

    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: Some((TICKET_PRICE / 2).into()),
        instant: Some(true),
    };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Add a dummy ticket in order to pass validation

    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.total_tickets = Uint256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    // Get the sent_amount
    let sent_amount = if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
        let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
        if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
            amount
        } else {
            panic!("DO NOT ENTER HERE")
        }
    } else {
        panic!("DO NOT ENTER HERE");
    };

    // Update the contract balance
    contract_balance = contract_balance - Uint256::from(sent_amount);

    // Add the funds to the contract address -------------------
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
    )]);

    for i in 0..num_weeks {
        // Get the pool size appreciation
        let pool_size_appreciation = contract_balance * (exchange_rate * weekly_rate_multiplier)
            - contract_balance * exchange_rate;

        // Update the exchange rate
        exchange_rate = exchange_rate * weekly_rate_multiplier;

        // Mock aUST-UST exchange rate
        deps.querier.with_exchange_rate(exchange_rate);

        // Advance one week in time
        if let Duration::Time(time) = WEEK {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute the lottery

        let lottery_msg = ExecuteMsg::ExecuteLottery {};
        let info = mock_info("addr0001", &[]);
        let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

        // Check how much aust was redeemed
        let sent_amount =
            if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
                let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
                if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
                    amount
                } else {
                    panic!("DO NOT ENTER HERE")
                }
            } else {
                panic!("DO NOT ENTER HERE");
            };

        // Update the contract balance
        contract_balance = contract_balance - Uint256::from(sent_amount);

        // Add the funds to the contract address -------------------
        deps.querier.with_token_balances(&[(
            &A_UST.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
        )]);

        // Advance block_time in time
        if let Duration::Time(time) = HOUR {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute prize
        let execute_prize_msg = ExecuteMsg::ExecutePrize { limit: None };
        let _res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg).unwrap();

        amount_distributed_through_lottery += Uint256::from(sent_amount) * exchange_rate;

        let percent_appreciation_towards_lottery =
            Decimal256::from_uint256(Uint256::from(sent_amount)) * exchange_rate
                / Decimal256::from_uint256(pool_size_appreciation);

        assert!(
            percent_appreciation_towards_lottery
                >= Decimal256::percent(SPLIT_FACTOR) - Decimal256::permille(100)
                && percent_appreciation_towards_lottery <= Decimal256::percent(SPLIT_FACTOR)
        );

        if i % 5 == 0 {
            println!(
                "Percent appreciation towards lottery after week {}: {}",
                i, percent_appreciation_towards_lottery
            );
        }
    }
}

#[test]
pub fn simulate_many_lotteries_with_one_sponsor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    let mut exchange_rate = Decimal256::from_str("1").unwrap();
    deps.querier.with_exchange_rate(exchange_rate);

    let num_weeks = 52;

    let weekly_rate_multiplier =
        Decimal256::from_str(&(1.2f64).powf(1.0 / (num_weeks as f64)).to_string()).unwrap();

    // Mock aUST-UST exchange rate

    // get env
    let mut env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Add a dummy ticket in order to pass validation

    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.total_tickets = Uint256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );
    let msg = ExecuteMsg::Sponsor {
        award: None,
        prize_distribution: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Calculate the number of minted_aust
    let minted_aust = Uint256::from(TICKET_PRICE) / exchange_rate;

    let mut contract_balance = minted_aust;

    let mut amount_distributed_through_lottery = Uint256::zero();

    // Add the funds to the contract address -------------------
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
    )]);

    for i in 0..num_weeks {
        // Get the pool size appreciation
        let pool_size_appreciation = contract_balance * (exchange_rate * weekly_rate_multiplier)
            - contract_balance * exchange_rate;

        // Update the exchange rate
        exchange_rate = exchange_rate * weekly_rate_multiplier;

        // Mock aUST-UST exchange rate
        deps.querier.with_exchange_rate(exchange_rate);

        // Advance one week in time
        if let Duration::Time(time) = WEEK {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute the lottery

        let lottery_msg = ExecuteMsg::ExecuteLottery {};
        let info = mock_info("addr0001", &[]);
        let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

        // Check how much aust was redeemed
        let sent_amount =
            if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
                let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
                if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
                    amount
                } else {
                    panic!("DO NOT ENTER HERE")
                }
            } else {
                panic!("DO NOT ENTER HERE");
            };

        // Update the contract balance
        contract_balance = contract_balance - Uint256::from(sent_amount);

        // Add the funds to the contract address -------------------
        deps.querier.with_token_balances(&[(
            &A_UST.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
        )]);

        // Advance block_time in time
        if let Duration::Time(time) = HOUR {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute prize
        let execute_prize_msg = ExecuteMsg::ExecutePrize { limit: None };
        let _res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg).unwrap();

        amount_distributed_through_lottery += Uint256::from(sent_amount) * exchange_rate;

        let percent_appreciation_towards_lottery =
            Decimal256::from_uint256(Uint256::from(sent_amount)) * exchange_rate
                / Decimal256::from_uint256(pool_size_appreciation);

        assert!(percent_appreciation_towards_lottery >= Decimal256::percent(99));

        if i % 5 == 0 {
            println!(
                "Percent appreciation towards lottery after week {}: {}",
                i, percent_appreciation_towards_lottery
            );
        }
    }

    println!("Initial pool size value: {}", minted_aust);
    println!(
        "Final pool size value: {}",
        contract_balance * exchange_rate
    );
    println!(
        "Total appreciation: {}",
        Decimal256::from_uint256(contract_balance) * exchange_rate
            / Decimal256::from_uint256(minted_aust)
    );
    println!(
        "Total spent on lottery: {}",
        amount_distributed_through_lottery
    );
    println!(
        "Percent of total appreciation towards lottery: {}",
        Decimal256::from_uint256(amount_distributed_through_lottery)
            / Decimal256::from_uint256(
                contract_balance * exchange_rate - minted_aust + amount_distributed_through_lottery
            )
    );
}

#[test]
pub fn simulate_many_lotteries_with_one_depositor_and_sponsor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    let mut exchange_rate = Decimal256::from_str("1").unwrap();
    deps.querier.with_exchange_rate(exchange_rate);

    let num_weeks = 52;

    let weekly_rate_multiplier =
        Decimal256::from_str(&(1.2f64).powf(1.0 / (num_weeks as f64)).to_string()).unwrap();

    // Mock aUST-UST exchange rate

    // get env
    let mut env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Add a dummy ticket in order to pass validation

    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.total_tickets = Uint256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );
    let msg = ExecuteMsg::Sponsor {
        award: None,
        prize_distribution: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Calculate the number of minted_aust
    let mut minted_aust = Uint256::from(TICKET_PRICE) / exchange_rate;

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(TICKET_PRICE).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from(
            TWO_MATCH_SEQUENCE,
        )]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Calculate the number of minted_aust
    minted_aust += Uint256::from(TICKET_PRICE) / exchange_rate;

    let mut contract_balance = minted_aust;

    let mut amount_distributed_through_lottery = Uint256::zero();

    // Add the funds to the contract address -------------------
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
    )]);

    for i in 0..num_weeks {
        // Get the pool size appreciation
        let pool_size_appreciation = contract_balance * (exchange_rate * weekly_rate_multiplier)
            - contract_balance * exchange_rate;

        // Update the exchange rate
        exchange_rate = exchange_rate * weekly_rate_multiplier;

        // Mock aUST-UST exchange rate
        deps.querier.with_exchange_rate(exchange_rate);

        // Advance one week in time
        if let Duration::Time(time) = WEEK {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute the lottery

        let lottery_msg = ExecuteMsg::ExecuteLottery {};
        let info = mock_info("addr0001", &[]);
        let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

        // Check how much aust was redeemed
        let sent_amount =
            if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
                let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
                if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
                    amount
                } else {
                    panic!("DO NOT ENTER HERE")
                }
            } else {
                panic!("DO NOT ENTER HERE");
            };

        // Update the contract balance
        contract_balance = contract_balance - Uint256::from(sent_amount);

        // Add the funds to the contract address -------------------
        deps.querier.with_token_balances(&[(
            &A_UST.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
        )]);

        // Advance block_time in time
        if let Duration::Time(time) = HOUR {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute prize
        let execute_prize_msg = ExecuteMsg::ExecutePrize { limit: None };
        let _res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg).unwrap();

        amount_distributed_through_lottery += Uint256::from(sent_amount) * exchange_rate;

        let percent_appreciation_towards_lottery =
            Decimal256::from_uint256(Uint256::from(sent_amount)) * exchange_rate
                / Decimal256::from_uint256(pool_size_appreciation);

        assert!(percent_appreciation_towards_lottery <= Decimal256::percent(88));

        if i % 5 == 0 {
            println!(
                "Percent appreciation towards lottery after week {}: {}",
                i, percent_appreciation_towards_lottery
            );
        }
    }

    println!("Initial pool size value: {}", minted_aust);
    println!(
        "Final pool size value: {}",
        contract_balance * exchange_rate
    );
    println!(
        "Total appreciation: {}",
        Decimal256::from_uint256(contract_balance) * exchange_rate
            / Decimal256::from_uint256(minted_aust)
    );
    println!(
        "Total spent on lottery: {}",
        amount_distributed_through_lottery
    );
    println!(
        "Percent of total appreciation towards lottery: {}",
        Decimal256::from_uint256(amount_distributed_through_lottery)
            / Decimal256::from_uint256(
                contract_balance * exchange_rate - minted_aust + amount_distributed_through_lottery
            )
    );

    println!("Withdrawing half of user deposits");

    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: Some((TICKET_PRICE / 2).into()),
        instant: Some(true),
    };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Add a dummy ticket in order to pass validation

    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.total_tickets = Uint256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    // Get the sent_amount
    let sent_amount = if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
        let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
        if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
            amount
        } else {
            panic!("DO NOT ENTER HERE")
        }
    } else {
        panic!("DO NOT ENTER HERE");
    };

    // Update the contract balance
    contract_balance = contract_balance - Uint256::from(sent_amount);

    // Add the funds to the contract address -------------------
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
    )]);

    for i in 0..num_weeks {
        // Get the pool size appreciation
        let pool_size_appreciation = contract_balance * (exchange_rate * weekly_rate_multiplier)
            - contract_balance * exchange_rate;

        // Update the exchange rate
        exchange_rate = exchange_rate * weekly_rate_multiplier;

        // Mock aUST-UST exchange rate
        deps.querier.with_exchange_rate(exchange_rate);

        // Advance one week in time
        if let Duration::Time(time) = WEEK {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute the lottery

        let lottery_msg = ExecuteMsg::ExecuteLottery {};
        let info = mock_info("addr0001", &[]);
        let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

        // Check how much aust was redeemed
        let sent_amount =
            if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
                let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
                if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
                    amount
                } else {
                    panic!("DO NOT ENTER HERE")
                }
            } else {
                panic!("DO NOT ENTER HERE");
            };

        // Update the contract balance
        contract_balance = contract_balance - Uint256::from(sent_amount);

        // Add the funds to the contract address -------------------
        deps.querier.with_token_balances(&[(
            &A_UST.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
        )]);

        // Advance block_time in time
        if let Duration::Time(time) = HOUR {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute prize
        let execute_prize_msg = ExecuteMsg::ExecutePrize { limit: None };
        let _res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg).unwrap();

        amount_distributed_through_lottery += Uint256::from(sent_amount) * exchange_rate;

        let percent_appreciation_towards_lottery =
            Decimal256::from_uint256(Uint256::from(sent_amount)) * exchange_rate
                / Decimal256::from_uint256(pool_size_appreciation);

        // assert!(percent_appreciation_towards_lottery <= Decimal256::percent(90));

        if i % 5 == 0 {
            println!(
                "Percent appreciation towards lottery after week {}: {}",
                i, percent_appreciation_towards_lottery
            );
        }
    }
}

#[test]
pub fn simulate_jackpot_growth_with_one_depositor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    let mut exchange_rate = Decimal256::from_str("1").unwrap();
    deps.querier.with_exchange_rate(exchange_rate);

    let num_weeks = 52;

    let weekly_rate_multiplier =
        Decimal256::from_str(&(1.2f64).powf(1.0 / (num_weeks as f64)).to_string()).unwrap();

    // Mock aUST-UST exchange rate

    // get env
    let mut env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(3 * TICKET_PRICE).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(THREE_MATCH_SEQUENCE),
            String::from(FOUR_MATCH_SEQUENCE),
        ]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Calculate the number of minted_aust
    let minted_aust = Uint256::from(3 * TICKET_PRICE) / exchange_rate;

    let mut contract_balance = minted_aust;

    let mut amount_distributed_through_lottery = Uint256::zero();

    // Add the funds to the contract address -------------------
    // This overwrites the aust from INITIAL_DEPOSIT_AMOUNT
    // but that is good for making it easier to interpret the results of the lottery
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
    )]);

    for i in 0..num_weeks {
        // Get the pool size appreciation
        let pool_size_appreciation = contract_balance * (exchange_rate * weekly_rate_multiplier)
            - contract_balance * exchange_rate;

        // Update the exchange rate
        exchange_rate = exchange_rate * weekly_rate_multiplier;

        // Mock aUST-UST exchange rate
        deps.querier.with_exchange_rate(exchange_rate);

        // Advance one week in time
        if let Duration::Time(time) = WEEK {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute the lottery

        let lottery_msg = ExecuteMsg::ExecuteLottery {};
        let info = mock_info("addr0001", &[]);
        let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

        if i % 5 == 0 {
            // Remaining prizes in state

            let state = STATE.load(deps.as_ref().storage).unwrap();

            println!(
                "Jackpot size after week {} of prize execution: {:?}",
                i, state.prize_buckets[5]
            );
        }

        // Check how much aust was redeemed
        let sent_amount =
            if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
                let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
                if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
                    amount
                } else {
                    panic!("DO NOT ENTER HERE")
                }
            } else {
                panic!("DO NOT ENTER HERE");
            };

        // Update the contract balance
        contract_balance = contract_balance - Uint256::from(sent_amount);

        // Add the funds to the contract address -------------------
        deps.querier.with_token_balances(&[(
            &A_UST.to_string(),
            &[(&MOCK_CONTRACT_ADDR.to_string(), &contract_balance.into())],
        )]);

        // Advance block_time in time
        if let Duration::Time(time) = HOUR {
            env.block.time = env.block.time.plus_seconds(time);
        }

        // Execute prize
        let execute_prize_msg = ExecuteMsg::ExecutePrize { limit: None };
        let _res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg).unwrap();

        amount_distributed_through_lottery += Uint256::from(sent_amount) * exchange_rate;

        let percent_appreciation_towards_lottery =
            Decimal256::from_uint256(Uint256::from(sent_amount)) * exchange_rate
                / Decimal256::from_uint256(pool_size_appreciation);

        assert!(percent_appreciation_towards_lottery <= Decimal256::percent(SPLIT_FACTOR));
    }
}

#[test]
pub fn ceil_helper_function() {
    // Test ceiling
    let res = uint256_times_decimal256_ceil(
        Uint256::from(5u128),
        Decimal256::from_ratio(Uint256::from(1u128), Uint256::from(2u128)),
    );
    assert_eq!(res, Uint256::from(3u128));

    // Test stay
    let res = uint256_times_decimal256_ceil(
        Uint256::from(5u128),
        Decimal256::from_ratio(Uint256::from(1u128), Uint256::from(5u128)),
    );
    assert_eq!(res, Uint256::from(1u128));
}

#[test]
pub fn calculate_max_bound_and_minimum_matches_for_winning_ticket() {
    let ticket = "abcdea";

    // Test with prize distribution with zeros for the two first buckets

    let prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS] = [
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::percent(20),
        Decimal256::percent(20),
        Decimal256::percent(20),
        Decimal256::percent(20),
        Decimal256::percent(20),
    ];

    let minimum_matches_for_winning_ticket =
        get_minimum_matches_for_winning_ticket(prize_distribution).unwrap();

    assert_eq!(minimum_matches_for_winning_ticket, 2);

    let min_bound = &ticket[..minimum_matches_for_winning_ticket];

    assert_eq!(min_bound, "ab");

    let max_bound = calculate_max_bound(min_bound, minimum_matches_for_winning_ticket);

    assert_eq!(max_bound, "abffff");

    // Test with prize distribution with zeros for the first buckets

    let prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS] = [
        Decimal256::zero(),
        Decimal256::percent(1),
        Decimal256::percent(19),
        Decimal256::percent(20),
        Decimal256::percent(20),
        Decimal256::percent(20),
        Decimal256::percent(20),
    ];

    let minimum_matches_for_winning_ticket =
        get_minimum_matches_for_winning_ticket(prize_distribution).unwrap();

    assert_eq!(minimum_matches_for_winning_ticket, 1);

    let min_bound = &ticket[..minimum_matches_for_winning_ticket];

    assert_eq!(min_bound, "a");

    let max_bound = calculate_max_bound(min_bound, minimum_matches_for_winning_ticket);

    assert_eq!(max_bound, "afffff");

    // Test with prize distribution with zeros until the last bucket

    let prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS] = [
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::percent(100),
    ];

    let minimum_matches_for_winning_ticket =
        get_minimum_matches_for_winning_ticket(prize_distribution).unwrap();

    assert_eq!(minimum_matches_for_winning_ticket, 6);

    let min_bound = &ticket[..minimum_matches_for_winning_ticket];

    assert_eq!(min_bound, "abcdea");

    let max_bound = calculate_max_bound(min_bound, minimum_matches_for_winning_ticket);

    assert_eq!(max_bound, "abcdea");

    // Expect an error when prize distribution is all zeros

    let prize_distribution: [Decimal256; NUM_PRIZE_BUCKETS] = [
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::zero(),
        Decimal256::zero(),
    ];

    let minimum_matches_for_winning_ticket =
        get_minimum_matches_for_winning_ticket(prize_distribution);

    let err = Err(StdError::generic_err(
        "The minimum matches for a winning ticket could not be calculated due to a malforming of the prize distribution"
    ));

    assert_eq!(minimum_matches_for_winning_ticket, err);
}

#[test]
pub fn test_ticket_encoding_and_decoding() {
    // Test inverse functionality #1
    let combinations = vec![
        String::from(THREE_MATCH_SEQUENCE),
        String::from(ZERO_MATCH_SEQUENCE),
    ];
    let encoded_tickets = vec_string_tickets_to_encoded_tickets(combinations.clone());
    println!("{}", encoded_tickets);
    let decoded_combinations =
        base64_encoded_tickets_to_vec_string_tickets(encoded_tickets).unwrap();
    println!("{:?}", decoded_combinations);
    assert_eq!(combinations, decoded_combinations);

    // Test inverse functionality #2
    let combinations = vec![String::from("000000")];
    let encoded_tickets = vec_string_tickets_to_encoded_tickets(combinations.clone());
    let decoded_combinations =
        base64_encoded_tickets_to_vec_string_tickets(encoded_tickets).unwrap();
    println!("{:?}", decoded_combinations);
    assert_eq!(combinations, decoded_combinations);

    // Test giving random data
    let encoded_tickets = String::from("aowief");
    let decoded_combinations = base64_encoded_tickets_to_vec_string_tickets(encoded_tickets);
    match decoded_combinations {
        Err(e)
            if e == StdError::generic_err(
                "Couldn't base64 decode the encoded tickets.".to_string(),
            ) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Test giving data with wrong ticket length
    let encoded_tickets = String::from("EjRWeA==");
    let decoded_combinations = base64_encoded_tickets_to_vec_string_tickets(encoded_tickets);
    match decoded_combinations {
        Err(e) if e == StdError::generic_err("Decoded tickets wrong length.") => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

#[test]
pub fn test_query_prizes() {
    // Add some prizes

    let mut deps = mock_dependencies(&[]);

    // get env
    let mut _env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Query them

    for i in 0..10 {
        for j in 0..3 {
            let prize = PrizeInfo {
                claimed: false,
                matches: [i, j, 2, 3, 1, 3, 3],
            };

            PRIZES
                .save(
                    deps.as_mut().storage,
                    (
                        U64Key::from(i as u64),
                        &Addr::unchecked(format!("addr000{}", j)),
                    ),
                    &prize,
                )
                .unwrap();
        }
    }

    let lottery_prizes = read_lottery_prizes(deps.as_ref(), 2, None, None).unwrap();

    let expected_prizes = (0..3)
        .map(|i| {
            (
                Addr::unchecked(format!("addr000{}", i)),
                PrizeInfo {
                    claimed: false,
                    matches: [2, i, 2, 3, 1, 3, 3],
                },
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(lottery_prizes, expected_prizes);

    println!("{:?}", lottery_prizes);

    // Test start after

    let start_after = Some(Addr::unchecked("addr0002"));
    let lottery_prizes = read_lottery_prizes(deps.as_ref(), 2, start_after, None).unwrap();
    assert_eq!(lottery_prizes.len(), 0);

    // Test limit

    let limit = Some(1);
    let lottery_prizes = read_lottery_prizes(deps.as_ref(), 2, None, limit).unwrap();
    assert_eq!(lottery_prizes.len(), 1);
}

#[test]
pub fn test_calculate_boost_multiplier() {
    // Test #1

    let boost_config = BoostConfig {
        base_multiplier: Decimal256::percent(20),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(200),
    };

    let snapshotted_user_lottery_deposit = Uint256::from(100u128);
    let snapshotted_total_user_lottery_deposits = Uint256::from(200u128);

    let snapshotted_user_voting_balance = Uint128::from(20u128);
    let snapshotted_total_voting_balance = Uint128::from(100u128);

    let multiplier = calculate_boost_multiplier(
        boost_config,
        snapshotted_user_lottery_deposit,
        snapshotted_total_user_lottery_deposits,
        snapshotted_user_voting_balance,
        snapshotted_total_voting_balance,
    );

    println!("{}", multiplier);
    assert_eq!(multiplier, Decimal256::percent(36));

    // Test #2

    let boost_config = BoostConfig {
        base_multiplier: Decimal256::percent(20),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(200),
    };

    let snapshotted_user_lottery_deposit = Uint256::from(100u128);
    let snapshotted_total_user_lottery_deposits = Uint256::from(200u128);

    let snapshotted_user_voting_balance = Uint128::from(80u128);
    let snapshotted_total_voting_balance = Uint128::from(100u128);

    let multiplier = calculate_boost_multiplier(
        boost_config,
        snapshotted_user_lottery_deposit,
        snapshotted_total_user_lottery_deposits,
        snapshotted_user_voting_balance,
        snapshotted_total_voting_balance,
    );

    println!("{}", multiplier);
    assert_eq!(multiplier, Decimal256::percent(84));

    // Hit max (exactly)

    let boost_config = BoostConfig {
        base_multiplier: Decimal256::percent(20),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(200),
    };

    let snapshotted_user_lottery_deposit = Uint256::from(100u128);
    let snapshotted_total_user_lottery_deposits = Uint256::from(200u128);

    let snapshotted_user_voting_balance = Uint128::from(100u128);
    let snapshotted_total_voting_balance = Uint128::from(100u128);

    let multiplier = calculate_boost_multiplier(
        boost_config,
        snapshotted_user_lottery_deposit,
        snapshotted_total_user_lottery_deposits,
        snapshotted_user_voting_balance,
        snapshotted_total_voting_balance,
    );

    println!("{}", multiplier);
    assert_eq!(multiplier, Decimal256::percent(100));

    // Hit max (over)

    let boost_config = BoostConfig {
        base_multiplier: Decimal256::percent(20),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(200),
    };

    let snapshotted_user_lottery_deposit = Uint256::from(50u128);
    let snapshotted_total_user_lottery_deposits = Uint256::from(200u128);

    let snapshotted_user_voting_balance = Uint128::from(100u128);
    let snapshotted_total_voting_balance = Uint128::from(100u128);

    let multiplier = calculate_boost_multiplier(
        boost_config,
        snapshotted_user_lottery_deposit,
        snapshotted_total_user_lottery_deposits,
        snapshotted_user_voting_balance,
        snapshotted_total_voting_balance,
    );

    println!("{}", multiplier);
    assert_eq!(multiplier, Decimal256::percent(100));

    // Hit min (over)

    let boost_config = BoostConfig {
        base_multiplier: Decimal256::percent(20),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(200),
    };

    let snapshotted_user_lottery_deposit = Uint256::from(100u128);
    let snapshotted_total_user_lottery_deposits = Uint256::from(200u128);

    let snapshotted_user_voting_balance = Uint128::from(0u128);
    let snapshotted_total_voting_balance = Uint128::from(100u128);

    let multiplier = calculate_boost_multiplier(
        boost_config,
        snapshotted_user_lottery_deposit,
        snapshotted_total_user_lottery_deposits,
        snapshotted_user_voting_balance,
        snapshotted_total_voting_balance,
    );

    println!("{}", multiplier);
    assert_eq!(multiplier, Decimal256::percent(20));
}

#[test]
pub fn test_paused() {
    // Instantiate contracts

    let mut deps = mock_dependencies(&[]);

    // get env
    let mut _env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Pause the contracts

    let info = mock_info(TEST_CREATOR, &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: Some(true),
        lotto_winner_boost_config: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Try to deposit and fail

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );

    // Correct deposit - buys two tickets
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
        operator: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);

    match res {
        Err(ContractError::ContractPaused {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let depositor_info = OldDepositorInfo {
        lottery_deposit: Uint256::zero(),
        savings_aust: Uint256::zero(),
        reward_index: Decimal256::zero(),
        pending_rewards: Decimal256::zero(),
        tickets: vec![],
        unbonding_info: vec![],
    };

    // Add something to old depositors

    bucket::<OldDepositorInfo>(deps.as_mut().storage, b"depositor")
        .save("addr1111".as_bytes(), &depositor_info)
        .unwrap();

    // Try to unpause and fail

    let info = mock_info(TEST_CREATOR, &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: Some(false),
        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);

    match res {
        Err(ContractError::Std(e))
            if e == StdError::generic_err("Cannot unpause contract with old depositors") => {}
        _ => panic!("DO NOT ENTER"),
    };

    // Remove old depositor

    old_remove_depositor_info(
        deps.as_mut().storage,
        &Addr::unchecked("addr1111".to_string()),
    );

    // Try to unpause and succeed

    let info = mock_info(TEST_CREATOR, &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: Some(false),
        lotto_winner_boost_config: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Try to deposit and succeed

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );

    // Correct deposit - buys two tickets
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
        operator: None,
    };

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
}

#[test]
pub fn test_update_depositor_stats() {
    // Instantiate contracts

    let mut deps = mock_dependencies(&[]);

    // get env
    let mut _env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Store depositor stats
    let addr = Addr::unchecked("addr0000");
    let depositor = DepositorStatsInfo {
        shares: Uint256::one(),
        num_tickets: 10,
        operator_addr: Addr::unchecked(""),
    };

    // TODO should return an error instead of ignoring
    store_depositor_stats(deps.as_mut().storage, &addr, depositor, 10).unwrap();

    // Verify that num_tickets is zero

    let depositor_stats = read_depositor_stats(deps.as_ref().storage, &addr);

    assert_eq!(depositor_stats.shares, Uint256::one());
    assert_eq!(depositor_stats.num_tickets, 0);
}

#[test]
pub fn test_historical_depositor_stats() {
    // Instantiate contracts

    let mut deps = mock_dependencies(&[]);

    // get env
    let mut _env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Store depositor stats

    let addr = Addr::unchecked("addr0000");

    // Height 10
    let depositor_10 = DepositorStatsInfo {
        shares: Uint256::one(),
        num_tickets: 0,
        operator_addr: Addr::unchecked(""),
    };

    store_depositor_stats(deps.as_mut().storage, &addr, depositor_10.clone(), 10).unwrap();

    // Height 15
    let depositor_15 = DepositorStatsInfo {
        shares: Uint256::from(2u128),
        num_tickets: 0,
        operator_addr: Addr::unchecked(""),
    };

    store_depositor_stats(deps.as_mut().storage, &addr, depositor_15.clone(), 15).unwrap();

    // Height 20

    let depositor_20 = DepositorStatsInfo {
        shares: Uint256::from(3u128),
        num_tickets: 0,
        operator_addr: Addr::unchecked(""),
    };

    store_depositor_stats(deps.as_mut().storage, &addr, depositor_20.clone(), 20).unwrap();

    // Verify depositors

    let depositor_stats_0 = read_depositor_stats_at_height(deps.as_ref().storage, &addr, 0);

    assert_eq!(
        depositor_stats_0,
        DepositorStatsInfo {
            shares: Uint256::zero(),
            num_tickets: 0,
            operator_addr: Addr::unchecked("")
        }
    );

    let depositor_stats_10 = read_depositor_stats_at_height(deps.as_ref().storage, &addr, 11);
    assert_eq!(depositor_stats_10, depositor_10);

    let depositor_stats_15 = read_depositor_stats_at_height(deps.as_ref().storage, &addr, 16);
    assert_eq!(depositor_stats_15, depositor_15);

    let depositor_stats_20 = read_depositor_stats_at_height(deps.as_ref().storage, &addr, 21);
    assert_eq!(depositor_stats_20, depositor_20);
}

#[test]
pub fn test_migrate() {
    // Instantiate contracts
    let mut deps = mock_dependencies(&[]);

    // get env
    let mut _env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    // Increase current lottery
    let state = STATE.load(deps.as_mut().storage).unwrap();
    let old_state = OldState {
        total_tickets: state.total_tickets,
        total_reserve: state.total_reserve,
        prize_buckets: state.prize_buckets,
        current_lottery: 2,
        next_lottery_time: state.next_lottery_time,
        next_lottery_exec_time: state.next_lottery_exec_time,
        next_epoch: state.next_epoch,
        global_reward_index: state.operator_reward_emission_index.global_reward_index,
        glow_emission_rate: state.operator_reward_emission_index.glow_emission_rate,
        last_reward_updated: state.operator_reward_emission_index.last_reward_updated,
    };
    OLDSTATE.save(deps.as_mut().storage, &old_state).unwrap();

    // Store old config

    let config = CONFIG.load(deps.as_ref().storage).unwrap();

    let old_config = OldConfig {
        owner: config.owner,
        a_terra_contract: config.a_terra_contract,
        gov_contract: config.gov_contract,
        distributor_contract: config.distributor_contract,
        anchor_contract: config.anchor_contract,
        oracle_contract: config.oracle_contract,
        stable_denom: config.stable_denom,
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
    };

    OLDCONFIG.save(deps.as_mut().storage, &old_config).unwrap();

    // Store old pool

    let old_pool = OldPool {
        total_user_lottery_deposits: Uint256::from(1u128),
        total_user_savings_aust: Uint256::from(1u128),
        total_sponsor_lottery_deposits: Uint256::from(1u128),
    };

    OLDPOOL.save(deps.as_mut().storage, &old_pool).unwrap();

    // Store some old lotteries

    for i in 0..old_state.current_lottery {
        let mut old_lottery = old_read_lottery_info(deps.as_ref().storage, 100);

        old_lottery.sequence = format!("00000{}", i);

        old_store_lottery_info(deps.as_mut().storage, i, &old_lottery).unwrap();
    }

    // Store some old prizes

    for i in 0..3 {
        for j in 0..3 {
            let prize_info = PrizeInfo {
                claimed: false,
                matches: [i; 7],
            };

            OLD_PRIZES
                .save(
                    deps.as_mut().storage,
                    (
                        &Addr::unchecked(format!("addr000{}", i)),
                        U64Key::from(j as u64),
                    ),
                    &prize_info,
                )
                .unwrap();
        }
    }

    // Store some old depositors

    for i in 0..15 {
        let mut old_depositor_info =
            old_read_depositor_info(deps.as_ref().storage, &Addr::unchecked(""));

        old_depositor_info.savings_aust = Uint256::from(i as u128);

        old_store_depositor_info(
            deps.as_mut().storage,
            &Addr::unchecked(format!("addr000{}", i)),
            &old_depositor_info,
        )
        .unwrap();
    }

    // Now migrate

    let migrate_msg = MigrateMsg {
        glow_prize_buckets: [Uint256::zero(); 7],
        max_tickets_per_depositor: 10_000,
        community_contract: COMMUNITY_ADDR.to_string(),
        lotto_winner_boost_config: None,
        ve_contract: VE_ADDR.to_string(),
    };

    let _res = migrate(deps.as_mut(), mock_env(), migrate_msg.clone()).unwrap();

    // Now try to unpause and fail

    let info = mock_info(TEST_CREATOR, &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        epoch_interval: None,
        max_holders: None,
        max_tickets_per_depositor: None,
        paused: Some(false),
        lotto_winner_boost_config: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);

    match res {
        Err(ContractError::Std(e))
            if e == StdError::generic_err("Cannot unpause contract with old depositors") => {}
        _ => panic!("DO NOT ENTER"),
    };

    // Migration loop

    let info = mock_info(TEST_CREATOR, &[]);
    let msg = ExecuteMsg::MigrateOldDepositors { limit: Some(10) };
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "migrate_old_depositors"),
            attr("num_migrated_entries", "10"),
        ]
    );

    let info = mock_info(TEST_CREATOR, &[]);
    let msg = ExecuteMsg::MigrateOldDepositors { limit: Some(10) };
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "migrate_old_depositors"),
            attr("num_migrated_entries", "5"),
        ]
    );

    // Now verify that the config is unpaused

    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert!(!config_response.paused);

    // Verify the new states are as expected

    // New Config

    let default_lotto_winner_boost_config: BoostConfig = BoostConfig {
        base_multiplier: Decimal256::from_ratio(Uint256::from(40u128), Uint256::from(100u128)),
        max_multiplier: Decimal256::one(),
        total_voting_power_weight: Decimal256::percent(150),
    };

    let new_config = Config {
        owner: old_config.owner,
        a_terra_contract: old_config.a_terra_contract,
        gov_contract: old_config.gov_contract,
        ve_contract: deps
            .api
            .addr_validate(migrate_msg.ve_contract.as_str())
            .unwrap(),
        community_contract: deps
            .api
            .addr_validate(migrate_msg.community_contract.as_str())
            .unwrap(),
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
        max_tickets_per_depositor: migrate_msg.max_tickets_per_depositor,
        glow_prize_buckets: migrate_msg.glow_prize_buckets,
        paused: false,
        lotto_winner_boost_config: default_lotto_winner_boost_config,
    };

    assert_eq!(new_config, CONFIG.load(deps.as_ref().storage).unwrap());

    // New Lottery Info

    for i in 0..state.current_lottery {
        let mut old_lottery = old_read_lottery_info(deps.as_ref().storage, 100);

        old_lottery.sequence = format!("00000{}", i);

        let lottery = read_lottery_info(deps.as_ref().storage, i);

        assert_eq!(
            lottery,
            LotteryInfo {
                rand_round: old_lottery.rand_round,
                sequence: old_lottery.sequence,
                awarded: old_lottery.awarded,
                timestamp: Timestamp::from_seconds(0),
                block_height: old_lottery.timestamp,
                prize_buckets: old_lottery.prize_buckets,
                number_winners: old_lottery.number_winners,
                page: old_lottery.page,
                glow_prize_buckets: [Uint256::zero(); 7],
                total_user_shares: Uint256::zero(),
            }
        );
    }

    // New prizes

    for i in 0..3 {
        for j in 0..3 {
            let prize_info = PrizeInfo {
                claimed: false,
                matches: [i; 7],
            };

            println!(
                "Reading from {:?}",
                (
                    U64Key::from(j as u64),
                    &Addr::unchecked(format!("addr000{}", i)).to_string()
                )
            );
            assert_eq!(
                prize_info,
                PRIZES
                    .load(
                        deps.as_ref().storage,
                        (
                            U64Key::from(j as u64),
                            &Addr::unchecked(format!("addr000{}", i))
                        )
                    )
                    .unwrap()
            );
        }
    }

    // New depositors

    let mut new_user_total_aust = Uint256::zero();

    for i in 0..15 {
        let mut old_depositor_info =
            old_read_depositor_info(deps.as_ref().storage, &Addr::unchecked(""));

        old_depositor_info.savings_aust = Uint256::from(i as u128);

        let old_depositor_aust_balance = old_depositor_info.savings_aust
            + old_depositor_info.lottery_deposit / Decimal256::permille(RATE);

        new_user_total_aust += old_depositor_aust_balance;

        let depositor_info = read_depositor_info(
            deps.as_ref().storage,
            &Addr::unchecked(format!("addr000{}", i)),
        );

        assert_eq!(
            depositor_info,
            DepositorInfo {
                shares: old_depositor_aust_balance,
                tickets: old_depositor_info.tickets,
                unbonding_info: old_depositor_info.unbonding_info,
                operator_addr: Addr::unchecked("")
            }
        );
    }

    // New State

    let new_state = State {
        total_tickets: old_state.total_tickets,
        total_reserve: old_state.total_reserve,
        prize_buckets: old_state.prize_buckets,
        current_lottery: old_state.current_lottery,
        next_lottery_time: old_state.next_lottery_time,
        next_lottery_exec_time: old_state.next_lottery_exec_time,
        next_epoch: old_state.next_epoch,
        operator_reward_emission_index: RewardEmissionsIndex {
            global_reward_index: old_state.global_reward_index,
            glow_emission_rate: old_state.glow_emission_rate,
            last_reward_updated: old_state.last_reward_updated,
        },
        sponsor_reward_emission_index: RewardEmissionsIndex {
            global_reward_index: old_state.global_reward_index,
            glow_emission_rate: old_state.glow_emission_rate,
            last_reward_updated: old_state.last_reward_updated,
        },
        last_lottery_execution_aust_exchange_rate: Decimal256::permille(RATE),
    };

    assert_eq!(new_state, STATE.load(deps.as_ref().storage).unwrap());

    // New Pool

    let new_pool = Pool {
        total_user_aust: new_user_total_aust,
        total_user_shares: new_user_total_aust,
        total_sponsor_lottery_deposits: old_pool.total_sponsor_lottery_deposits,
        total_operator_shares: Uint256::zero(),
    };

    assert_eq!(new_pool, POOL.load(deps.as_ref().storage).unwrap());
}

#[test]
pub fn anchor_pool_smaller_than_total_deposits() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    let special_rate = Decimal256::from_str(".05234").unwrap();

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(special_rate);

    // get env
    let env = mock_env();

    // mock instantiate the contracts
    mock_instantiate_small_ticket_price(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(SMALL_TICKET_PRICE).into(),
        }],
    );

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: vec_string_tickets_to_encoded_tickets(vec![String::from("234567")]),
        operator: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Add the funds to the contract address -------------------

    let minted_aust = Uint256::from(SMALL_TICKET_PRICE) / special_rate;
    // Get the number of minted shares
    let minted_shares = minted_aust;

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &minted_aust.into())],
    )]);

    // Compare shares_supply with contract_a_balance -----------

    let pool = query_pool(deps.as_ref()).unwrap();
    let contract_a_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    println!("hi: {}, {}", minted_aust, pool.total_user_aust);

    // user_aust should equal contract_a_balance
    assert_eq!(pool.total_user_aust, contract_a_balance);

    // Check that the depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0001").unwrap()
        ),
        DepositorInfo {
            shares: minted_shares,
            tickets: vec![String::from("234567")],
            unbonding_info: vec![],
            operator_addr: Addr::unchecked("")
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(1u64),
            total_reserve: Uint256::zero(),
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: HOUR.mul(3).after(&mock_env().block),
            operator_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            sponsor_reward_emission_index: RewardEmissionsIndex {
                last_reward_updated: 12345,
                global_reward_index: Decimal256::zero(),
                glow_emission_rate: Decimal256::zero(),
            },
            last_lottery_execution_aust_exchange_rate: special_rate
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_aust: minted_aust,
            total_user_shares: minted_shares,
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_operator_shares: Uint256::zero(),
        }
    );

    // Address withdraws a quarter of their money ----------------

    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: Some((SMALL_TICKET_PRICE / 4).into()),
        instant: None,
    };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Message for redeem amount operation of aUST

    // Get the sent_amount
    let sent_amount = if let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &res.messages[0].msg {
        let send_msg: Cw20ExecuteMsg = from_binary(msg).unwrap();
        if let Cw20ExecuteMsg::Send { amount, .. } = send_msg {
            amount
        } else {
            panic!("DO NOT ENTER HERE")
        }
    } else {
        panic!("DO NOT ENTER HERE");
    };

    // Update contract_balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &(contract_a_balance - sent_amount.into()).into(),
        )],
    )]);

    // Verify that Anchor Pool is solvent

    // // Compare shares_supply with contract_a_balance
    // let pool = query_pool(deps.as_ref()).unwrap();

    // let contract_a_balance = query_token_balance(
    //     deps.as_ref(),
    //     Addr::unchecked(A_UST),
    //     Addr::unchecked(MOCK_CONTRACT_ADDR),
    // )
    // .unwrap();

    // TODO Think about
    // // Assert that Lotto pool is solvent
    // assert!(
    //     contract_a_balance * special_rate
    //         >= pool.total_user_lottery_deposits + pool.total_sponsor_lottery_deposits
    // )
}

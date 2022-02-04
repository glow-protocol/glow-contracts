use crate::contract::{
    execute, instantiate, query, query_config, query_pool, query_state, query_ticket_info,
    INITIAL_DEPOSIT_AMOUNT,
};
use crate::helpers::{
    calculate_max_bound, calculate_winner_prize, encoded_tickets_to_combinations,
    get_minimum_matches_for_winning_ticket, uint256_times_decimal256_ceil,
};
use crate::mock_querier::{
    mock_dependencies, mock_env, mock_info, WasmMockQuerier, MOCK_CONTRACT_ADDR,
};
use crate::state::{
    query_prizes, read_depositor_info, read_lottery_info, read_sponsor_info, store_depositor_info,
    DepositorInfo, LotteryInfo, PrizeInfo, CONFIG, STATE,
};
use crate::test_helpers::{
    calculate_lottery_prize_buckets, calculate_prize_buckets,
    calculate_remaining_state_prize_buckets, combinations_to_encoded_tickets,
    generate_sequential_ticket_combinations,
};
use glow_protocol::lotto::{NUM_PRIZE_BUCKETS, TICKET_LENGTH};
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
    Claim, ConfigResponse, DepositorInfoResponse, ExecuteMsg, InstantiateMsg, PoolResponse,
    QueryMsg, SponsorInfoResponse, StateResponse,
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
pub const DISTRIBUTOR_ADDR: &str = "distributor";
pub const ORACLE_ADDR: &str = "oracle";

pub const RATE: u64 = 1023; // as a permille
const SMALL_TICKET_PRICE: u64 = 10;
const TICKET_PRICE: u64 = 10_000_000; // 10 * 10^6

const SPLIT_FACTOR: u64 = 75; // as a %
const INSTANT_WITHDRAWAL_FEE: u64 = 10; // as a %
const RESERVE_FACTOR: u64 = 5; // as a %
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
        Uint256::from(0u128),
        Uint256::from(0u128),
        Uint256::from(0u128),
        Uint256::from(0u128),
        Uint256::from(0u128),
        // Uint256::from(10 * u128::pow(10, 6)),
        // Uint256::from(10 * u128::pow(10, 6)),
        // Uint256::from(10 * u128::pow(10, 6)),
        // Uint256::from(10 * u128::pow(10, 6)),
        // Uint256::from(10 * u128::pow(10, 6)),
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
        initial_emission_rate: Decimal256::zero(),
        initial_lottery_execution: FIRST_LOTTO_TIME,
        max_tickets_per_depositor: MAX_TICKETS_PER_DEPOSITOR,
        glow_prize_buckets: *GLOW_PRIZE_BUCKETS,
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
        initial_emission_rate: Decimal256::zero(),
        initial_lottery_execution: FIRST_LOTTO_TIME,
        max_tickets_per_depositor: MAX_TICKETS_PER_DEPOSITOR,
        glow_prize_buckets: *GLOW_PRIZE_BUCKETS,
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
        distributor_contract: DISTRIBUTOR_ADDR.to_string(),
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
            max_tickets_per_depositor: MAX_TICKETS_PER_DEPOSITOR
        }
    );

    // Check that the glow_emission_rate and last_block_updated are set correctly
    let state = STATE.load(deps.as_ref().storage).unwrap();
    assert_eq!(state.glow_emission_rate, Decimal256::zero());
    assert_eq!(state.last_reward_updated, mock_env().block.height);

    // Register contracts
    let msg = ExecuteMsg::RegisterContracts {
        gov_contract: GOV_ADDR.to_string(),
        distributor_contract: DISTRIBUTOR_ADDR.to_string(),
    };

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();
    let config = query_config(deps.as_ref()).unwrap();
    assert_eq!(config.gov_contract, GOV_ADDR.to_string());
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
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    let pool = query_pool(deps.as_ref()).unwrap();
    assert_eq!(
        pool,
        PoolResponse {
            total_user_lottery_deposits: Uint256::zero(),
            total_user_savings_aust: Uint256::zero(),
            total_sponsor_lottery_deposits: Uint256::zero(),
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
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check max_tickets_per_depositor has changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.max_tickets_per_depositor, 100);

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
        encoded_tickets: combinations_to_encoded_tickets(too_many_combinations),
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
        encoded_tickets: combinations_to_encoded_tickets(too_many_combinations),
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
        encoded_tickets: combinations_to_encoded_tickets(too_many_combinations),
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
        encoded_tickets: combinations_to_encoded_tickets(too_many_combinations),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(THREE_MATCH_SEQUENCE),
            String::from(ZERO_MATCH_SEQUENCE),
        ]),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards savings
    let minted_savings_aust = minted_aust - minted_lottery_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

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
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![
                String::from(ZERO_MATCH_SEQUENCE),
                String::from(ONE_MATCH_SEQUENCE)
            ],
            unbonding_info: vec![]
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
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_lottery_deposits: minted_lottery_aust_value,
            total_user_savings_aust: minted_savings_aust,
            total_sponsor_lottery_deposits: Uint256::zero(),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(TWO_MATCH_SEQUENCE)]),
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 3);

    // deposit again
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(THREE_MATCH_SEQUENCE)]),
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 5);

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ZERO_MATCH_SEQUENCE_2)]),
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 6);

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ZERO_MATCH_SEQUENCE_3)]),
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 8);

    // Test sequential buys of the same ticket by the same address
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(FOUR_MATCH_SEQUENCE)]),
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(FOUR_MATCH_SEQUENCE)]),
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
            encoded_tickets: combinations_to_encoded_tickets(vec![String::from(
                ZERO_MATCH_SEQUENCE_4,
            )]),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ZERO_MATCH_SEQUENCE_4)]),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
        recipient: "addr1111".to_string(),
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

    //TODO: Revise this. Clippy complains as variables not being used
    let expected_tickets_attempted = 2;
    match res {
        Err(ContractError::InsufficientGiftDepositAmount(amount_required)) => {
            assert_eq!(expected_tickets_attempted, amount_required)
        }
        _ => panic!("DO NOT ENTER HERE"),
    }
    // Invalid recipient - you cannot make a gift to yourself
    let msg = ExecuteMsg::Gift {
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE_3),
            String::from(ZERO_MATCH_SEQUENCE_4),
        ]),
        recipient: "addr0000".to_string(),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
        recipient: "addr1111".to_string(),
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Get the number of minted aust
    let minted_aust = Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards savings
    let minted_savings_aust = minted_aust - minted_lottery_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

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
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![
                String::from(ZERO_MATCH_SEQUENCE),
                String::from(ONE_MATCH_SEQUENCE)
            ],
            unbonding_info: vec![]
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
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_lottery_deposits: minted_lottery_aust_value,
            total_user_savings_aust: minted_savings_aust,
            total_sponsor_lottery_deposits: Uint256::zero(),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ONE_MATCH_SEQUENCE)]),
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
        query_ticket_info(deps.as_ref(), String::from("23456"))
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
            lottery_deposit: Uint256::zero(),
            savings_aust: Uint256::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![Claim {
                amount: Uint256::from(sent_amount) * Decimal256::permille(RATE),
                release_at: WEEK.after(&mock_env().block),
            }]
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
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_lottery_deposits: Uint256::zero(),
            total_user_savings_aust: Uint256::zero(),
            total_sponsor_lottery_deposits: Uint256::zero(),
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

    deps.querier.with_tax(
        Decimal::percent(1),
        &[(&"uusd".to_string(), &Uint128::from(1000000u128))],
    );

    // Withdraw with a given amount
    for index in 0..10 {
        let msg = ExecuteMsg::Deposit {
            encoded_tickets: combinations_to_encoded_tickets(vec![format!(
                "{:0length$}",
                index,
                length = TICKET_LENGTH
            )]),
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
        amount: Some(Uint256::from(5 * TICKET_PRICE).into()),
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

    // Correct withdraw, user has 5 tickets to be withdrawn
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr2222").unwrap()
        )
        .tickets,
        vec![
            // TODO: Don't hardcode the number of tickets
            // String::from("000005"),
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
        Uint256::from(4u64)
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
            // TODO Don't hardcode
            // String::from("000006"),
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
        Uint256::from(3u64)
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ZERO_MATCH_SEQUENCE)]),
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

    // Get the number of minted aust that will go towards savings
    let _minted_savings_aust = minted_aust - minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the value of minted aust going towards the lottery
    let _minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

    // Get the amount to be withdrawn
    let depositor =
        read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap());

    // Get the amount of aust equivalent to the depositor's lottery deposit
    let depositor_lottery_aust = depositor.lottery_deposit / Decimal256::permille(RATE);

    // Calculate the depositor's aust balance
    let depositor_aust_balance = depositor.savings_aust + depositor_lottery_aust;

    // Calculate the depositor's balance from their aust balance
    let _depositor_balance = depositor_aust_balance * Decimal256::permille(RATE);

    let aust_to_redeem = depositor_aust_balance;
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
        query_ticket_info(deps.as_ref(), String::from("23456"))
            .unwrap()
            .holders,
        empty_addr
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap()),
        DepositorInfo {
            lottery_deposit: Uint256::zero(),
            savings_aust: Uint256::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![]
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
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_lottery_deposits: Uint256::zero(),
            total_user_savings_aust: Uint256::zero(),
            total_sponsor_lottery_deposits: Uint256::zero(),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ZERO_MATCH_SEQUENCE)]),
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

    // Get the number of minted aust that will go towards savings
    let _minted_savings_aust = minted_aust - minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the value of minted aust going towards the lottery
    let _minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

    // Get the amount to be withdrawn
    let depositor =
        read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap());

    // Get the amount of aust equivalent to the depositor's lottery deposit
    let depositor_lottery_aust = depositor.lottery_deposit / Decimal256::permille(RATE);

    // Calculate the depositor's aust balance
    let depositor_aust_balance = depositor.savings_aust + depositor_lottery_aust;

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
            lottery_deposit: Uint256::zero(),
            savings_aust: Uint256::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![]
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(SIX_MATCH_SEQUENCE)]),
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

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards savings
    let minted_savings_aust = minted_aust - minted_lottery_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(SIX_MATCH_SEQUENCE)],
            unbonding_info: vec![]
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
    let exec_height = env.block.height;

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
    let lottery_prize_buckets =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners);
    let glow_prize_buckets = calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners);

    let lottery = read_lottery_info(deps.as_ref().storage, 0u64);
    assert_eq!(
        lottery,
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            block_height: env.block.height,
            total_user_lottery_deposits: minted_lottery_aust_value
        }
    );

    let prize_info = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(
        prize_info,
        PrizeInfo {
            claimed: false,
            matches: number_winners,
            lottery_deposit: minted_lottery_aust_value
        }
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Uint256::zero(),);

    let remaining_state_prize_buckets =
        calculate_remaining_state_prize_buckets(state_prize_buckets, number_winners);

    // From the initialization of the contract
    assert_eq!(state.prize_buckets, remaining_state_prize_buckets);

    let info = mock_info("addr0000", &[]);
    let msg = ExecuteMsg::ClaimLottery {
        lottery_ids: Vec::from([0u64]),
    };

    // Claim lottery should work, even if there are no unbonded claims
    let res = execute(deps.as_mut(), env, info.clone(), msg).unwrap();

    let config = CONFIG.load(deps.as_ref().storage).unwrap();
    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);
    let winner_address = info.sender;
    let (mut ust_to_send, glow_to_send): (Uint128, Uint128) = calculate_winner_prize(
        &deps.as_mut().querier,
        &config,
        &prize_info,
        &lottery_info,
        &winner_address,
    )
    .unwrap();

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(
        prizes,
        PrizeInfo {
            claimed: true,
            matches: [0, 0, 0, 0, 0, 0, 1],
            lottery_deposit: minted_lottery_aust_value
        }
    );

    //deduct reserve fee
    let config = query_config(deps.as_ref()).unwrap();
    let reserve_fee = Uint256::from(ust_to_send) * config.reserve_factor;
    ust_to_send -= Uint128::from(reserve_fee);

    //check total_reserve
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.total_reserve, reserve_fee);

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
            to_address: "addr0000".to_string(),
            amount: vec![Coin {
                denom: String::from("uusd"),
                amount: ust_to_send,
            }],
        }))]
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
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
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
    let aust_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    let pool = query_pool(deps.as_ref()).unwrap();

    // Lottery balance equals aust_balance - total_user_savings_aust
    let aust_lottery_balance = aust_balance - pool.total_user_savings_aust;

    // Get the pooled lottery_deposit
    let pooled_lottery_deposits = aust_lottery_balance * new_rate;

    // Get the amount to redeem
    let amount_to_redeem = pooled_lottery_deposits
        - pool.total_user_lottery_deposits
        - pool.total_sponsor_lottery_deposits;

    // Divide by the rate to get the number of aust to redeem
    let aust_to_redeem: Uint128 = (amount_to_redeem / new_rate).into();

    // Verify amount to redeem for the lottery
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: A_UST.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: ANCHOR.to_string(),
                amount: aust_to_redeem,
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

    // Execute 3rd lottery
    let lottery_msg = ExecuteMsg::ExecuteLottery {};
    let info = mock_info("addr0001", &[]);
    let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

    // Amount of aust to redeem

    // Get this contracts aust balance
    let aust_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    let pool = query_pool(deps.as_ref()).unwrap();

    // Lottery balance equals aust_balance - total_user_savings_aust
    let aust_lottery_balance = aust_balance - pool.total_user_savings_aust;

    // Get the pooled lottery_deposit
    let pooled_lottery_deposits = aust_lottery_balance * new_rate;

    // Get the amount to redeem
    let amount_to_redeem = pooled_lottery_deposits
        - pool.total_user_lottery_deposits
        - pool.total_sponsor_lottery_deposits;

    // Divide by the rate to get the number of aust to redeem
    let aust_to_redeem: Uint128 = (amount_to_redeem / new_rate).into();

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
                amount: aust_to_redeem,
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ZERO_MATCH_SEQUENCE)]),
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

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards savings
    let minted_savings_aust = minted_aust - minted_lottery_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(ZERO_MATCH_SEQUENCE)],
            unbonding_info: vec![]
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
    let exec_height = env.block.height;
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Check that state equals calculated prize
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, state_prize_buckets);

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Check lottery info was updated correctly
    let awarded_prize = Uint256::zero();
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            number_winners: [0; NUM_PRIZE_BUCKETS],
            page: "".to_string(),
            glow_prize_buckets: [Uint256::zero(); NUM_PRIZE_BUCKETS],
            block_height: env.block.height,
            total_user_lottery_deposits: minted_lottery_aust_value
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(SIX_MATCH_SEQUENCE)]),
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

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards savings
    let minted_savings_aust = minted_aust - minted_lottery_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(SIX_MATCH_SEQUENCE)],
            unbonding_info: vec![]
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
    let exec_height = env.block.height;

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }
    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let number_winners = [0, 0, 0, 0, 0, 0, 1];
    let lottery_prize_buckets =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners);

    let glow_prize_buckets = calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            block_height: env.block.height,
            total_user_lottery_deposits: minted_lottery_aust_value
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(prizes.matches, number_winners);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Uint256::zero(),);

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
                state_prize_buckets[NUM_PRIZE_BUCKETS - 1].to_string()
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(SIX_MATCH_SEQUENCE)]),
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

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards savings
    let minted_savings_aust = minted_aust - minted_lottery_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_0),
        DepositorInfo {
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(SIX_MATCH_SEQUENCE)],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 2 hits
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(TWO_MATCH_SEQUENCE)]),
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
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(TWO_MATCH_SEQUENCE)],
            unbonding_info: vec![]
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
    let exec_height = env.block.height;

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let number_winners = [0, 0, 1, 0, 0, 0, 1];
    let lottery_prize_buckets =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners);
    let glow_prize_buckets = calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners);

    // calculate the value of each deposit accounting for rounding errors
    let each_lottery_deposit_amount = (Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE)
        * Decimal256::percent(SPLIT_FACTOR))
        * Decimal256::permille(RATE);

    // calculate the total minted_aust_value
    let total_lottery_deposit_amount = Uint256::from(2u128) * each_lottery_deposit_amount;

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            block_height: env.block.height,
            total_user_lottery_deposits: total_lottery_deposit_amount
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw_0, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 0, 0, 0, 0, 1]);

    let prizes = query_prizes(deps.as_ref(), &address_raw_1, 0u64).unwrap();
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(FOUR_MATCH_SEQUENCE)]),
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

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards savings
    let minted_savings_aust = minted_aust - minted_lottery_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_0),
        DepositorInfo {
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(FOUR_MATCH_SEQUENCE)],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 4 hits
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(FOUR_MATCH_SEQUENCE)]),
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
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(FOUR_MATCH_SEQUENCE)],
            unbonding_info: vec![]
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
    let exec_height = env.block.height;

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Check that state equals calculated prize
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, state_prize_buckets);

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let number_winners = [0, 0, 0, 0, 2, 0, 0];
    let lottery_prize_buckets =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners);

    let glow_prize_buckets = calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners);

    // calculate the value of each deposit accounting for rounding errors
    let each_lottery_deposit_amount = (Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE)
        * Decimal256::percent(SPLIT_FACTOR))
        * Decimal256::permille(RATE);

    // calculate the total minted_aust_value
    let total_lottery_deposit_amount = Uint256::from(2u128) * each_lottery_deposit_amount;

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            block_height: env.block.height,
            total_user_lottery_deposits: total_lottery_deposit_amount
        }
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Uint256::zero(),);

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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(SIX_MATCH_SEQUENCE)]),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ONE_MATCH_SEQUENCE)]),
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(FOUR_MATCH_SEQUENCE)]),
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(FOUR_MATCH_SEQUENCE_2)]),
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(FOUR_MATCH_SEQUENCE_3)]),
    };
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw = deps.api.addr_validate("addr0000").unwrap();

    // calculate the value of each deposit accounting for rounding errors
    let each_lottery_deposit_amount = (Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE)
        * Decimal256::percent(SPLIT_FACTOR))
        * Decimal256::permille(RATE);

    let each_savings_aust_amount = (Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE))
        - Decimal256::percent(SPLIT_FACTOR)
            * (Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE));

    // calculate the total minted_aust_value
    let total_lottery_deposit_amount = Uint256::from(5u128) * each_lottery_deposit_amount;

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            lottery_deposit: total_lottery_deposit_amount,
            savings_aust: Uint256::from(5u128) * (each_savings_aust_amount),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![
                String::from(SIX_MATCH_SEQUENCE),
                String::from(ONE_MATCH_SEQUENCE),
                String::from(FOUR_MATCH_SEQUENCE),
                String::from(FOUR_MATCH_SEQUENCE_2),
                String::from(FOUR_MATCH_SEQUENCE_3),
            ],
            unbonding_info: vec![]
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
    let exec_height = env.block.height;
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Check that state equals calculated prize
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, state_prize_buckets);

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let number_winners = [0, 0, 0, 0, 3, 0, 1];
    let lottery_prize_buckets =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners);

    let glow_prize_buckets = calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners);

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
            timestamp: exec_height,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            block_height: env.block.height,
            total_user_lottery_deposits: total_lottery_deposit_amount
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(prizes.matches, number_winners);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Uint256::zero());

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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(SIX_MATCH_SEQUENCE)]),
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

    // calculate the value of each deposit accounting for rounding errors
    let each_lottery_deposit_amount = (Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE)
        * Decimal256::percent(SPLIT_FACTOR))
        * Decimal256::permille(RATE);

    // calculate the total minted_aust_value
    let total_lottery_deposit_amount = Uint256::from(3u128) * each_lottery_deposit_amount;

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
    let exec_height = env.block.height;
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Check that state equals calculated prize
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.prize_buckets, state_prize_buckets);

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let number_winners = [0, 0, 0, 0, 0, 0, 3];
    let lottery_prize_buckets =
        calculate_lottery_prize_buckets(state_prize_buckets, number_winners);
    let glow_prize_buckets = calculate_lottery_prize_buckets(*GLOW_PRIZE_BUCKETS, number_winners);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: SIX_MATCH_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            prize_buckets: lottery_prize_buckets,
            number_winners,
            page: "".to_string(),
            glow_prize_buckets,
            block_height: env.block.height,
            total_user_lottery_deposits: total_lottery_deposit_amount
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_0, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 0, 0, 0, 0, 1]);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Uint256::zero());

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
            encoded_tickets: combinations_to_encoded_tickets(vec![format!(
                "be{:0length$}",
                100 + index,
                length = TICKET_LENGTH - 2
            )]),
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
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
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

    // Get the number of minted aust that will go towards savings
    let _minted_savings_aust = minted_aust - minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

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
    assert_eq!(Decimal256::zero(), state.global_reward_index);

    // Register contracts
    mock_register_contracts(deps.as_mut());

    // Execute epoch ops

    let msg = ExecuteMsg::ExecuteEpochOps {};
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();
    assert_eq!(state.last_reward_updated, env.block.height);

    let state = query_state(deps.as_ref(), env.clone(), Some(env.block.height)).unwrap();
    assert_eq!(Decimal256::zero(), state.global_reward_index);

    // Increase glow emission rate
    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.glow_emission_rate = Decimal256::one();
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

    let res: DepositorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            env,
            QueryMsg::Depositor {
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
fn claim_rewards_one_depositor() {
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
    state.glow_emission_rate = Decimal256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    // User has no deposits, so no claimable rewards and empty msg returned
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(res.messages.len(), 0);

    // Deposit of 20_000_000 uusd
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
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

    // Get the number of minted aust that will go towards savings
    let _minted_savings_aust = minted_aust - minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

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

    let res: DepositorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Depositor {
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
fn claim_rewards_multiple_depositors() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.glow_emission_rate = Decimal256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    //TODO: should query glow emission rate instead of hard-code
    /*
    STATE.update(deps.as_mut().storage,  |mut state| {
        state.glow_emission_rate = Decimal256::one();
        Ok(state)
    }).unwrap();
     */

    // USER 0 Deposits 20_000_000 uusd
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
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

    // USER 1 Deposits another 20_000_000 uusd
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(SIX_MATCH_SEQUENCE),
            String::from(TWO_MATCH_SEQUENCE),
        ]),
    };
    let info = mock_info(
        "addr1111",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );
    let _res = execute(deps.as_mut(), env.clone(), info, msg);

    let info = mock_info("addr0000", &[]);

    // calculate the value of each deposit accounting for rounding errors
    let each_lottery_deposit_amount = (Uint256::from(2 * TICKET_PRICE)
        / Decimal256::permille(RATE)
        * Decimal256::percent(SPLIT_FACTOR))
        * Decimal256::permille(RATE);

    // calculate the total minted_aust_value
    let total_lottery_deposit_amount = Uint256::from(2u128) * each_lottery_deposit_amount;

    // After 100 blocks
    env.block.height += 100;

    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();
    println!("Global reward index: {:?}", state.global_reward_index);
    println!("Emission rate {:?}", state.glow_emission_rate);
    println!("Last reward updated {:?}", state.last_reward_updated);
    println!("Current height {:?}", env.block.height);

    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    println!("{:?}", res.attributes);
    println!("Total deposits test: {}", total_lottery_deposit_amount);
    println!(
        "{}",
        (Decimal256::from_str("100").unwrap()
            / Decimal256::from_uint256(total_lottery_deposit_amount)
            * each_lottery_deposit_amount)
    );
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "addr0000".to_string(),
                amount: (Decimal256::from_str("100").unwrap()
                    / Decimal256::from_uint256(total_lottery_deposit_amount)
                    * each_lottery_deposit_amount)
                    .into(),
            })
            .unwrap(),
        }))]
    );

    // Checking USER 0 state is correct
    let res: DepositorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Depositor {
                address: "addr0000".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(res.pending_rewards, Decimal256::zero());

    assert_eq!(res.reward_index, state.global_reward_index);
    assert_eq!(
        res.reward_index,
        Decimal256::from_uint256(Uint256::from(100u128))
            / Decimal256::from_uint256(total_lottery_deposit_amount)
    );

    // Checking USER 1 state is correct
    let res: DepositorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Depositor {
                address: "addr1111".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        res.pending_rewards,
        Decimal256::from_uint256(each_lottery_deposit_amount) * state.global_reward_index
    );
    assert_eq!(res.reward_index, state.global_reward_index);

    //TODO: Add a subsequent deposit at a later env.block.height and test again
}

#[test]
fn claim_rewards_depositor_and_sponsor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    mock_instantiate(&mut deps);
    mock_register_contracts(deps.as_mut());

    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.glow_emission_rate = Decimal256::one();
    STATE.save(deps.as_mut().storage, &state).unwrap();

    // USER 0 Deposits 20_000_000 uusd -----------------------------
    let msg = ExecuteMsg::Deposit {
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(ZERO_MATCH_SEQUENCE),
            String::from(ONE_MATCH_SEQUENCE),
        ]),
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );

    let mut env = mock_env();

    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Sponsor deposits 20_000_000 uusd ------------------------------
    let msg = ExecuteMsg::Sponsor {
        award: Some(false),
        prize_distribution: None,
    };

    let info = mock_info(
        "addr1111",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint256::from(2 * TICKET_PRICE).into(),
        }],
    );
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
    println!("{:?}", _res.attributes);

    let info = mock_info("addr0000", &[]);

    // Calculations

    // calculate the value of each deposit accounting for rounding errors
    let user_lottery_deposit_amount = (Uint256::from(2 * TICKET_PRICE)
        / Decimal256::permille(RATE)
        * Decimal256::percent(SPLIT_FACTOR))
        * Decimal256::permille(RATE);

    let sponsor_lottery_deposit_amount =
        Uint256::from(2 * TICKET_PRICE) / Decimal256::permille(RATE) * Decimal256::permille(RATE);

    // calculate the total minted_aust_value
    let total_lottery_deposit_amount = user_lottery_deposit_amount + sponsor_lottery_deposit_amount;

    // Move forward 100 blocks ------------------------------------
    env.block.height += 100;

    // Query the state --------------------------------------------
    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();
    println!("Global reward index: {:?}", state.global_reward_index);
    println!("Emission rate {:?}", state.glow_emission_rate);
    println!("Last reward updated {:?}", state.last_reward_updated);
    println!("Current height {:?}", env.block.height);

    // Claim rewards for user 1
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // assert that res has a message to send 50 GLOW (half of the total emission of 100)
    // from the distributor to addr0000
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "addr0000".to_string(),
                amount: (Decimal256::from_str("100").unwrap()
                    / Decimal256::from_uint256(total_lottery_deposit_amount)
                    * user_lottery_deposit_amount)
                    .into(),
            })
            .unwrap(),
        }))]
    );

    // Checking USER 0 state is correct
    let res: DepositorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Depositor {
                address: "addr0000".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();

    // USER 0 shouldn't have any pending rewards remaining
    assert_eq!(res.pending_rewards, Decimal256::zero());
    // The reward index of the USER should equal the global reward index
    assert_eq!(res.reward_index, state.global_reward_index);

    // Checking sponsor state is correct
    let res: SponsorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Sponsor {
                address: "addr1111".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    // assert that the sponsor has 50 GLOW pending rewards
    assert_eq!(
        res.pending_rewards,
        Decimal256::from_uint256(sponsor_lottery_deposit_amount) * state.global_reward_index
    );

    // assert that the user reward index equals the global_reward_index
    assert_eq!(res.reward_index, state.global_reward_index);

    // Move forward 100 blocks ------------------------------------
    env.block.height += 100;

    // query the state --------------------------------------------
    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();

    // Claim rewards for USER 0
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // assert that res has a message to send 50 GLOW (half of the total emission of 100)
    // from the distributor to addr0000
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "addr0000".to_string(),
                amount: (Decimal256::from_str("100").unwrap()
                    / Decimal256::from_uint256(total_lottery_deposit_amount)
                    * user_lottery_deposit_amount)
                    .into(),
            })
            .unwrap(),
        }))]
    );

    // Checking USER 0 state is correct
    let res: DepositorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Depositor {
                address: "addr0000".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();

    // USER 0 shouldn't have any pending rewards remaining
    assert_eq!(res.pending_rewards, Decimal256::zero());

    // the reward index of USER 0 should equal the global reward index
    assert_eq!(res.reward_index, state.global_reward_index);

    // Checking sponsor state is correct
    let res: SponsorInfoResponse = from_binary(
        &query(
            deps.as_ref(),
            env,
            QueryMsg::Sponsor {
                address: "addr1111".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();

    // assert the sponsors pending rewards
    assert_eq!(
        res.pending_rewards,
        Decimal256::from_uint256(sponsor_lottery_deposit_amount) * state.global_reward_index
    );

    // assert that the user reward index equals the global_reward_index
    assert_eq!(res.reward_index, state.global_reward_index);
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
            to_address: GOV_ADDR.to_string(),
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
            last_reward_updated: 12445,
            global_reward_index: Decimal256::zero(),
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            glow_emission_rate: Decimal256::one(),
            next_epoch: HOUR.mul(3).after(&env.block)
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ONE_MATCH_SEQUENCE)]),
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Add the funds to the contract address -------------------

    // Get the number of minted aust
    let minted_aust = Uint256::from(TICKET_PRICE) / Decimal256::permille(RATE);

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards savings
    let minted_savings_aust = minted_aust - minted_lottery_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * Decimal256::permille(RATE);

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
    assert_eq!(
        pool.total_user_savings_aust,
        contract_a_balance - contract_a_balance * Decimal256::percent(SPLIT_FACTOR)
    );

    // Check that the depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0001").unwrap()
        ),
        DepositorInfo {
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(ONE_MATCH_SEQUENCE)],
            unbonding_info: vec![]
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
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_lottery_deposits: minted_lottery_aust_value,
            total_user_savings_aust: minted_savings_aust,
            total_sponsor_lottery_deposits: Uint256::zero(),
        }
    );

    // Address withdraws a small amount of money ----------------

    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: Some(10u128.into()),
        instant: None,
    };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Get the amount to be withdrawn
    let depositor =
        read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap());

    // Get the amount of aust equivalent to the depositor's lottery deposit
    let depositor_lottery_aust = depositor.lottery_deposit / Decimal256::permille(RATE);

    // Calculate the depositor's aust balance
    let depositor_aust_balance = depositor.savings_aust + depositor_lottery_aust;

    // Calculate the depositor's balance from their aust balance
    let depositor_balance = depositor_aust_balance * Decimal256::permille(RATE);

    let withdraw_ratio = Decimal256::from_ratio(Uint256::from(10u128), depositor_balance);

    // Calculate the amount of savings aust to withdraw
    let withdrawn_savings_aust =
        uint256_times_decimal256_ceil(depositor.savings_aust, withdraw_ratio);

    // Withdrawn lottery deposit calculations

    let withdrawn_lottery_aust =
        uint256_times_decimal256_ceil(depositor_lottery_aust, withdraw_ratio);
    let ceil_withdrawn_lottery_aust_value =
        uint256_times_decimal256_ceil(withdrawn_lottery_aust, Decimal256::permille(RATE));

    // Total aust to redeem calculations

    // Get the total aust to redeem
    let total_aust_to_redeem = withdrawn_lottery_aust + withdrawn_savings_aust;

    // Get the value of the redeemed aust. aust_to_redeem * rate TODO = depositor_balance * withdraw_ratio
    let _total_aust_to_redeem_value = total_aust_to_redeem * Decimal256::permille(RATE);

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

    assert_eq!(Uint256::from(sent_amount), total_aust_to_redeem);

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
            lottery_deposit: minted_lottery_aust_value - ceil_withdrawn_lottery_aust_value,
            savings_aust: minted_savings_aust - withdrawn_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![Claim {
                amount: Uint256::from(sent_amount) * Decimal256::permille(RATE),
                release_at: WEEK.after(&env.block),
            }]
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
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_lottery_deposits: minted_lottery_aust_value
                - ceil_withdrawn_lottery_aust_value,
            total_sponsor_lottery_deposits: Uint256::zero(),
            total_user_savings_aust: minted_savings_aust - withdrawn_savings_aust,
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ONE_MATCH_SEQUENCE)]),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(TWO_MATCH_SEQUENCE)]),
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(ONE_MATCH_SEQUENCE)]),
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Add the funds to the contract address -------------------

    // Get the number of minted aust
    let minted_aust = Uint256::from(SMALL_TICKET_PRICE) / special_rate;

    // Get the number of minted aust that will go towards the lottery
    let minted_lottery_aust = minted_aust * Decimal256::percent(SPLIT_FACTOR);

    // Get the number of minted aust that will go towards savings
    let minted_savings_aust = minted_aust - minted_lottery_aust;

    // Get the value of minted aust going towards the lottery
    let minted_lottery_aust_value = minted_lottery_aust * special_rate;

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

    // Shares supply should equal contract_a_balance because no lottery has been executed yet
    // Shares supply should equal contract_a_balance times split factor
    assert_eq!(
        pool.total_user_savings_aust,
        contract_a_balance - contract_a_balance * Decimal256::percent(SPLIT_FACTOR)
    );

    // Check that the depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0001").unwrap()
        ),
        DepositorInfo {
            lottery_deposit: minted_lottery_aust_value,
            savings_aust: minted_savings_aust,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(ONE_MATCH_SEQUENCE)],
            unbonding_info: vec![]
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
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_user_lottery_deposits: minted_lottery_aust_value,
            total_user_savings_aust: minted_savings_aust,
            total_sponsor_lottery_deposits: Uint256::zero(),
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

    let pool = query_pool(deps.as_ref()).unwrap();

    let contract_a_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    // Assert that Lotto pool is solvent
    assert!(
        (contract_a_balance - pool.total_user_savings_aust) * special_rate
            >= pool.total_user_lottery_deposits + pool.total_sponsor_lottery_deposits
    )
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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(TWO_MATCH_SEQUENCE)]),
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

        assert!(percent_appreciation_towards_lottery <= Decimal256::percent(SPLIT_FACTOR));

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

        assert!(percent_appreciation_towards_lottery <= Decimal256::percent(SPLIT_FACTOR));

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
        encoded_tickets: combinations_to_encoded_tickets(vec![String::from(TWO_MATCH_SEQUENCE)]),
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

        assert!(percent_appreciation_towards_lottery <= Decimal256::percent(90));

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
        encoded_tickets: combinations_to_encoded_tickets(vec![
            String::from(THREE_MATCH_SEQUENCE),
            String::from(FOUR_MATCH_SEQUENCE),
        ]),
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
    let encoded_tickets = combinations_to_encoded_tickets(combinations.clone());
    println!("{}", encoded_tickets);
    let decoded_combinations = encoded_tickets_to_combinations(encoded_tickets).unwrap();
    println!("{:?}", decoded_combinations);
    assert_eq!(combinations, decoded_combinations);

    // Test inverse functionality #2
    let combinations = vec![String::from("000000")];
    // TODO Understand why I have to clone in the following line
    let encoded_tickets = combinations_to_encoded_tickets(combinations.clone());
    let decoded_combinations = encoded_tickets_to_combinations(encoded_tickets).unwrap();
    println!("{:?}", decoded_combinations);
    assert_eq!(combinations, decoded_combinations);

    // Test giving random data
    let encoded_tickets = String::from("aowief");
    let decoded_combinations = encoded_tickets_to_combinations(encoded_tickets);
    match decoded_combinations {
        Err(_) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Test giving data with wrong ticket length
    let encoded_tickets = String::from("EjRWeA==");
    let decoded_combinations = encoded_tickets_to_combinations(encoded_tickets);
    match decoded_combinations {
        Err(_) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

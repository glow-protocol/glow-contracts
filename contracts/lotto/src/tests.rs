use crate::contract::{
    execute, instantiate, query, query_config, query_pool, query_state, query_ticket_info,
    INITIAL_DEPOSIT_AMOUNT,
};
use crate::helpers::calculate_winner_prize;
use crate::mock_querier::{mock_dependencies, mock_env, mock_info, MOCK_CONTRACT_ADDR};
use crate::state::{
    query_prizes, read_depositor_info, read_lottery_info, read_sponsor_info, DepositorInfo,
    LotteryInfo, PrizeInfo, STATE,
};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, coin, from_binary, to_binary, Addr, Api, BankMsg, Coin, CosmosMsg, Decimal, Deps,
    DepsMut, Env, Response, SubMsg, Timestamp, Uint128, WasmMsg,
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
use std::ops::{Add, Div, Mul};
use std::str::FromStr;

const TEST_CREATOR: &str = "creator";
const ANCHOR: &str = "anchor";
const A_UST: &str = "aterra-ust";
const DENOM: &str = "uusd";
const GOV_ADDR: &str = "gov";
const DISTRIBUTOR_ADDR: &str = "distributor";
const ORACLE_ADDR: &str = "oracle";

const TICKET_PRICE: u64 = 1_000_000_000; // 10_000_000 as %
const SPLIT_FACTOR: u64 = 75; // as a %
const INSTANT_WITHDRAWAL_FEE: u64 = 10; // as a %
const RESERVE_FACTOR: u64 = 5; // as a %
const MAX_HOLDERS: u8 = 10;
const RATE: u64 = 1023; // as a permille
const WEEK_TIME: u64 = 604800; // in seconds
const HOUR_TIME: u64 = 3600; // in seconds
const ROUND_DELTA: u64 = 10;
const FIRST_LOTTO_TIME: u64 = 1595961494; // timestamp between deployment and 1 week after
const WINNING_SEQUENCE: &str = "be1ce";

// generate glow instantiate msg
pub(crate) fn instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        owner: TEST_CREATOR.to_string(),
        stable_denom: DENOM.to_string(),
        anchor_contract: ANCHOR.to_string(),
        aterra_contract: A_UST.to_string(),
        oracle_contract: ORACLE_ADDR.to_string(),
        lottery_interval: WEEK_TIME,
        block_time: HOUR_TIME,
        round_delta: ROUND_DELTA,
        ticket_price: Decimal256::percent(TICKET_PRICE),
        max_holders: MAX_HOLDERS,
        prize_distribution: [
            Decimal256::zero(),
            Decimal256::zero(),
            Decimal256::percent(5),
            Decimal256::percent(15),
            Decimal256::percent(30),
            Decimal256::percent(50),
        ],
        target_award: Decimal256::zero(),
        reserve_factor: Decimal256::percent(RESERVE_FACTOR),
        split_factor: Decimal256::percent(SPLIT_FACTOR),
        instant_withdrawal_fee: Decimal256::percent(INSTANT_WITHDRAWAL_FEE),
        unbonding_period: WEEK_TIME,
        initial_emission_rate: Decimal256::zero(),
        initial_lottery_execution: FIRST_LOTTO_TIME,
    }
}

//
fn mock_instantiate(deps: DepsMut) -> Response {
    let msg = instantiate_msg();

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
    assert_eq!(0, res.messages.len());

    let config = query_config(deps.as_ref()).unwrap();

    // assert that config is initialized properly
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
            block_time: HOUR,
            round_delta: ROUND_DELTA,
            ticket_price: Decimal256::percent(TICKET_PRICE),
            max_holders: MAX_HOLDERS,
            prize_distribution: [
                Decimal256::zero(),
                Decimal256::zero(),
                Decimal256::percent(5),
                Decimal256::percent(15),
                Decimal256::percent(30),
                Decimal256::percent(50)
            ],
            target_award: Decimal256::zero(),
            reserve_factor: Decimal256::percent(RESERVE_FACTOR),
            split_factor: Decimal256::percent(SPLIT_FACTOR),
            instant_withdrawal_fee: Decimal256::percent(INSTANT_WITHDRAWAL_FEE),
            unbonding_period: WEEK
        }
    );

    // Register contracts
    let msg = ExecuteMsg::RegisterContracts {
        gov_contract: GOV_ADDR.to_string(),
        distributor_contract: DISTRIBUTOR_ADDR.to_string(),
    };

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();
    let config = query_config(deps.as_ref()).unwrap();

    // assert that the gov_contract address in the config is correct
    assert_eq!(config.gov_contract, GOV_ADDR.to_string());

    // assert that the distributor contract in the config is correct
    assert_eq!(config.distributor_contract, DISTRIBUTOR_ADDR.to_string());

    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();

    // assert that the inital state is correct
    assert_eq!(
        state,
        StateResponse {
            total_tickets: Uint256::zero(),
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: (WEEK + HOUR).unwrap().after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    let pool = query_pool(deps.as_ref()).unwrap();

    // assert that the pools are all empty
    assert_eq!(
        pool,
        PoolResponse {
            total_deposits: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            total_sponsor_amount: Decimal256::zero(),
            lottery_shares: Decimal256::zero(),
            sponsor_shares: Decimal256::zero(),
            deposit_shares: Decimal256::zero(),
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

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // update owner
    let info = mock_info(TEST_CREATOR, &[]);

    let msg = ExecuteMsg::UpdateConfig {
        owner: Some("owner1".to_string()),
        oracle_addr: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
        reserve_factor: None,
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

    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check lottery_interval has changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.lottery_interval, Duration::Time(1800));

    // update reserve_factor to 1%
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        oracle_addr: None,
        reserve_factor: Some(Decimal256::percent(1)),
        instant_withdrawal_fee: None,
        unbonding_period: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check reserve_factor has changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.reserve_factor, Decimal256::percent(1));

    // check only owner can update config
    let info = mock_info("owner2", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        oracle_addr: None,
        owner: Some(String::from("new_owner")),
        reserve_factor: None,
        instant_withdrawal_fee: None,
        unbonding_period: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::Unauthorized {}) => {}
        _ => panic!("Must return unauthorized error"),
    }
}

#[test]
fn deposit() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Return InvalidDepositAmount because you tried to specify two tickets combinations
    // but only deposited enough for a single ticket.
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("34567")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "ukrw".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg.clone());
    match res {
        Err(ContractError::InvalidDepositAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Return InvalidDepositAmount because you tried to specify two ticket combinations
    // but didn't send any funds.
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint128::zero(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidDepositAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Return InvalidSequence because you tried to specify a ticket combination with 6 digits instead of 5.
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("135797"), String::from("34567")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );
    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Return InvalidSequence because you tried to specify a ticket combination with 4 digits instead of 5.
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("3457")],
    };

    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - only hex values allowed
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("135w9"), String::from("34567")],
    };
    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Set the mock aUST-UST exchange rate for the mock querier.
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // Correctly purchase two tickets
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("34567")],
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check that the address of the sender was stored correctly in both sequence buckets
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("13579"))
            .unwrap()
            .holders,
        vec![Addr::unchecked("addr0000")]
    );
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("34567"))
            .unwrap()
            .holders,
        vec![Addr::unchecked("addr0000")]
    );

    // Check that the depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0000").unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE * 2u64),
            shares: Decimal256::percent(TICKET_PRICE * 2u64) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("13579"), String::from("34567")],
            unbonding_info: vec![]
        }
    );

    let minted_shares = Decimal256::percent(TICKET_PRICE * 2u64).div(Decimal256::permille(RATE));

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(2u64),
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: (WEEK + HOUR).unwrap().after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_deposits: Decimal256::percent(TICKET_PRICE * 2u64),
            lottery_deposits: Decimal256::percent(TICKET_PRICE * 2u64)
                * Decimal256::percent(SPLIT_FACTOR),
            total_sponsor_amount: Decimal256::zero(),
            lottery_shares: minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            deposit_shares: minted_shares - minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            sponsor_shares: Decimal256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: String::from("uusd"),
                amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
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
                Decimal256::percent(TICKET_PRICE * 2u64).to_string()
            ),
            attr("tickets", 2u64.to_string()),
            attr(
                "shares_minted",
                (Decimal256::percent(TICKET_PRICE * 2u64) / Decimal256::permille(RATE)).to_string()
            ),
        ]
    );

    // test round-up tickets
    let deposit_amount =
        Decimal256::percent(TICKET_PRICE) * Decimal256::from_ratio(3, 2) * Uint256::one();

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: deposit_amount.into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("14657")],
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 3);

    // deposit again
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("19876")],
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 5);

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("45637")],
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 6);

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("45639")],
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 8);

    // Test sequential buys of the same ticket by the same address (should fail)
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("88888")],
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("88888")],
    };

    // We let users have a repeated ticket
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Ticket is already owner by 10 holders
    let addresses_count = 10u64;
    let addresses_range = 0..addresses_count;
    let addresses = addresses_range
        .map(|c| format!("addr{:0>4}", c))
        .collect::<Vec<String>>();

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    for (_index, address) in addresses.iter().enumerate() {
        // Users buys winning ticket
        let msg = ExecuteMsg::Deposit {
            combinations: vec![String::from("66666")],
        };
        let info = mock_info(
            address.as_str(),
            &[Coin {
                denom: "uusd".to_string(),
                amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
            }],
        );

        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    }

    let holders = query_ticket_info(deps.as_ref(), String::from("66666"))
        .unwrap()
        .holders;
    println!("holders: {:?}", holders);
    println!("len: {:?}", holders.len());

    // 11th holder with same sequence, should fail
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("66666")],
    };
    let info = mock_info(
        "addr1111",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidHolderSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

#[test]
fn deposit_with_tax() {
    // Get mock dependencies and set the tax rate
    let mut deps = mock_dependencies(&[]);
    deps.querier.with_tax(
        Decimal::percent(1),
        &[(&"uusd".to_string(), &Uint128::from(1_000_000u128))],
    );

    // Initialize contract
    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Return InvalidDepositAmount because you tried to specify two tickets combinations
    // but only deposited enough for a single ticket.
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("34567")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "ukrw".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg.clone());
    match res {
        Err(ContractError::InvalidDepositAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Return InvalidDepositAmount because you tried to specify two ticket combinations
    // but didn't send any funds.
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: Uint128::zero(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidDepositAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Return InvalidSequence because you tried to specify a ticket combination with 6 digits instead of 5.
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("135797"), String::from("34567")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );
    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Return InvalidSequence because you tried to specify a ticket combination with 4 digits instead of 5.
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("3457")],
    };

    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - only hex values allowed
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("135w9"), String::from("34567")],
    };
    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Set the mock aUST-UST exchange rate for the mock querier.
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // Correctly purchase two tickets
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("34567")],
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check that the address of the sender was stored correctly in both sequence buckets
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("13579"))
            .unwrap()
            .holders,
        vec![Addr::unchecked("addr0000")]
    );
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("34567"))
            .unwrap()
            .holders,
        vec![Addr::unchecked("addr0000")]
    );

    let post_tax_deposit_amount = deduct_tax(
        deps.as_ref(),
        Coin {
            denom: String::from("uusd"),
            amount: Uint128::from(Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()),
        },
    )
    .unwrap()
    .amount;

    let minted_shares =
        Decimal256::from_uint256(post_tax_deposit_amount) / Decimal256::permille(RATE);

    // Check that the depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0000").unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::from_uint256(post_tax_deposit_amount),
            shares: minted_shares,
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("13579"), String::from("34567")],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(2u64),
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&mock_env().block),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: (WEEK + HOUR).unwrap().after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    // Assert that the pool was updated properly
    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_deposits: Decimal256::from_uint256(post_tax_deposit_amount),
            lottery_deposits: Decimal256::from_uint256(post_tax_deposit_amount)
                * Decimal256::percent(SPLIT_FACTOR),
            total_sponsor_amount: Decimal256::zero(),
            lottery_shares: minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            deposit_shares: minted_shares - minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            sponsor_shares: Decimal256::zero(),
        }
    );

    // Assert that res contains a message for depositing the coins in anchor
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: String::from("uusd"),
                amount: post_tax_deposit_amount,
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        }))]
    );

    // Assert that the res attributes are correct
    assert_eq!(
        res.attributes,
        vec![
            attr("action", "deposit"),
            attr("depositor", "addr0000"),
            attr("recipient", "addr0000"),
            attr(
                "deposit_amount",
                Decimal256::percent(TICKET_PRICE * 2u64).to_string()
            ),
            attr("tickets", 2u64.to_string()),
            attr("shares_minted", minted_shares.to_string()),
        ]
    );

    // Test the ticket round-up functionality
    // Deposit 10/6 the price of a ticket which is a bit more than 1.5 times the price of a ticket
    // So that the user will still be eligible for a round-up ticket post taxes
    // ------------------------------------------------------------------
    let deposit_amount =
        Decimal256::percent(TICKET_PRICE) * Decimal256::from_ratio(10, 6) * Uint256::one();

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: deposit_amount.into(),
        }],
    );

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("14657")],
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 3);

    // Deposit again
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("19876")],
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 5);

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("45637")],
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 6);

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("45639")],
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let depositor_info = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0000").unwrap(),
    );

    assert_eq!(depositor_info.tickets.len(), 8);

    // Test sequential buys of the same ticket by the same address (should fail after the max ticket holders is reached)
    // -------------------------------------------------------------------------
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("88888")],
    };

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("88888")],
    };

    // We let users have a repeated ticket
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Ticket is already owner by 10 holders
    let addresses_count = 10u64;
    let addresses_range = 0..addresses_count;
    let addresses = addresses_range
        .map(|c| format!("addr{:0>4}", c))
        .collect::<Vec<String>>();

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    for (_index, address) in addresses.iter().enumerate() {
        // Users buys winning ticket
        let msg = ExecuteMsg::Deposit {
            combinations: vec![String::from("66666")],
        };
        let info = mock_info(
            address.as_str(),
            &[Coin {
                denom: "uusd".to_string(),
                amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
            }],
        );

        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    }

    let holders = query_ticket_info(deps.as_ref(), String::from("66666"))
        .unwrap()
        .holders;

    assert_eq!(holders.len(), 10);

    // 11th holder with same sequence, should fail
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("66666")],
    };
    let info = mock_info(
        "addr1111",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidHolderSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

#[test]
fn gift_tickets() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Must deposit stable_denom coins
    let msg = ExecuteMsg::Gift {
        combinations: vec![String::from("13579"), String::from("34567")],
        recipient: "addr1111".to_string(),
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "ukrw".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg.clone());
    match res {
        Err(ContractError::InvalidGiftAmount {}) => {}
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
        Err(ContractError::InvalidGiftAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let wrong_amount = Decimal256::percent(TICKET_PRICE);

    // correct base denom, deposit different to TICKET_PRICE
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (wrong_amount * Uint256::one()).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg);

    match res {
        Err(ContractError::InsufficientGiftDepositAmount(amount_required)) => {
            assert_eq!(amount_required, 2u64);
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid recipient - you cannot make a gift to yourself
    let msg = ExecuteMsg::Gift {
        combinations: vec![String::from("13597"), String::from("34567")],
        recipient: "addr0000".to_string(),
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );
    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidGift {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - more number of digits
    let msg = ExecuteMsg::Gift {
        combinations: vec![String::from("135797"), String::from("34567")],
        recipient: "addr1111".to_string(),
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );
    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - less number of digits
    let msg = ExecuteMsg::Gift {
        combinations: vec![String::from("13579"), String::from("3457")],
        recipient: "addr1111".to_string(),
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );
    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - only numbers allowed
    let msg = ExecuteMsg::Gift {
        combinations: vec![String::from("135w9"), String::from("34567")],
        recipient: "addr1111".to_string(),
    };

    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Correct gift - gifts two tickets
    let msg = ExecuteMsg::Gift {
        combinations: vec![String::from("13579"), String::from("34567")],
        recipient: "addr1111".to_string(),
    };

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check address of sender was stored correctly in both sequence buckets
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("13579"))
            .unwrap()
            .holders,
        vec![deps.api.addr_validate("addr1111").unwrap()]
    );
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("34567"))
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
            deposit_amount: Decimal256::percent(TICKET_PRICE * 2u64),
            shares: Decimal256::percent(TICKET_PRICE * 2u64) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("13579"), String::from("34567")],
            unbonding_info: vec![]
        }
    );

    let minted_shares = Decimal256::percent(TICKET_PRICE * 2u64).div(Decimal256::permille(RATE));

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(2u64),
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: (WEEK + HOUR).unwrap().after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_deposits: Decimal256::percent(TICKET_PRICE * 2u64),
            lottery_deposits: Decimal256::percent(TICKET_PRICE * 2u64)
                * Decimal256::percent(SPLIT_FACTOR),
            total_sponsor_amount: Decimal256::zero(),
            lottery_shares: minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            deposit_shares: minted_shares - minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            sponsor_shares: Decimal256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: DENOM.to_string(),
                amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
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
                Decimal256::percent(TICKET_PRICE * 2u64).to_string()
            ),
            attr("tickets", 2u64.to_string()),
            attr(
                "shares_minted",
                (Decimal256::percent(TICKET_PRICE * 2u64) / Decimal256::permille(RATE)).to_string()
            ),
        ]
    );
}

#[test]
fn sponsor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    let sponsor_amount = 100_000_000u128;

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

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

    let msg = ExecuteMsg::Sponsor { award: None };

    let _res = execute(deps.as_mut(), mock_env(), info, msg);
    println!("{:?}", _res);

    let sponsor_info = read_sponsor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0001").unwrap(),
    );

    let pool = query_pool(deps.as_ref()).unwrap();

    let net_amount = deduct_tax(
        deps.as_ref(),
        Coin {
            denom: String::from("uusd"),
            amount: Uint128::from(sponsor_amount),
        },
    )
    .unwrap()
    .amount;

    let minted_shares = Decimal256::from_uint256(net_amount) / Decimal256::permille(RATE);

    assert_eq!(sponsor_info.amount, Decimal256::from_uint256(net_amount));
    assert_eq!(sponsor_info.shares, minted_shares);

    assert_eq!(
        pool.total_sponsor_amount,
        Decimal256::from_uint256(net_amount)
    );
    assert_eq!(pool.sponsor_shares, minted_shares);

    // withdraw sponsor
    // adding 1 to account for rounding error
    let app_shares = (Decimal256::from_uint256(net_amount + Uint128::from(1u128))
        / Decimal256::permille(RATE))
        * Uint256::one();

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &app_shares.into())],
    )]);

    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::SponsorWithdraw {};
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let sponsor_info = read_sponsor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0001").unwrap(),
    );

    let pool = query_pool(deps.as_ref()).unwrap();

    assert_eq!(sponsor_info.amount, Decimal256::zero());
    assert_eq!(sponsor_info.shares, Decimal256::zero());
    assert_eq!(pool.total_sponsor_amount, Decimal256::zero());
    assert_eq!(pool.sponsor_shares, Decimal256::zero());
}

#[test]
fn withdraw() {
    // Initialize contract
    let mut deps = mock_dependencies(&[Coin {
        denom: DENOM.to_string(),
        amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
    }]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    let deposit_amount = (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into();

    // Address buys one ticket
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: deposit_amount,
        }],
    );

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("23456")],
    };

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Add 1 to account for rounding error
    let shares = Uint256::one()
        + (Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE)) * Uint256::one();

    let info = mock_info("addr0001", &[]);

    let msg = ExecuteMsg::Withdraw {
        amount: None,
        instant: None,
    };

    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR.to_string(),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT) + deposit_amount,
        }],
    );

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &shares.into())],
    )]);

    // Correct withdraw, user has 1 ticket to be withdrawn
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let empty_addr: Vec<Addr> = vec![];
    // Check address of sender was removed correctly in the sequence bucket
    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("23456"))
            .unwrap()
            .holders,
        empty_addr
    );

    deps.querier.with_tax(
        Decimal::percent(1),
        &[(&"uusd".to_string(), &Uint128::from(1000000u128))],
    );

    let _redeem_amount = deduct_tax(
        deps.as_ref(),
        Coin {
            denom: String::from("uusd"),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        },
    )
    .unwrap()
    .amount;

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0001").unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![Claim {
                amount: Decimal256::from_uint256(Uint256::from(10000000u128)),
                release_at: WEEK.after(&mock_env().block),
            }]
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::zero(),
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: (WEEK + HOUR).unwrap().after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_deposits: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            total_sponsor_amount: Decimal256::zero(),
            lottery_shares: Decimal256::zero(),
            deposit_shares: Decimal256::zero(),
            sponsor_shares: Decimal256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: A_UST.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: ANCHOR.to_string(),
                amount: shares.into(),
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
            attr("redeem_amount_anchor", shares.to_string()),
            attr(
                "redeem_stable_amount",
                Decimal256::from_str("10000000").unwrap().to_string()
            ),
            attr("instant_withdrawal_fee", Decimal256::zero().to_string())
        ]
    );

    // Withdraw with a given amount
    for index in 0..10 {
        // Users buys winning ticket
        let msg = ExecuteMsg::Deposit {
            combinations: vec![format!("{:0>5}", index)],
        };
        let info = mock_info(
            "addr2222",
            &[Coin {
                denom: "uusd".to_string(),
                amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
            }],
        );

        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    }

    let dep = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr2222").unwrap(),
    );

    println!("depositor: {:?}", dep);
    // Add 1 to account for rounding error
    let shares = Uint256::one()
        + (Decimal256::percent(TICKET_PRICE * 10) / Decimal256::permille(RATE)) * Uint256::one();

    let info = mock_info("addr2222", &[]);

    // Withdraws half of its tickets
    let msg = ExecuteMsg::Withdraw {
        amount: Some((Decimal256::percent(5 * TICKET_PRICE) * Uint256::one()).into()),
        instant: None,
    };

    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR.to_string(),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT) + deposit_amount,
        }],
    );

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &shares.into())],
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
            String::from("00005"),
            String::from("00006"),
            String::from("00007"),
            String::from("00008"),
            String::from("00009")
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
        query_ticket_info(deps.as_ref(), String::from("00002"))
            .unwrap()
            .holders,
        empty_addr
    );

    assert_eq!(
        query_ticket_info(deps.as_ref(), String::from("00005"))
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
            String::from("00006"),
            String::from("00007"),
            String::from("00008"),
            String::from("00009")
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
        query_ticket_info(deps.as_ref(), String::from("00005"))
            .unwrap()
            .holders,
        empty_addr
    );
}

#[test]
fn instant_withdraw() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    let deposit_amount = (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into();

    // Address buys one ticket
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: DENOM.to_string(),
            amount: deposit_amount,
        }],
    );

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("23456")],
    };

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let shares = Uint256::one()
        + (Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE)) * Uint256::one();

    let info = mock_info("addr0001", &[]);

    let msg = ExecuteMsg::Withdraw {
        amount: None,
        instant: Some(true),
    };

    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR.to_string(),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT) + deposit_amount,
        }],
    );

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &shares.into())],
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

    deps.querier.with_tax(
        Decimal::percent(1),
        &[(&"uusd".to_string(), &Uint128::from(1000000u128))],
    );

    let _redeem_amount = deduct_tax(
        deps.as_ref(),
        Coin {
            denom: String::from("uusd"),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        },
    )
    .unwrap()
    .amount;

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap()),
        DepositorInfo {
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
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
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: (WEEK + HOUR).unwrap().after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_deposits: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            total_sponsor_amount: Decimal256::zero(),
            lottery_shares: Decimal256::zero(),
            deposit_shares: Decimal256::zero(),
            sponsor_shares: Decimal256::zero(),
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
                    amount: shares.into(),
                    msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
                })
                .unwrap(),
            })),
            SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                to_address: info.sender.to_string(),
                amount: vec![Coin {
                    denom: "uusd".to_string(),
                    amount: Uint128::from(9000000u128)
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
            attr("redeem_amount_anchor", shares.to_string()),
            attr(
                "redeem_stable_amount",
                Decimal256::from_str("9000000").unwrap().to_string()
            ),
            attr(
                "instant_withdrawal_fee",
                Decimal256::from_str("1000000").unwrap().to_string()
            )
        ]
    )
}

#[test]
fn claim() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Address buys one ticket
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("23456")],
    };

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Address withdraws one ticket
    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: None,
        instant: None,
    };

    // Add one to account for rounding error
    let shares = Uint256::one()
        + (Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE)) * Uint256::one();

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &shares.into())],
    )]);

    let shares = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();
    let pool = query_pool(deps.as_ref()).unwrap();
    println!("shares: {}", shares);
    println!("pooled_deposits: {}", shares * Decimal256::permille(RATE));
    println!("total deposits: {}", pool.total_deposits);

    // Correct withdraw, user has 1 ticket to be withdrawn
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Claim amount that you don't have, should fail
    let info = mock_info("addr0002", &[]);
    let msg = ExecuteMsg::Claim {};

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InsufficientClaimableFunds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    //TODO: test insufficient funds in contract

    // Claim amount that you have, but still in unbonding state, should fail
    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Claim {};

    let mut env = mock_env();

    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg);
    match res {
        Err(ContractError::InsufficientClaimableFunds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let msg = ExecuteMsg::Claim {};

    println!("Block time 1: {}", env.block.time);

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time * 2);
    }
    println!("Block time 2: {}", env.block.time);

    // TODO: change also the exchange rate here
    // This update is not needed (??)
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: DENOM.to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT + 10000000u128),
        }],
    );

    let dep = read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap());

    println!("DepositorInfo: {:x?}", dep);

    // Claim amount is already unbonded, so claim execution should work
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap()),
        DepositorInfo {
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
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
                amount: Uint128::from(10_000_000u64),
            }],
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "claim_unbonded"),
            attr("depositor", "addr0001"),
            attr("redeemed_amount", 10_000_000u64.to_string()),
        ]
    );
}

#[test]
fn claim_lottery_single_winner() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    // Users buys winning ticket
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from(WINNING_SEQUENCE)],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw = deps.api.addr_validate("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(WINNING_SEQUENCE)],
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

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let exec_height = env.block.height;

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Check lottery info was updated correctly
    let pool = query_pool(deps.as_ref()).unwrap();

    let total_prize = calculate_total_prize(
        deps.as_ref(),
        pool.lottery_shares,
        pool.deposit_shares,
        pool.sponsor_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(20_000_000u128),
        1,
    );

    let awarded_prize = total_prize * Decimal256::percent(50);
    println!("awarded_prize: {}", awarded_prize);

    let lottery = read_lottery_info(deps.as_ref().storage, 0u64);
    assert_eq!(
        lottery,
        LotteryInfo {
            rand_round: 20170,
            sequence: WINNING_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 0, 0, 0, 1],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(
        prizes,
        PrizeInfo {
            claimed: false,
            matches: [0, 0, 0, 0, 0, 1]
        }
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero(),);

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

    let info = mock_info("addr0000", &[]);
    let msg = ExecuteMsg::ClaimLottery {
        lottery_ids: Vec::from([0u64]),
    };

    // Claim lottery should work, even if there are no unbonded claims
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let mut prize = calculate_winner_prize(
        awarded_prize,
        prizes.matches,
        lottery.number_winners,
        query_config(deps.as_ref()).unwrap().prize_distribution,
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(
        prizes,
        PrizeInfo {
            claimed: true,
            matches: [0, 0, 0, 0, 0, 1]
        }
    );

    //deduct reserve fee
    let config = query_config(deps.as_ref()).unwrap();
    let reserve_fee = Uint256::from(prize) * config.reserve_factor;
    prize -= Uint128::from(reserve_fee);

    //check total_reserve
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.total_reserve, Decimal256::from_uint256(reserve_fee));

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
            to_address: "addr0000".to_string(),
            amount: vec![Coin {
                denom: String::from("uusd"),
                amount: prize,
            }],
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "claim_lottery"),
            attr("lottery_ids", "[0]"),
            attr("depositor", "addr0000"),
            attr("redeemed_amount", prize.to_string()),
        ]
    );
}

#[test]
fn execute_lottery() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
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
        Err(ContractError::LotteryInProgress {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time);
    }

    // Add 100 UST to our contract balance
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: DENOM.to_string(),
            amount: Uint128::from(100_000_000u128),
        }],
    );

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
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("34567")],
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // TODO: Test with 10 and 20, to check the pooled_deposits if statement
    // Add 10 aUST to our contract balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(10_000_000u128),
        )],
    )]);

    // Execute lottery, now with tickets
    let lottery_msg = ExecuteMsg::ExecuteLottery {};
    let info = mock_info("addr0001", &[]);
    let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_lottery"),
            attr("redeemed_amount", "0"),
        ]
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
        Expiration::AtTime(env.block.time).add(WEEK).unwrap()
    );

    // Directly check next_lottery_time has been set up for next week
    let next_lottery_time = query_state(deps.as_ref(), mock_env(), None)
        .unwrap()
        .next_lottery_time;

    assert_eq!(
        next_lottery_time,
        Expiration::AtTime(env.block.time).add(WEEK).unwrap()
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

    // TODO: In this case, there should be a redeemd submsg as pooled_deposits > deposits
    // Add 20 aUST to our contract balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(20_000_000u128),
        )],
    )]);

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time * 2);
    }

    // Execute 2nd lottery
    let lottery_msg = ExecuteMsg::ExecuteLottery {};
    let info = mock_info("addr0001", &[]);
    let res = execute(deps.as_mut(), env.clone(), info.clone(), lottery_msg).unwrap();

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: A_UST.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: ANCHOR.to_string(),
                amount: Uint128::from(337242u128), // TODO: Do the math, not hard-coded value
                msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
            })
            .unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_lottery"),
            attr("redeemed_amount", "337242"),
        ]
    );

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    // Execute prize
    let _res = execute(deps.as_mut(), env.clone(), info, execute_prize_msg).unwrap();

    // Directly check next_lottery_time has been set up for next week
    let next_lottery_time = query_state(deps.as_ref(), mock_env(), None)
        .unwrap()
        .next_lottery_time;

    assert_eq!(
        next_lottery_time,
        Expiration::AtTime(env.block.time).add(WEEK).unwrap()
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    println!("state: {:?}", state);
}

#[test]
fn execute_lottery_no_tickets() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
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

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: DENOM.to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    // Users buys a non-winning ticket
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("11111")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw = deps.api.addr_validate("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("11111")],
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

    // Run lottery, one winner (5 hits) - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let exec_height = env.block.height;
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Check lottery info was updated correctly
    let awarded_prize = Decimal256::zero();

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: WINNING_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            total_prizes: awarded_prize,
            number_winners: [0; 6],
            page: "".to_string()
        }
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero());

    // TODO: Calculate and avoid hard-coding
    assert_eq!(
        state.award_available,
        Decimal256::from_str("107844998").unwrap()
    );

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

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    // Users buys winning ticket
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from(WINNING_SEQUENCE)],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw = deps.api.addr_validate("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(WINNING_SEQUENCE)],
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

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let exec_height = env.block.height;

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }
    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Check lottery info was updated correctly

    let pool = query_pool(deps.as_ref()).unwrap();

    let total_prize = calculate_total_prize(
        deps.as_ref(),
        pool.lottery_shares,
        pool.deposit_shares,
        pool.sponsor_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(20_000_000u128),
        1,
    );

    let awarded_prize = total_prize * Decimal256::percent(50);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: WINNING_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 0, 0, 0, 1],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 0, 0, 0, 1]);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero(),);

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

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
fn execute_prize_winners_diff_ranks() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // Users buys winning ticket - 5 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from(WINNING_SEQUENCE)],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_0 = deps.api.addr_validate("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_0),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from(WINNING_SEQUENCE)],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 2 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("be000")],
    };
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_1 = deps.api.addr_validate("addr0001").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_1),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("be000")],
            unbonding_info: vec![]
        }
    );

    // Run lottery, one winner (5 hits), one winner (2 hits) - should run correctly
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

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let exec_height = env.block.height;

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Check lottery info was updated correctly
    let pool = query_pool(deps.as_ref()).unwrap();

    let total_prize = calculate_total_prize(
        deps.as_ref(),
        pool.lottery_shares,
        pool.deposit_shares,
        pool.sponsor_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(30_000_000u128),
        2,
    );

    let awarded_prize_0 = total_prize * Decimal256::percent(50);
    let awarded_prize_1 = total_prize * Decimal256::percent(5);
    let awarded_prize = awarded_prize_0 + awarded_prize_1;

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: WINNING_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 1, 0, 0, 1],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw_0, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 0, 0, 0, 1]);

    let prizes = query_prizes(deps.as_ref(), &address_raw_1, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 1, 0, 0, 0]);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(state.current_lottery, 1u64);

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

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
fn execute_prize_winners_same_rank() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // Users buys winning ticket - 4 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("be1c0")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_0 = deps.api.addr_validate("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_0),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("be1c0")],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 4 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("be1c0")],
    };
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_1 = deps.api.addr_validate("addr0001").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_1),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("be1c0")],
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

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let exec_height = env.block.height;

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let pool = query_pool(deps.as_ref()).unwrap();
    // total prize
    let total_prize = calculate_total_prize(
        deps.as_ref(),
        pool.lottery_shares,
        pool.deposit_shares,
        pool.sponsor_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(30_000_000u128),
        2,
    );

    let awarded_prize = total_prize * Decimal256::percent(30);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: WINNING_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 0, 0, 2, 0],
            page: "".to_string()
        }
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero(),);

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

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
fn execute_prize_one_winner_multiple_ranks() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // Users buys winning ticket - 5 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from(WINNING_SEQUENCE)],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("be1c4")],
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("be1c5")],
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("be1c6")],
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("b01ce")],
    };
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw = deps.api.addr_validate("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            deposit_amount: Decimal256::percent(5 * TICKET_PRICE),
            shares: Decimal256::percent(5 * TICKET_PRICE) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![
                String::from(WINNING_SEQUENCE),
                String::from("be1c4"),
                String::from("be1c5"),
                String::from("be1c6"),
                String::from("b01ce")
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

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let exec_height = env.block.height;

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let pool = query_pool(deps.as_ref()).unwrap();

    // total prize
    let total_prize = calculate_total_prize(
        deps.as_ref(),
        pool.lottery_shares,
        pool.deposit_shares,
        pool.sponsor_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(55_000_000u128),
        5,
    );

    let awarded_prize = total_prize * Decimal256::percent(50 + 30);

    println!(
        "lottery_info: {:x?}",
        read_lottery_info(deps.as_ref().storage, 0u64)
    );

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: WINNING_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 0, 0, 3, 1],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 0, 0, 3, 1]);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero());

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

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
fn execute_prize_multiple_winners_one_ticket() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from(WINNING_SEQUENCE)],
    };

    // User 0 buys winning ticket - 5 hits
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap();

    // User 1 buys winning ticket - 5 hits
    let info = mock_info(
        "addr1111",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap();

    // User 2 buys winning ticket - 5 hits
    let info = mock_info(
        "addr2222",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_0 = deps.api.addr_validate("addr0000").unwrap();
    let address_1 = deps.api.addr_validate("addr1111").unwrap();
    let address_2 = deps.api.addr_validate("addr2222").unwrap();

    let ticket = query_ticket_info(deps.as_ref(), String::from(WINNING_SEQUENCE)).unwrap();

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

    // Execute Lottery
    let msg = ExecuteMsg::ExecuteLottery {};
    let exec_height = env.block.height;

    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Advance block_time in time
    if let Duration::Time(time) = HOUR {
        env.block.time = env.block.time.plus_seconds(time);
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let pool = query_pool(deps.as_ref()).unwrap();

    // total prize
    let total_prize = calculate_total_prize(
        deps.as_ref(),
        pool.lottery_shares,
        pool.deposit_shares,
        pool.sponsor_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(31_000_000u128),
        3,
    );

    let awarded_prize = total_prize * Decimal256::percent(50);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            rand_round: 20170,
            sequence: WINNING_SEQUENCE.to_string(),
            awarded: true,
            timestamp: exec_height,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 0, 0, 0, 3],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_0, 0u64).unwrap();
    assert_eq!(prizes.matches, [0, 0, 0, 0, 0, 1]);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero());

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

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
fn execute_prize_pagination() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        MOCK_CONTRACT_ADDR,
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let addresses_count = 480u64;
    let addresses_range = 0..addresses_count;
    let addresses = addresses_range
        .map(|c| format!("addr{:0>4}", c))
        .collect::<Vec<String>>();
    // println!("addresses: {:?}", addresses);

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    for (index, address) in addresses.iter().enumerate() {
        // Users buys winning ticket
        let msg = ExecuteMsg::Deposit {
            combinations: vec![format!("be{:0>3}", 100 + index)],
        };
        let info = mock_info(
            address.as_str(),
            &[Coin {
                denom: "uusd".to_string(),
                amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
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
fn claim_rewards_one_depositor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
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
        combinations: vec![String::from("13579"), String::from("34567")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
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

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "addr0000".to_string(),
                amount: Uint128::from(100u128),
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
        Decimal256::percent(10000u64) / (Decimal256::percent(TICKET_PRICE * 2u64))
    );
}

#[test]
fn claim_rewards_multiple_depositors() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
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
        combinations: vec![String::from("13579"), String::from("34567")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );

    let mut env = mock_env();

    let _res = execute(deps.as_mut(), env.clone(), info, msg);

    // USER 1 Deposits another 20_000_000 uusd
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from(WINNING_SEQUENCE), String::from("11111")],
    };
    let info = mock_info(
        "addr1111",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );
    let _res = execute(deps.as_mut(), env.clone(), info, msg);

    let info = mock_info("addr0000", &[]);

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
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "addr0000".to_string(),
                amount: Uint128::from(50u128),
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
    assert_eq!(
        res.reward_index,
        Decimal256::percent(10000u64) / (Decimal256::percent(TICKET_PRICE * 4u64))
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
    assert_eq!(res.pending_rewards, Decimal256::percent(5000u64));
    assert_eq!(res.reward_index, state.global_reward_index);

    //TODO: Add a subsequent deposit at a later env.block.height and test again
}

#[test]
fn claim_rewards_depositor_and_sponsor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
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

    // USER 0 Deposits 20_000_000 uusd -----------------------------
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("34567")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );

    let mut env = mock_env();

    let _res = execute(deps.as_mut(), env.clone(), info, msg);

    // Sponsor deposits 20_000_000 uusd ------------------------------
    let msg = ExecuteMsg::Sponsor { award: Some(false) };

    let info = mock_info(
        "addr1111",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE * 2u64) * Uint256::one()).into(),
        }],
    );
    let _res = execute(deps.as_mut(), env.clone(), info, msg);

    let info = mock_info("addr0000", &[]);

    // Move forward 100 blocks ------------------------------------
    env.block.height += 100;

    // query the state --------------------------------------------
    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();
    println!("Global reward index: {:?}", state.global_reward_index);
    println!("Emission rate {:?}", state.glow_emission_rate);
    println!("Last reward updated {:?}", state.last_reward_updated);
    println!("Current height {:?}", env.block.height);

    // claim rewards for user 1
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    println!("{:?}", res.attributes);

    // assert that res has a message to send 50 GLOW (half of the total emission of 100)
    // from the distributor to addr0000
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "addr0000".to_string(),
                amount: Uint128::from(50u128),
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
    // the reward index of the USER should equal the global reward index
    // which is the reward accrued (100) divided by the total deposited value (TICKET_PRICE * 4)
    assert_eq!(
        res.reward_index,
        Decimal256::percent(10000u64) / (Decimal256::percent(TICKET_PRICE * 4u64))
    );

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
    // assert that the user has 50 GLOW pending rewards
    assert_eq!(res.pending_rewards, Decimal256::percent(5000u64));

    // assert that the user reward index equals the global_reward_index
    assert_eq!(res.reward_index, state.global_reward_index);

    // Move forward 100 blocks ------------------------------------
    env.block.height += 100;

    // query the state --------------------------------------------
    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();

    // claim rewards for user 1
    let msg = ExecuteMsg::ClaimRewards {};
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    println!("{:?}", res.attributes);

    // assert that res has a message to send 50 GLOW (half of the total emission of 100)
    // from the distributor to addr0000
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: DISTRIBUTOR_ADDR.to_string(),
            funds: vec![],
            msg: to_binary(&FaucetExecuteMsg::Spend {
                recipient: "addr0000".to_string(),
                amount: Uint128::from(50u128),
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
    // which is the reward accrued (100) divided by the total deposited value (TICKET_PRICE * 4)
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
    // assert that the user has 50 GLOW pending rewards
    assert_eq!(res.pending_rewards, Decimal256::percent(10000u64));

    // assert that the user reward index equals the global_reward_index
    assert_eq!(res.reward_index, state.global_reward_index);
}

#[test]
fn execute_epoch_operations() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    deps.querier.with_tax(
        Decimal::percent(1),
        &[(&"uusd".to_string(), &Uint128::from(1000000u128))],
    );

    let info = mock_info("addr0000", &[]);
    let mut env = mock_env();

    let mut state = STATE.load(deps.as_mut().storage).unwrap();
    state.total_reserve = Decimal256::percent(50000);
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
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
            to_address: GOV_ADDR.to_string(),
            amount: vec![Coin {
                denom: DENOM.to_string(),
                amount: Uint128::from(496u128), // 1% tax
            }],
        }))]
    );

    let state = query_state(deps.as_ref(), env.clone(), None).unwrap();

    // Glow Emission rate must be 1 as hard-coded in mock querier
    assert_eq!(
        state,
        StateResponse {
            total_tickets: Uint256::zero(),
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            last_reward_updated: 12445,
            global_reward_index: Decimal256::zero(),
            next_lottery_time: Expiration::AtTime(Timestamp::from_seconds(FIRST_LOTTO_TIME)),
            next_lottery_exec_time: Expiration::Never {},
            glow_emission_rate: Decimal256::one(),
            next_epoch: WEEK.after(&env.block)
        }
    );

    // fails, next epoch time not expired
    let msg = ExecuteMsg::ExecuteEpochOps {};
    let res = execute(deps.as_mut(), env, info, msg);
    match res {
        Err(ContractError::InvalidEpochExecution {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

fn calculate_total_prize(
    deps: Deps,
    lottery_shares: Decimal256,
    deposit_shares: Decimal256,
    sponsor_shares: Decimal256,
    initial_balance: Decimal256,
    aust_balance: Uint256,
    total_tickets: u64,
) -> Decimal256 {
    let aust_lottery_balance = aust_balance.multiply_ratio(
        (lottery_shares + sponsor_shares) * Uint256::one(),
        (deposit_shares + lottery_shares + sponsor_shares) * Uint256::one(),
    );

    let lottery_deposits =
        Decimal256::from_uint256(aust_lottery_balance) * Decimal256::permille(RATE);
    let lottery_yield = lottery_deposits
        - (Decimal256::percent(TICKET_PRICE * total_tickets)) * Decimal256::percent(SPLIT_FACTOR);

    let net_yield = Decimal256::from_uint256(Uint256::from(
        deduct_tax(deps, coin((lottery_yield * Uint256::one()).into(), "uusd"))
            .unwrap()
            .amount,
    ));

    initial_balance + net_yield
}

// This is all without taxes lmao

#[test]
fn small_withdraw() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // get env
    let env = mock_env();

    // mock instantiate the contracts
    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    // User deposits and buys one ticket -------------------
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("23456")],
    };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Add the funds to the contract address -------------------

    // Add an extra one to account for the rounding error
    // How do go about this properly?
    // The extra one is not gonna be there in production
    let minted_shares = Decimal256::percent(TICKET_PRICE * 1u64).div(Decimal256::permille(RATE));

    let minted_aust_value = minted_shares * Decimal256::permille(RATE);

    // you send ust
    // gets converted aUst but because of rounding
    // deposit * RATE / RATE != deposit (off by one)

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &(minted_shares * Uint256::one()).into(),
        )],
    )]);

    // Verify that deposit worked properly ------------------

    // Check that the depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_validate("addr0001").unwrap()
        ),
        DepositorInfo {
            deposit_amount: minted_aust_value,
            shares: Decimal256::percent(TICKET_PRICE * 1u64) / Decimal256::permille(RATE),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("23456")],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        query_state(deps.as_ref(), mock_env(), None).unwrap(),
        StateResponse {
            total_tickets: Uint256::from(1u64),
            total_reserve: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&mock_env().block),
            next_lottery_exec_time: Expiration::Never {},
            next_epoch: (WEEK + HOUR).unwrap().after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        query_pool(deps.as_ref()).unwrap(),
        PoolResponse {
            total_deposits: minted_aust_value,
            lottery_deposits: minted_aust_value * Decimal256::percent(SPLIT_FACTOR),
            total_sponsor_amount: Decimal256::zero(),
            lottery_shares: minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            deposit_shares: minted_shares - minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            sponsor_shares: Decimal256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: String::from("uusd"),
                amount: (Decimal256::percent(TICKET_PRICE * 1u64) * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "deposit"),
            attr("depositor", "addr0001"),
            attr("recipient", "addr0001"),
            attr(
                "deposit_amount",
                Decimal256::percent(TICKET_PRICE * 1u64).to_string()
            ),
            attr("tickets", 1u64.to_string()),
            attr("shares_minted", minted_shares.to_string()),
        ]
    );

    // Check that things are looking good for withdrawal

    let contract_a_balance = query_token_balance(
        deps.as_ref(),
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    let contract_ust_balance = contract_a_balance * Decimal256::permille(RATE);

    let pool = query_pool(deps.as_ref()).unwrap();

    println!(
        "{}, {}, {}, {}",
        contract_ust_balance.to_string(),
        contract_a_balance.to_string(),
        pool.total_deposits.to_string(),
        pool.total_sponsor_amount.to_string()
    );

    // Address withdraws a small amount of money ----------------
    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Withdraw {
        amount: Some(10u128.into()),
        instant: None,
    };
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Verify that the withdrawal was handled properly
    // let depositor =
    //     read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap());

    // let depositor_ratio = Decimal256::one();

    // let aust_amount = depositor_ratio * Decimal256::from_uint256(contract_a_balance);

    // let pooled_deposits = aust_amount * Decimal256::permille(RATE);

    // let withdraw_ratio = Decimal256::from_uint256(10u128) / pooled_deposits;

    // let aust_to_redeem = aust_amount * withdraw_ratio;
    // let redeemed_amount = pooled_deposits * withdraw_ratio;
    // let withdrawn_deposits = depositor.deposit_amount * withdraw_ratio;

    // println!("Iteration");
    // println!(
    //     "{:?}, {}",
    //     depositor.deposit_amount.to_string(),
    //     ((Decimal256::percent(TICKET_PRICE)) - withdrawn_amount).to_string()
    // );
    // println!(
    //     "{}, {}. {}, {}",
    //     pooled_deposits.to_string(),
    //     depositor.deposit_amount.to_string(),
    //     redeemed_amount.to_string(),
    //     withdrawn_deposits.to_string()
    // );
    // // println!("{}", withdraw_ratio);
    // // println!("{}", withdraw_ratio * depositor.deposit_amount);
    // println!(
    //     "{}, {}, {}",
    //     contract_a_balance.to_string(),
    //     (aust_to_redeem * Uint256::one()).to_string(),
    //     withdraw_ratio
    // );

    // withdrawn_amount += withdrawn_deposits;

    // deps.querier.with_token_balances(&[(
    //     &A_UST.to_string(),
    //     &[(
    //         &MOCK_CONTRACT_ADDR.to_string(),
    //         &(contract_a_balance - aust_to_redeem * Uint256::one()).into(),
    //     )],
    // )]);

    // deps.querier.update_balance(
    //     MOCK_CONTRACT_ADDR.to_string(),
    //     vec![Coin {
    //         denom: "uusd".to_string(),
    //         amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT) + Uint128::from(15u128 * 10u128),
    //     }],
    // );

    // // Advance one week in time -----------------
    // println!("Block time 1: {}", env.block.time);
    // if let Duration::Time(time) = WEEK {
    //     env.block.time = env.block.time.plus_seconds(time * 2);
    // }
    // println!("Block time 2: {}", env.block.time);

    // // Claim amount is already unbonded, so claim execution should work
    // let msg = ExecuteMsg::Claim {};
    // let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // // let shares = Uint256::one()
    // //     + (Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE)) * Uint256::one();

    // let shares = query_token_balance(
    //     deps.as_ref(),
    //     Addr::unchecked(A_UST),
    //     Addr::unchecked(MOCK_CONTRACT_ADDR),
    // )
    // .unwrap();

    // // Check depositor info was updated correctly

    // assert_eq!(
    //     read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap()),
    //     DepositorInfo {
    //         // deposit amount minus the amount withdrawn
    //         deposit_amount: (Decimal256::percent(TICKET_PRICE)) - withdrawn_amount,
    //         // shares minus the amount withdrawn divided by rate,
    //         // but it's not that easy because of rounding errors
    //         shares: Decimal256::from_uint256(shares),
    //         reward_index: Decimal256::zero(),
    //         pending_rewards: Decimal256::zero(),
    //         tickets: vec![],
    //         // no remaining unbonding info
    //         unbonding_info: vec![]
    //     }
    // );

    // assert_eq!(
    //     res.messages,
    //     vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
    //         to_address: "addr0001".to_string(),
    //         amount: vec![Coin {
    //             denom: String::from("uusd"),
    //             amount: Uint128::from(10_000_000u64)
    //                 - Uint128::from(Decimal256::percent(TICKET_PRICE) * Uint256::one())
    //                 - Uint128::from(10u128 * 15u128),
    //         }],
    //     }))]
    // );

    // assert_eq!(
    //     res.attributes,
    //     vec![
    //         attr("action", "claim_unbonded"),
    //         attr("depositor", "addr0001"),
    //         attr(
    //             "redeemed_amount",
    //             (Uint256::from(10_000_000u64)
    //                 - (Decimal256::percent(TICKET_PRICE) * Uint256::one())
    //                 - Uint256::from(10u128 * 15u128))
    //             .to_string()
    //         ),
    //     ]
    // );
}

// #[test]
// fn max_unbonding_claim() {
//     // Initialize contract
//     let mut deps = mock_dependencies(&[]);

//     // Mock aUST-UST exchange rate
//     deps.querier.with_exchange_rate(Decimal256::permille(RATE));

//     // deps.querier.with_tax(
//     //     Decimal::percent(0),
//     //     &[(&"uusd".to_string(), &Uint128::from(1_000_000u128))],
//     // );

//     let mut env = mock_env();

//     mock_instantiate(deps.as_mut());
//     mock_register_contracts(deps.as_mut());

//     // User deposits and buys one ticket -------------------
//     let info = mock_info(
//         "addr0001",
//         &[Coin {
//             denom: "uusd".to_string(),
//             amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
//         }],
//     );
//     let msg = ExecuteMsg::Deposit {
//         combinations: vec![String::from("23456")],
//     };
//     let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

//     // Add the funds to the contract address -------------------
//     let shares = (Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE)) * Uint256::one();

//     deps.querier.with_token_balances(&[(
//         &A_UST.to_string(),
//         &[(&MOCK_CONTRACT_ADDR.to_string(), &shares.into())],
//     )]);

//     // Address withdraws a small amount of money 15 times -----------------
//     let info = mock_info("addr0001", &[]);
//     let msg = ExecuteMsg::Withdraw {
//         amount: Some(10u128.into()),
//         instant: None,
//     };

//     // withdraw amount isn't perfect because of fixed points
//     let mut withdrawn_amount = Decimal256::from_uint256(0u128);

//     for _ in 0..15 {
//         let depositor =
//             read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap());

//         let depositor_ratio = Decimal256::one();

//         let contract_a_balance = query_token_balance(
//             deps.as_ref(),
//             Addr::unchecked(A_UST),
//             Addr::unchecked(MOCK_CONTRACT_ADDR),
//         )
//         .unwrap();

//         let aust_amount = depositor_ratio * Decimal256::from_uint256(contract_a_balance);

//         let pooled_deposits = aust_amount * Decimal256::permille(RATE);

//         let withdraw_ratio = Decimal256::from_uint256(10u128) / pooled_deposits;

//         let aust_to_redeem = aust_amount * withdraw_ratio;
//         let redeemed_amount = pooled_deposits * withdraw_ratio;
//         let withdrawn_deposits = depositor.deposit_amount * withdraw_ratio;

//         println!("Iteration");
//         println!(
//             "{:?}, {}",
//             depositor.deposit_amount.to_string(),
//             ((Decimal256::percent(TICKET_PRICE)) - withdrawn_amount).to_string()
//         );
//         println!(
//             "{}, {}. {}, {}",
//             pooled_deposits.to_string(),
//             depositor.deposit_amount.to_string(),
//             redeemed_amount.to_string(),
//             withdrawn_deposits.to_string()
//         );
//         // println!("{}", withdraw_ratio);
//         // println!("{}", withdraw_ratio * depositor.deposit_amount);
//         println!(
//             "{}, {}, {}",
//             contract_a_balance.to_string(),
//             (aust_to_redeem * Uint256::one()).to_string(),
//             withdraw_ratio
//         );

//         let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

//         withdrawn_amount += withdrawn_deposits;

//         deps.querier.with_token_balances(&[(
//             &A_UST.to_string(),
//             &[(
//                 &MOCK_CONTRACT_ADDR.to_string(),
//                 &(contract_a_balance - aust_to_redeem * Uint256::one()).into(),
//             )],
//         )]);
//     }

//     deps.querier.update_balance(
//         MOCK_CONTRACT_ADDR.to_string(),
//         vec![Coin {
//             denom: "uusd".to_string(),
//             amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT) + Uint128::from(15u128 * 10u128),
//         }],
//     );

//     // Address tries to make another claim but fails
//     let res = execute(deps.as_mut(), env.clone(), info.clone(), msg);
//     match res {
//         Err(ContractError::MaxUnbondingClaims {}) => {}
//         _ => panic!("DO NOT ENTER HERE"),
//     }

//     // Advance one week in time -----------------
//     println!("Block time 1: {}", env.block.time);
//     if let Duration::Time(time) = WEEK {
//         env.block.time = env.block.time.plus_seconds(time * 2);
//     }
//     println!("Block time 2: {}", env.block.time);

//     // Claim amount is already unbonded, so claim execution should work
//     let msg = ExecuteMsg::Claim {};
//     let res = execute(deps.as_mut(), env, info, msg).unwrap();

//     // let shares = Uint256::one()
//     //     + (Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE)) * Uint256::one();

//     let shares = query_token_balance(
//         deps.as_ref(),
//         Addr::unchecked(A_UST),
//         Addr::unchecked(MOCK_CONTRACT_ADDR),
//     )
//     .unwrap();

//     // Check depositor info was updated correctly

//     assert_eq!(
//         read_depositor_info(&deps.storage, &deps.api.addr_validate("addr0001").unwrap()),
//         DepositorInfo {
//             // deposit amount minus the amount withdrawn
//             deposit_amount: (Decimal256::percent(TICKET_PRICE)) - withdrawn_amount,
//             // shares minus the amount withdrawn divided by rate,
//             // but it's not that easy because of rounding errors
//             shares: Decimal256::from_uint256(shares),
//             reward_index: Decimal256::zero(),
//             pending_rewards: Decimal256::zero(),
//             tickets: vec![],
//             // no remaining unbonding info
//             unbonding_info: vec![]
//         }
//     );

//     assert_eq!(
//         res.messages,
//         vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
//             to_address: "addr0001".to_string(),
//             amount: vec![Coin {
//                 denom: String::from("uusd"),
//                 amount: Uint128::from(10_000_000u64)
//                     - Uint128::from(Decimal256::percent(TICKET_PRICE) * Uint256::one())
//                     - Uint128::from(10u128 * 15u128),
//             }],
//         }))]
//     );

//     assert_eq!(
//         res.attributes,
//         vec![
//             attr("action", "claim_unbonded"),
//             attr("depositor", "addr0001"),
//             attr(
//                 "redeemed_amount",
//                 (Uint256::from(10_000_000u64)
//                     - (Decimal256::percent(TICKET_PRICE) * Uint256::one())
//                     - Uint256::from(10u128 * 15u128))
//                 .to_string()
//             ),
//         ]
//     );
// }

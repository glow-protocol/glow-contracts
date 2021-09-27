use crate::contract::{
    execute, instantiate, query, query_config, query_state, query_ticket_info,
    INITIAL_DEPOSIT_AMOUNT,
};
use crate::mock_querier::mock_dependencies;
use crate::state::{
    query_prizes, read_depositor_info, read_lottery_info, Config, DepositorInfo, LotteryInfo,
    State, STATE,
};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::testing::{mock_env, mock_info, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    attr, from_binary, to_binary, Addr, Api, BankMsg, Coin, CosmosMsg, Decimal, DepsMut, Env,
    Response, SubMsg, Timestamp, Uint128, WasmMsg,
};
use cw20::Cw20ExecuteMsg;
use glow_protocol::distributor::ExecuteMsg as FaucetExecuteMsg;
use glow_protocol::lotto::{
    Claim, ConfigResponse, DepositorInfoResponse, ExecuteMsg, InstantiateMsg, QueryMsg,
    StateResponse,
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

const TICKET_PRICE: u64 = 1_000_000_000; // 10_000_000 as %
const SPLIT_FACTOR: u64 = 75; // as a %
const INSTANT_WITHDRAWAL_FEE: u64 = 10; // as a %
const RESERVE_FACTOR: u64 = 5; // as a %
const MAX_HOLDERS: u8 = 10;
const RATE: u64 = 1023; // as a permille
const WEEK_TIME: u64 = 604800; // in seconds
const HOUR_TIME: u64 = 3600; // in seconds

pub(crate) fn instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        owner: TEST_CREATOR.to_string(),
        stable_denom: DENOM.to_string(),
        anchor_contract: ANCHOR.to_string(),
        aterra_contract: A_UST.to_string(),
        lottery_interval: WEEK_TIME,
        block_time: HOUR_TIME,
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
    }
}

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

    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
    let config = query_config(deps.as_ref()).unwrap();
    assert_eq!(config.gov_contract, GOV_ADDR.to_string());
    assert_eq!(config.distributor_contract, DISTRIBUTOR_ADDR.to_string());

    let state = query_state(deps.as_ref(), env, None).unwrap();
    assert_eq!(
        state,
        StateResponse {
            total_tickets: Uint256::zero(),
            total_reserve: Decimal256::zero(),
            total_deposits: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            shares_supply: Decimal256::zero(),
            deposit_shares: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&mock_env().block),
            last_reward_updated: 12345, // hard-coded
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    // Cannot register contracts again //TODO
    let _msg = ExecuteMsg::RegisterContracts {
        gov_contract: GOV_ADDR.to_string(),
        distributor_contract: DISTRIBUTOR_ADDR.to_string(),
    };
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
        lottery_interval: None,
        block_time: None,
        ticket_price: None,
        prize_distribution: None,
        reserve_factor: None,
        split_factor: None,
        unbonding_period: None,
    };
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // Check owner has changed
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();

    assert_eq!("owner1".to_string(), config_response.owner);

    // update lottery interval to 30 minutes
    let info = mock_info("owner1", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        lottery_interval: Some(1800),
        block_time: None,
        ticket_price: None,
        prize_distribution: None,
        reserve_factor: None,
        split_factor: None,
        unbonding_period: None,
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
        lottery_interval: None,
        block_time: None,
        ticket_price: None,
        prize_distribution: None,
        reserve_factor: Some(Decimal256::percent(1)),
        split_factor: None,
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
        owner: None,
        lottery_interval: Some(1800),
        block_time: None,
        ticket_price: None,
        prize_distribution: None,
        reserve_factor: None,
        split_factor: None,
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

    // Must deposit stable_denom coins
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
        Err(ContractError::InvalidDepositAmount {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - more number of digits
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

    // Invalid ticket sequence - less number of digits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("3457")],
    };

    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - only numbers allowed
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("135w9"), String::from("34567")],
    };
    let res = execute(deps.as_mut(), mock_env(), info.clone(), msg);
    match res {
        Err(ContractError::InvalidSequence {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Correct deposit - buys two tickets
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("34567")],
    };

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check address of sender was stored correctly in both sequence buckets
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

    // Check depositor info was updated correctly
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
            total_deposits: Decimal256::percent(TICKET_PRICE * 2u64),
            lottery_deposits: Decimal256::percent(TICKET_PRICE * 2u64)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: minted_shares,
            deposit_shares: minted_shares - minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
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
            attr("action", "batch_deposit"),
            attr("depositor", "addr0000"),
            attr(
                "deposit_amount",
                Decimal256::percent(TICKET_PRICE * 2u64).to_string()
            ),
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
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    // Ticket is already owner by 10 holders
    let addresses_count = 10u64;
    let addresses_range = 0..addresses_count;
    let addresses = addresses_range
        .map(|c| format!("addr{:0>4}", c))
        .collect::<Vec<String>>();

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    for (index, address) in addresses.iter().enumerate() {
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

    let wrong_amount = Decimal256::percent(TICKET_PRICE * 4);

    // correct base denom, deposit different to TICKET_PRICE
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (wrong_amount * Uint256::one()).into(),
        }],
    );

    let res = execute(deps.as_mut(), mock_env(), info, msg);

    //TODO: Revise this. Clippy complains as variables not being used
    let _amount_required = TICKET_PRICE * 2u64;
    match res {
        Err(ContractError::InsufficientGiftDepositAmount(_amount_required)) => {}
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

    /*
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT + TICKET_PRICE),
        }],
    );
     */

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
            total_deposits: Decimal256::percent(TICKET_PRICE * 2u64),
            lottery_deposits: Decimal256::percent(TICKET_PRICE * 2u64)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: minted_shares,
            deposit_shares: minted_shares - minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
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
            attr("action", "gift_tickets"),
            attr("gifter", "addr0000"),
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

    // TODO: cover more cases eg. sequential buys and repeated ticket in same buy
    // TODO: deposit fails when current lottery deposit time is expired
}

// TODO: write sponsor testcases

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

    let dep1 = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0001").unwrap(),
    );

    println!("dep1: {:x?}", dep1);

    let stor1 = query_state(deps.as_ref(), mock_env(), None).unwrap();

    println!("stor1: {:x?}", stor1);

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

    let dep1 = read_depositor_info(
        deps.as_ref().storage,
        &deps.api.addr_validate("addr0001").unwrap(),
    );

    println!("dep2: {:x?}", dep1);

    let stor1 = query_state(deps.as_ref(), mock_env(), None).unwrap();

    println!("stor2: {:x?}", stor1);

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

    // TODO: use below redeem amount instead of hardcoded unbonding info

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
            total_deposits: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            shares_supply: Decimal256::zero(),
            deposit_shares: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
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

    // TODO: use below redeem amount instead of hardcoded unbonding info

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
            total_deposits: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            shares_supply: Decimal256::zero(),
            deposit_shares: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&mock_env().block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
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
    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    println!("shares: {}", shares);
    println!("pooled_deposits: {}", shares * Decimal256::permille(RATE));
    println!("total deposits: {}", state.total_deposits);

    // Correct withdraw, user has 1 ticket to be withdrawn
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Claim amount that you don't have, should fail
    let info = mock_info("addr0002", &[]);
    let msg = ExecuteMsg::Claim { lottery: None };

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::InsufficientClaimableFunds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    //TODO: test insufficient funds in contract

    // Claim amount that you have, but still in unbonding state, should fail
    let info = mock_info("addr0001", &[]);
    let msg = ExecuteMsg::Claim { lottery: None };

    let mut env = mock_env();

    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg);
    match res {
        Err(ContractError::InsufficientClaimableFunds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let msg = ExecuteMsg::Claim { lottery: None };

    println!("Block time 1: {}", env.block.time);

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time = env.block.time.plus_seconds(time * 2);
    }
    println!("Block time 2: {}", env.block.time);
    // TODO: change also the exchange rate here

    // TODO: add case asking for more amount that the one we have (which is non-zero)
    // TODO: add case asking for an amount (not None) that we do have
    // TODO: add case where contract balances are not enough to fulfill claim

    // TODO: this update is not needed (??)
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
            attr("action", "claim"),
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
        combinations: vec![String::from("00000")],
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
            tickets: vec![String::from("00000")],
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
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Check lottery info was updated correctly

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    let total_prize = calculate_total_prize(
        state.shares_supply,
        state.deposit_shares,
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
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 0, 0, 0, 1],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(prizes, [0, 0, 0, 0, 0, 1]);

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero(),);

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

    let info = mock_info("addr0000", &[]);
    let msg = ExecuteMsg::Claim {
        lottery: Some(0u64),
    };

    // Claim lottery should work, even if there are no unbonded claims
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    let prize = calculate_winner_prize(
        awarded_prize,
        prizes,
        lottery.number_winners,
        query_config(deps.as_ref()).unwrap().prize_distribution,
    );

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
            attr("action", "claim"),
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
        Err(ContractError::InvalidLotteryExecution {}) => {}
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

    let to_redeem = Uint256::from(10_000_000u128);

    // TODO: add test case with deposit_shares != 0

    let res = execute(deps.as_mut(), env.clone(), info, msg);

    // Lottery cannot be run with 0 tickets participating
    match res {
        Err(ContractError::InvalidLotteryExecution {}) => {}
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
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info.clone(),
        lottery_msg.clone(),
    )
    .unwrap();

    let current_balance = Uint256::from(100_000_000u128);

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_lottery"),
            attr("redeemed_amount", "0"),
        ]
    );

    // Execute prize
    let execute_prize_msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info.clone(),
        execute_prize_msg.clone(),
    )
    .unwrap();

    // Directly check next_lottery_time has been set up for next week
    let next_lottery_time = query_state(deps.as_ref(), mock_env(), None)
        .unwrap()
        .next_lottery_time;

    assert_eq!(
        next_lottery_time,
        Expiration::AtTime(env.block.time).add(WEEK).unwrap()
    );

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
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info.clone(),
        lottery_msg.clone(),
    )
    .unwrap();

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

    // Execute prize
    let res = execute(deps.as_mut(), env.clone(), info.clone(), execute_prize_msg).unwrap();

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
        Err(ContractError::InvalidLotteryExecution {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    // Run lottery, no winners - should run correctly
    let res = execute(deps.as_mut(), env.clone(), info, msg);
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
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Check lottery info was updated correctly
    let awarded_prize = Decimal256::zero();

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            number_winners: [0; 6],
            page: "".to_string()
        }
    );

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero());

    // total prize = balance - old_balance - lottery_deposits
    let total_prize = Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT));

    // TODO: Calculate and avoid hard-coding
    assert_eq!(
        state.award_available,
        Decimal256::from_str("107844998.977").unwrap()
    );

    // reinvest lottery deposits
    let lottery_deposits = Decimal256::percent(TICKET_PRICE) * Decimal256::percent(SPLIT_FACTOR);

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
        combinations: vec![String::from("00000")],
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
            tickets: vec![String::from("00000")],
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
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Check lottery info was updated correctly

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    let total_prize = calculate_total_prize(
        state.shares_supply,
        state.deposit_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(20_000_000u128),
        1,
    );

    let awarded_prize = total_prize * Decimal256::percent(50);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 0, 0, 0, 1],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(prizes, [0, 0, 0, 0, 0, 1]);

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
        combinations: vec![String::from("00000")],
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
            tickets: vec![String::from("00000")],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 2 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("00100")],
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
            tickets: vec![String::from("00100")],
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
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Check lottery info was updated correctly

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();

    let total_prize = calculate_total_prize(
        state.shares_supply,
        state.deposit_shares,
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
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 1, 0, 0, 1],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw_0, 0u64).unwrap();
    assert_eq!(prizes, [0, 0, 0, 0, 0, 1]);

    let prizes = query_prizes(deps.as_ref(), &address_raw_1, 0u64).unwrap();
    assert_eq!(prizes, [0, 0, 1, 0, 0, 0]);

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

    // Users buys winning ticket - 5 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("00001")],
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
            tickets: vec![String::from("00001")],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 5 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("00001")],
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
            tickets: vec![String::from("00001")],
            unbonding_info: vec![]
        }
    );

    // Run lottery, one winner (5 hits), one winner (5 hits) - should run correctly
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
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    // total prize
    let total_prize = calculate_total_prize(
        state.shares_supply,
        state.deposit_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(30_000_000u128),
        2,
    );

    let awarded_prize = total_prize * Decimal256::percent(30);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
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
        combinations: vec![String::from("00000")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    // Users buys winning ticket - 5 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("00001")],
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("00002")],
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("00003")],
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("01003")],
    };
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

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
                String::from("00000"),
                String::from("00001"),
                String::from("00002"),
                String::from("00003"),
                String::from("01003")
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
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    // total prize
    let total_prize = calculate_total_prize(
        state.shares_supply,
        state.deposit_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(55_000_000u128),
        5,
    );

    let awarded_prize = total_prize * Decimal256::percent(50 + 30);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 0, 0, 3, 1],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_raw, 0u64).unwrap();
    assert_eq!(prizes, [0, 0, 0, 0, 3, 1]);

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
        combinations: vec![String::from("00000")],
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

    let _res = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap();

    let address_0 = deps.api.addr_validate("addr0000").unwrap();
    let address_1 = deps.api.addr_validate("addr1111").unwrap();
    let address_2 = deps.api.addr_validate("addr2222").unwrap();

    let ticket = query_ticket_info(deps.as_ref(), String::from("00000")).unwrap();

    assert_eq!(
        ticket.holders,
        vec![address_0.clone(), address_1.clone(), address_2.clone()]
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
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let state = query_state(deps.as_ref(), mock_env(), None).unwrap();
    // total prize
    let total_prize = calculate_total_prize(
        state.shares_supply,
        state.deposit_shares,
        Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT)),
        Uint256::from(31_000_000u128),
        3,
    );

    let awarded_prize = total_prize * Decimal256::percent(50);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            number_winners: [0, 0, 0, 0, 0, 3],
            page: "".to_string()
        }
    );

    let prizes = query_prizes(deps.as_ref(), &address_0, 0u64).unwrap();
    assert_eq!(prizes, [0, 0, 0, 0, 0, 1]);

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

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    for (index, address) in addresses.iter().enumerate() {
        // Users buys winning ticket
        let msg = ExecuteMsg::Deposit {
            combinations: vec![format!("{:0>5}", index)],
            // combinations: vec![String::from("00000")],
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

    let msg = ExecuteMsg::ExecutePrize { limit: None };
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);

    println!("lottery_info: {:x?}", lottery_info);
    assert!(!lottery_info.awarded);

    // Second pagination round
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);

    println!("lottery_info: {:x?}", lottery_info);

    // Third pagination round
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly
    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);

    println!("lottery_info: {:x?}", lottery_info);

    // Fourth pagination round
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);

    println!("lottery_info: {:x?}", lottery_info);

    // Fifth pagination round
    let _res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);

    println!("lottery_info: {:x?}", lottery_info);

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
    STATE.save(deps.as_mut().storage, &state);

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
    STATE.save(deps.as_mut().storage, &state);

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
        combinations: vec![String::from("00000"), String::from("11111")],
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
    assert_eq!(res.pending_rewards, Decimal256::zero());
    assert_eq!(res.reward_index, Decimal256::zero());

    //TODO: Add a subsequent deposit at a later env.block.height and test again
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
    STATE.save(deps.as_mut().storage, &state);

    /*
    STATE.update(deps.as_mut().storage,  |mut state| {
        state.total_reserve = Decimal256::percent(50000);
        Ok(state)
    }).unwrap();
     */

    env.block.height += 100;

    let msg = ExecuteMsg::ExecuteEpochOps {};
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

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
            total_deposits: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            shares_supply: Decimal256::zero(),
            deposit_shares: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            last_reward_updated: 12445,
            global_reward_index: Decimal256::zero(),
            next_lottery_time: WEEK.after(&env.block),
            glow_emission_rate: Decimal256::one()
        }
    );
}

// TODO: Refactor tests
// TODO: Test prize_strategy functions combinations (without wasm)

fn calculate_total_prize(
    shares_supply: Decimal256,
    deposit_shares: Decimal256,
    initial_balance: Decimal256,
    aust_balance: Uint256,
    total_tickets: u64,
) -> Decimal256 {
    let aust_lottery_balance = aust_balance.multiply_ratio(
        (shares_supply - deposit_shares) * Uint256::one(),
        shares_supply * Uint256::one(),
    );

    let lottery_deposits =
        Decimal256::from_uint256(aust_lottery_balance) * Decimal256::permille(RATE);
    let net_yield = lottery_deposits
        - (Decimal256::percent(TICKET_PRICE * total_tickets)) * Decimal256::percent(SPLIT_FACTOR);
    initial_balance + net_yield
}
// TODO: can use in contract as well
fn calculate_winner_prize(
    total_awarded: Decimal256,
    address_rank: [u32; 6],
    lottery_winners: [u32; 6],
    prize_dis: [Decimal256; 6],
) -> Uint128 {
    let mut to_send: Uint128 = Uint128::zero();
    for i in 2..6 {
        if lottery_winners[i] == 0 {
            continue;
        }
        let ranked_price: Uint256 = (total_awarded * prize_dis[i]) * Uint256::one();

        let amount: Uint128 = ranked_price
            .multiply_ratio(address_rank[i], lottery_winners[i])
            .into();

        to_send += amount;
    }
    to_send
}

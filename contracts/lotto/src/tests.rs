use crate::contract::{execute, instantiate, query, INITIAL_DEPOSIT_AMOUNT};
use crate::mock_querier::mock_dependencies;
use crate::state::{
    read_config, read_depositor_info, read_lottery_info, read_sequence_info, read_state,
    store_state, Config, DepositorInfo, LotteryInfo, State,
};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::testing::{mock_env, mock_info, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    attr, from_binary, to_binary, Api, BankMsg, CanonicalAddr, Coin, CosmosMsg, Decimal, DepsMut,
    Env, Response, SubMsg, Timestamp, Uint128, WasmMsg,
};
use cw20::Cw20ExecuteMsg;
use glow_protocol::distributor::ExecuteMsg as FaucetExecuteMsg;
use glow_protocol::lotto::{
    Claim, ConfigResponse, DepositorInfoResponse, ExecuteMsg, InstantiateMsg, QueryMsg,
};

use crate::error::ContractError;
use cw0::{Duration, Expiration, HOUR, WEEK};
use moneymarket::market::{Cw20HookMsg, ExecuteMsg as AnchorMsg};
use moneymarket::querier::deduct_tax; //TODO: import from glow_protocol package
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
        prize_distribution: vec![
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

#[allow(dead_code)] //TODO: use this fn
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

    let res = instantiate(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();
    assert_eq!(0, res.messages.len());

    let config: Config = read_config(deps.as_ref().storage).unwrap();

    assert_eq!(
        config,
        Config {
            owner: deps.api.addr_canonicalize(TEST_CREATOR).unwrap(),
            a_terra_contract: deps.api.addr_canonicalize(A_UST).unwrap(),
            gov_contract: CanonicalAddr::from(vec![]),
            distributor_contract: CanonicalAddr::from(vec![]),
            anchor_contract: deps.api.addr_canonicalize(ANCHOR).unwrap(),
            stable_denom: DENOM.to_string(),
            lottery_interval: WEEK,
            block_time: HOUR,
            ticket_price: Decimal256::percent(TICKET_PRICE),
            prize_distribution: vec![
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

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    let config: Config = read_config(deps.as_ref().storage).unwrap();
    assert_eq!(
        config.gov_contract,
        deps.api.addr_canonicalize(GOV_ADDR).unwrap()
    );
    assert_eq!(
        config.distributor_contract,
        deps.api.addr_canonicalize(DISTRIBUTOR_ADDR).unwrap()
    );

    let state: State = read_state(deps.as_ref().storage).unwrap();
    assert_eq!(
        state,
        State {
            total_tickets: Uint256::zero(),
            total_reserve: Decimal256::zero(),
            total_deposits: Decimal256::zero(),
            lottery_deposits: Decimal256::zero(),
            shares_supply: Decimal256::zero(),
            deposit_shares: Decimal256::zero(),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&mock_env().block),
            last_reward_updated: 0,
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

// TODO: deposit fails when current lottery deposit time is expired
// TODO: test buy only one ticket
// TODO: test buying LARGE amount of tickets
// TODO: test executing lottery with LARGE amount of winners

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

    /*
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT + TICKET_PRICE,
        }],
    );
     */

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check address of sender was stored correctly in both sequence buckets
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("13579")),
        vec![deps.api.addr_canonicalize("addr0000").unwrap()]
    );
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("34567")),
        vec![deps.api.addr_canonicalize("addr0000").unwrap()]
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_canonicalize("addr0000").unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE * 2u64),
            shares: Decimal256::percent(TICKET_PRICE * 2u64) / Decimal256::permille(RATE),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("13579"), String::from("34567")],
            unbonding_info: vec![]
        }
    );

    let minted_shares = Decimal256::percent(TICKET_PRICE * 2u64).div(Decimal256::permille(RATE));

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
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
            last_reward_updated: 12345, //TODO: hardcoded. why this value?
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

    // TODO: cover more cases eg. sequential buys and repeated ticket in same buy
    // TODO: deposit fails when current lottery deposit time is expired
    // TODO: correct base denom, deposit greater than tickets test case
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
        read_sequence_info(deps.as_ref().storage, &String::from("13579")),
        vec![deps.api.addr_canonicalize("addr1111").unwrap()]
    );
    assert_eq!(
        read_sequence_info(deps.as_ref().storage, &String::from("34567")),
        vec![deps.api.addr_canonicalize("addr1111").unwrap()]
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            deps.as_ref().storage,
            &deps.api.addr_canonicalize("addr1111").unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE * 2u64),
            shares: Decimal256::percent(TICKET_PRICE * 2u64) / Decimal256::permille(RATE),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("13579"), String::from("34567")],
            unbonding_info: vec![]
        }
    );

    let minted_shares = Decimal256::percent(TICKET_PRICE * 2u64).div(Decimal256::permille(RATE));

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
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
        &deps.api.addr_canonicalize("addr0001").unwrap(),
    );

    println!("dep1: {:x?}", dep1);

    let stor1 = read_state(&deps.storage).unwrap();

    println!("stor1: {:x?}", stor1);

    let shares = (Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE)) * Uint256::one();

    let info = mock_info("addr0001", &[]);

    let msg = ExecuteMsg::Withdraw { instant: None };

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
        &deps.api.addr_canonicalize("addr0001").unwrap(),
    );

    println!("dep2: {:x?}", dep1);

    let stor1 = read_state(&deps.storage).unwrap();

    println!("stor2: {:x?}", stor1);

    // Check address of sender was removed correctly in the sequence bucket
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("23456")),
        vec![]
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
            &deps.api.addr_canonicalize("addr0001").unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![Claim {
                amount: Decimal256::from_uint256(Uint256::from(9999999u128)),
                release_at: WEEK.after(&mock_env().block),
            }]
        }
    );

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
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
                Decimal256::from_str("9999999.933").unwrap().to_string()
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

    let shares = (Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE)) * Uint256::one();

    let info = mock_info("addr0001", &[]);

    let msg = ExecuteMsg::Withdraw {
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

    // Check address of sender was removed correctly in the sequence bucket
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("23456")),
        vec![]
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
            &deps.storage,
            &deps.api.addr_canonicalize("addr0001").unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
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
                    amount: Uint128::from(8999999u128)
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
                Decimal256::from_str("8999999.9397").unwrap().to_string()
            ),
            attr(
                "instant_withdrawal_fee",
                Decimal256::from_str("999999.9933").unwrap().to_string()
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
    let msg = ExecuteMsg::Withdraw { instant: None };

    let shares = (Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE)) * Uint256::one();

    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(&MOCK_CONTRACT_ADDR.to_string(), &shares.into())],
    )]);

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

    let dep = read_depositor_info(
        &deps.storage,
        &deps.api.addr_canonicalize("addr0001").unwrap(),
    );

    println!("DepositorInfo: {:x?}", dep);

    // Claim amount is already unbonded, so claim execution should work
    let res = execute(deps.as_mut(), env, info, msg).unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            &deps.storage,
            &deps.api.addr_canonicalize("addr0001").unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
            redeemable_amount: Uint128::zero(),
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
                amount: Uint128::from(9_999_999u64), //TODO: should be 10_000_000
            }],
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "claim"),
            attr("depositor", "addr0001"),
            attr("redeemed_amount", 9_999_999u64.to_string()),
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

    // It should not fail, but redeem message is not called
    // TODO: add test case
    /*
    let res = execute(&mut deps, env.clone(), msg.clone());

    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "There is no available funds to execute the lottery",)
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

     */

    // Add 10 aUST to our contract balance
    deps.querier.with_token_balances(&[(
        &A_UST.to_string(),
        &[(
            &MOCK_CONTRACT_ADDR.to_string(),
            &Uint128::from(10_000_000u128),
        )],
    )]);

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

    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Directly check next_lottery_time has been set up for next week
    let next_lottery_time = read_state(deps.as_ref().storage).unwrap().next_lottery_time;

    assert_eq!(
        next_lottery_time,
        Expiration::AtTime(env.block.time).add(WEEK).unwrap()
    );

    let current_balance = Uint256::from(100_000_000u128);

    assert_eq!(
        res.messages,
        vec![
            SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: A_UST.to_string(),
                funds: vec![],
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: ANCHOR.to_string(),
                    amount: to_redeem.into(),
                    msg: to_binary(&Cw20HookMsg::RedeemStable {}).unwrap(),
                })
                .unwrap(),
            })),
            SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: MOCK_CONTRACT_ADDR.to_string(),
                funds: vec![],
                msg: to_binary(&ExecuteMsg::_ExecutePrize {
                    balance: current_balance
                })
                .unwrap(),
            }))
        ]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_lottery"),
            attr("redeemed_amount", to_redeem),
        ]
    );
}

#[test]
fn execute_prize_no_tickets() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    let info = mock_info("addr0001", &[]);

    let balance = Uint256::from(INITIAL_DEPOSIT_AMOUNT);

    let msg = ExecuteMsg::_ExecutePrize { balance };
    let res = execute(deps.as_mut(), mock_env(), info, msg);

    match res {
        Err(ContractError::Unauthorized {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);
    let msg = ExecuteMsg::_ExecutePrize { balance };

    /* The contract does not have UST balance, should fail
    let res = execute(&mut deps, env.clone(), msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "There is no UST balance to fund the prize",)
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(150_000_000_000u128),
        }],
    );

     */

    // Run lottery, no winners - should run correctly
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check lottery info was updated correctly
    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: Decimal256::zero(),
            winners: vec![]
        }
    );

    let state = read_state(deps.as_ref().storage).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero()); // From the initialization of the contract
    assert_eq!(
        state.award_available,
        Decimal256::from_uint256(Uint256::from(100_000_000u128))
    );

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr("accrued_interest", Uint256::zero().to_string()),
            attr("total_awarded_prize", Decimal256::zero().to_string()),
            attr("reinvested_amount", Uint256::zero().to_string()),
        ]
    );
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

    let address_raw = deps.api.addr_canonicalize("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("11111")],
            unbonding_info: vec![]
        }
    );

    let balance = Uint256::from(INITIAL_DEPOSIT_AMOUNT);

    // Run lottery, one winner (5 hits) - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);
    let msg = ExecuteMsg::_ExecutePrize { balance };
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check lottery info was updated correctly
    let awarded_prize = Decimal256::zero();

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            winners: vec![]
        }
    );

    let state = read_state(deps.as_ref().storage).unwrap();
    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero());

    // total prize = balance - old_balance - lottery_deposits
    let total_prize = Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT));

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

    // reinvest lottery deposits
    let lottery_deposits = Decimal256::percent(TICKET_PRICE) * Decimal256::percent(SPLIT_FACTOR);

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: "uusd".to_string(),
                amount: (lottery_deposits * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr("accrued_interest", Uint128::zero().to_string()),
            attr("total_awarded_prize", awarded_prize.to_string()),
            attr(
                "reinvested_amount",
                (lottery_deposits * Uint256::one()).to_string()
            ),
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

    let address_raw = deps.api.addr_canonicalize("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00000")],
            unbonding_info: vec![]
        }
    );

    // Run lottery, one winner (5 hits) - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);
    let msg = ExecuteMsg::_ExecutePrize {
        balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
    };
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check lottery info was updated correctly

    // total prize = balance - lottery_deposits
    let total_prize = Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT));

    let awarded_prize = total_prize * Decimal256::percent(50);

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            winners: vec![(5, vec![address_raw.clone()])]
        }
    );

    let prize_assigned = read_depositor_info(deps.as_ref().storage, &address_raw).redeemable_amount;

    // prize assigned should be (140k - 7500) / 2

    let mock_prize = awarded_prize - (awarded_prize * Decimal256::percent(RESERVE_FACTOR));

    assert_eq!(prize_assigned, (mock_prize * Uint256::one()).into());

    let state = read_state(deps.as_ref().storage).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(
        state.total_reserve,
        awarded_prize * Decimal256::percent(RESERVE_FACTOR)
    );

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

    // reinvest lottery deposits
    let lottery_deposits = Decimal256::percent(TICKET_PRICE) * Decimal256::percent(SPLIT_FACTOR);

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: "uusd".to_string(),
                amount: (lottery_deposits * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr("accrued_interest", Uint128::zero().to_string()),
            attr("total_awarded_prize", awarded_prize.to_string()),
            attr(
                "reinvested_amount",
                (lottery_deposits * Uint256::one()).to_string()
            ),
        ]
    );
}

//TODO: Test lottery from ExecuteLottery, not directly from _ExecutePrize

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

    let address_raw_0 = deps.api.addr_canonicalize("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_0),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00000")],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 4 hits
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

    let address_raw_1 = deps.api.addr_canonicalize("addr0001").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_1),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00100")],
            unbonding_info: vec![]
        }
    );

    // Run lottery, one winner (5 hits), one winner (4 hits) - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);
    let msg = ExecuteMsg::_ExecutePrize {
        balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
    };
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check lottery info was updated correctly

    // total prize = balance  - lottery_deposits
    let total_prize = Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT));

    let awarded_prize_0 = total_prize * Decimal256::percent(50);
    let awarded_prize_1 = total_prize * Decimal256::percent(30);
    let awarded_prize = awarded_prize_0 + awarded_prize_1;

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            winners: vec![
                (5, vec![address_raw_0.clone()]),
                (4, vec![address_raw_1.clone()])
            ]
        }
    );

    let prize_assigned_0 =
        read_depositor_info(deps.as_ref().storage, &address_raw_0).redeemable_amount;
    let prize_assigned_1 =
        read_depositor_info(deps.as_ref().storage, &address_raw_1).redeemable_amount;

    let mock_prize_0 = awarded_prize_0 - (awarded_prize_0 * Decimal256::percent(RESERVE_FACTOR));
    let mock_prize_1 = awarded_prize_1 - (awarded_prize_1 * Decimal256::percent(RESERVE_FACTOR));

    assert_eq!(prize_assigned_0, (mock_prize_0 * Uint256::one()).into());
    assert_eq!(prize_assigned_1, (mock_prize_1 * Uint256::one()).into());

    let state = read_state(deps.as_ref().storage).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(
        state.total_reserve,
        awarded_prize * Decimal256::percent(RESERVE_FACTOR)
    );

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

    // reinvest lottery deposits
    let lottery_deposits =
        Decimal256::percent(TICKET_PRICE * 2) * Decimal256::percent(SPLIT_FACTOR);

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: "uusd".to_string(),
                amount: (lottery_deposits * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr("accrued_interest", Uint128::zero().to_string()),
            attr("total_awarded_prize", awarded_prize.to_string()),
            attr(
                "reinvested_amount",
                (lottery_deposits * Uint256::one()).to_string()
            ),
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
        combinations: vec![String::from("00000")],
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: DENOM.to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_0 = deps.api.addr_canonicalize("addr0000").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_0),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00000")],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 4 hits
    let msg = ExecuteMsg::Deposit {
        combinations: vec![String::from("00000")],
    };
    let info = mock_info(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRICE) * Uint256::one()).into(),
        }],
    );

    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let address_raw_1 = deps.api.addr_canonicalize("addr0001").unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(deps.as_ref().storage, &address_raw_1),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRICE),
            shares: Decimal256::percent(TICKET_PRICE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128::zero(),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00000")],
            unbonding_info: vec![]
        }
    );

    // Run lottery, one winner (5 hits), one winner (4 hits) - should run correctly
    let info = mock_info(MOCK_CONTRACT_ADDR, &[]);
    let msg = ExecuteMsg::_ExecutePrize {
        balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
    };
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check lottery info was updated correctly

    // total prize
    let total_prize = Decimal256::from_uint256(Uint256::from(INITIAL_DEPOSIT_AMOUNT));

    let awarded_prize = total_prize * Decimal256::percent(50);
    let awarded_prize_each = awarded_prize * Decimal256::percent(50); //divide by two

    assert_eq!(
        read_lottery_info(deps.as_ref().storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            winners: vec![(5, vec![address_raw_0.clone(), address_raw_1.clone()])]
        }
    );

    let prize_assigned_0 =
        read_depositor_info(deps.as_ref().storage, &address_raw_0).redeemable_amount;
    let prize_assigned_1 =
        read_depositor_info(deps.as_ref().storage, &address_raw_1).redeemable_amount;

    let mock_prize_each =
        awarded_prize_each - (awarded_prize_each * Decimal256::percent(RESERVE_FACTOR));

    assert_eq!(prize_assigned_0, (mock_prize_each * Uint256::one()).into());
    assert_eq!(prize_assigned_1, (mock_prize_each * Uint256::one()).into());

    let state = read_state(&deps.storage).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(
        state.total_reserve,
        awarded_prize * Decimal256::percent(RESERVE_FACTOR)
    );

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

    // reinvest lottery deposits
    let lottery_deposits =
        Decimal256::percent(TICKET_PRICE * 2) * Decimal256::percent(SPLIT_FACTOR);

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ANCHOR.to_string(),
            funds: vec![Coin {
                denom: DENOM.to_string(),
                amount: (lottery_deposits * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "execute_prize"),
            attr("accrued_interest", Uint128::zero().to_string()),
            attr("total_awarded_prize", awarded_prize.to_string()),
            attr(
                "reinvested_amount",
                (lottery_deposits * Uint256::one()).to_string()
            ),
        ]
    );
}

#[test]
fn execute_prize_many_different_winning_combinations() {
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

    let addresses_count = 1500u64;
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
    let msg = ExecuteMsg::_ExecutePrize {
        balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
    };
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // Check lottery info was updated correctly

    let lottery_info = read_lottery_info(deps.as_ref().storage, 0u64);

    assert!(lottery_info.awarded);
}

#[test]
fn claim_rewards_one_depositor() {
    // Initialize contract
    let mut deps = mock_dependencies(&[]);

    mock_instantiate(deps.as_mut());
    mock_register_contracts(deps.as_mut());

    let info = mock_info("addr0000", &[]);

    let mut state = read_state(deps.as_ref().storage).unwrap();
    state.glow_emission_rate = Decimal256::one();
    store_state(deps.as_mut().storage, &state).unwrap();

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

    let mut state = read_state(deps.as_mut().storage).unwrap();
    state.glow_emission_rate = Decimal256::one();
    store_state(&mut deps.storage, &state).unwrap();

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

    let state = read_state(deps.as_mut().storage).unwrap();
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

    let mut state = read_state(&deps.storage).unwrap();
    state.total_reserve = Decimal256::percent(50000); // 500
    store_state(deps.as_mut().storage, &state).unwrap();

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

    let state = read_state(deps.as_ref().storage).unwrap();
    // Glow Emission rate must be 1 as hard-coded in mock querier
    assert_eq!(
        state,
        State {
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

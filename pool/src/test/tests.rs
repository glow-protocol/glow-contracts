use crate::contract::{handle, init, query, query_config, INITIAL_DEPOSIT_AMOUNT, SEQUENCE_DIGITS};
use crate::state::{
    read_config, read_depositor_info, read_sequence_info, read_state, store_config, store_state,
    Config, DepositorInfo, State,
};

use crate::msg::{ConfigResponse, HandleMsg, InitMsg, QueryMsg, StateResponse};
use crate::test::mock_querier::mock_dependencies;
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::testing::{mock_env, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    from_binary, log, to_binary, Api, BankMsg, Coin, CosmosMsg, Decimal, Extern, HandleResponse,
    HumanAddr, InitResponse, Querier, StdError, Storage, Uint128, WasmMsg,
};
use cw20::{Cw20CoinHuman, Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};

use cw0::{Duration, HOUR, WEEK};
use moneymarket::market::{Cw20HookMsg, HandleMsg as AnchorMsg};
use std::ops::{Add, Mul};
use std::str::FromStr;
use terraswap::hook::InitHook;
use terraswap::token::InitMsg as TokenInitMsg;

const TICKET_PRIZE: u64 = 1000; // 10 as %
const SPLIT_FACTOR: u64 = 75; // as a %
const RESERVE_FACTOR: u64 = 5; // as a %
const RATE: u64 = 1023; // as a permille

fn initialize<S: Storage, A: Api, Q: Querier>(mut deps: &mut Extern<S, A, Q>) {
    // TODO: Refactor tests
}

#[test]
fn proper_initialization() {
    let mut deps = mock_dependencies(
        20,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let msg = InitMsg {
        owner: HumanAddr::from("owner"),
        stable_denom: "uusd".to_string(),
        anchor_contract: HumanAddr::from("anchor"),
        lottery_interval: WEEK,
        block_time: HOUR,
        ticket_prize: Decimal256::percent(TICKET_PRIZE),
        prize_distribution: vec![
            Decimal256::zero(),
            Decimal256::zero(),
            Decimal256::percent(5),
            Decimal256::percent(15),
            Decimal256::percent(30),
            Decimal256::percent(50),
        ],
        reserve_factor: Decimal256::percent(RESERVE_FACTOR),
        split_factor: Decimal256::percent(SPLIT_FACTOR),
        unbonding_period: WEEK,
    };

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let res = init(&mut deps, env.clone(), msg).unwrap();

    assert_eq!(res, InitResponse::default());

    // Test query config
    let query_res = query(&deps, QueryMsg::Config {}).unwrap();
    let config_res: ConfigResponse = from_binary(&query_res).unwrap();
    assert_eq!(HumanAddr::from("owner"), config_res.owner);
    assert_eq!("uusd".to_string(), config_res.stable_denom);
    assert_eq!(HumanAddr::from("anchor"), config_res.anchor_contract);
    assert_eq!(WEEK, config_res.lottery_interval);
    assert_eq!(HOUR, config_res.block_time);
    assert_eq!(Decimal256::percent(TICKET_PRIZE), config_res.ticket_prize);
    assert_eq!(
        vec![
            Decimal256::zero(),
            Decimal256::zero(),
            Decimal256::percent(5),
            Decimal256::percent(15),
            Decimal256::percent(30),
            Decimal256::percent(50)
        ],
        config_res.prize_distribution
    );
    assert_eq!(Decimal256::percent(5), config_res.reserve_factor);
    assert_eq!(Decimal256::percent(75), config_res.split_factor);
    assert_eq!(WEEK, config_res.unbonding_period);

    // Test query state
    let query_res = query(&deps, QueryMsg::State { block_height: None }).unwrap();
    let state_res: StateResponse = from_binary(&query_res).unwrap();
    assert_eq!(state_res.total_tickets, Uint256::zero());
    assert_eq!(
        state_res.total_reserve,
        Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT)
    );
    assert_eq!(state_res.total_deposits, Decimal256::zero());
    assert_eq!(state_res.lottery_deposits, Decimal256::zero());
    assert_eq!(state_res.shares_supply, Decimal256::zero());
    assert_eq!(state_res.award_available, Decimal256::zero());
    assert_eq!(state_res.spendable_balance, Decimal256::zero());
    assert_eq!(
        state_res.current_balance,
        Uint256::from(INITIAL_DEPOSIT_AMOUNT)
    );
    assert_eq!(state_res.current_lottery, 0);
    assert_eq!(state_res.next_lottery_time, WEEK.after(&env.block));
}

#[test]
fn update_config() {
    let mut deps = mock_dependencies(
        20,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let msg = InitMsg {
        owner: HumanAddr::from("owner"),
        stable_denom: "uusd".to_string(),
        anchor_contract: HumanAddr::from("anchor"),
        lottery_interval: WEEK,
        block_time: HOUR,
        ticket_prize: Decimal256::percent(TICKET_PRIZE),
        prize_distribution: vec![
            Decimal256::zero(),
            Decimal256::zero(),
            Decimal256::percent(5),
            Decimal256::percent(15),
            Decimal256::percent(30),
            Decimal256::percent(50),
        ],
        reserve_factor: Decimal256::percent(5),
        split_factor: Decimal256::percent(75),
        unbonding_period: WEEK,
    };

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = init(&mut deps, env.clone(), msg).unwrap();

    // update owner
    let env = mock_env("owner", &[]);
    let msg = HandleMsg::UpdateConfig {
        owner: Some(HumanAddr::from("owner1".to_string())),
        lottery_interval: None,
        block_time: None,
        ticket_prize: None,
        prize_distribution: None,
        reserve_factor: None,
        split_factor: None,
        unbonding_period: None,
    };
    let res = handle(&mut deps, env, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // Check owner has changed
    let res = query(&deps, QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();

    assert_eq!(HumanAddr::from("owner1"), config_response.owner);

    // update lottery interval to 1000 blocks
    let env = mock_env("owner1", &[]);
    let msg = HandleMsg::UpdateConfig {
        owner: None,
        lottery_interval: Some(Duration::Height(1000)),
        block_time: None,
        ticket_prize: None,
        prize_distribution: None,
        reserve_factor: None,
        split_factor: None,
        unbonding_period: None,
    };

    let res = handle(&mut deps, env, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check lottery_interval has changed
    let res = query(&deps, QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.lottery_interval, Duration::Height(1000));

    // update reserve_factor to 1%
    let env = mock_env("owner1", &[]);
    let msg = HandleMsg::UpdateConfig {
        owner: None,
        lottery_interval: None,
        block_time: None,
        ticket_prize: None,
        prize_distribution: None,
        reserve_factor: Some(Decimal256::percent(1)),
        split_factor: None,
        unbonding_period: None,
    };

    let res = handle(&mut deps, env, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check reserve_factor has changed
    let res = query(&deps, QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.reserve_factor, Decimal256::percent(1));

    // check only owner can update config
    let env = mock_env("owner", &[]);
    let msg = HandleMsg::UpdateConfig {
        owner: None,
        lottery_interval: Some(Duration::Height(1000)),
        block_time: None,
        ticket_prize: None,
        prize_distribution: None,
        reserve_factor: None,
        split_factor: None,
        unbonding_period: None,
    };

    let res = handle(&mut deps, env, msg);
    match res {
        Err(StdError::Unauthorized { .. }) => {}
        _ => panic!("Must return unauthorized error"),
    }
}

#[test]
fn single_deposit() {
    // Initialize contract
    let mut deps = mock_dependencies(
        20,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );
    let msg = InitMsg {
        owner: HumanAddr::from("owner"),
        stable_denom: "uusd".to_string(),
        anchor_contract: HumanAddr::from("anchor"),
        lottery_interval: WEEK,
        block_time: HOUR,
        ticket_prize: Decimal256::percent(TICKET_PRIZE),
        prize_distribution: vec![
            Decimal256::zero(),
            Decimal256::zero(),
            Decimal256::percent(5),
            Decimal256::percent(15),
            Decimal256::percent(30),
            Decimal256::percent(50),
        ],
        reserve_factor: Decimal256::percent(5),
        split_factor: Decimal256::percent(75),
        unbonding_period: WEEK,
    };

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = init(&mut deps, env.clone(), msg).unwrap();

    // Must deposit stable_denom coins
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("13579"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "ukrw".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "Deposit amount must be greater than 0 uusd")
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // correct base denom, zero deposit
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::zero(),
        }],
    );

    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "Deposit amount must be greater than 0 uusd")
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    let wrong_amount = Decimal256::percent(TICKET_PRIZE * 2);

    // correct base denom, deposit different to ticket_prize
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (wrong_amount * Uint256::one()).into(),
        }],
    );

    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(
                msg,
                format!(
                    "Deposit amount must be equal to a ticket prize: {} uusd",
                    Decimal256::percent(TICKET_PRIZE)
                )
            )
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // TODO: current lottery deposit time is expired

    // Invalid ticket sequence - more number of digits
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("123579"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );
    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(
                msg,
                format!(
                    "Ticket sequence must be {} characters between 0-9",
                    SEQUENCE_DIGITS
                )
            )
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - less number of digits
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("1359"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );
    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(
                msg,
                format!(
                    "Ticket sequence must be {} characters between 0-9",
                    SEQUENCE_DIGITS
                )
            )
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - only numbers allowed
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("1e579"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );
    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(
                msg,
                format!(
                    "Ticket sequence must be {} characters between 0-9",
                    SEQUENCE_DIGITS
                )
            )
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Correct single deposit - buys one ticket
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("13579"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(1023));

    /*
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT + TICKET_PRIZE,
        }],
    );
     */

    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Check address of sender was stored correctly in the sequence bucket
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("13579")),
        vec![deps
            .api
            .canonical_address(&HumanAddr::from("addr0000"))
            .unwrap()]
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            &deps.storage,
            &deps
                .api
                .canonical_address(&HumanAddr::from("addr0000"))
                .unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE),
            shares: Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128(0),
            tickets: vec![String::from("13579")],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::one(),
            total_reserve: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            total_deposits: Decimal256::percent(TICKET_PRIZE),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE) * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE),
            award_available: Decimal256::zero(),
            spendable_balance: Decimal256::zero(),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block)
        }
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: String::from("uusd"),
                amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "single_deposit"),
            log("depositor", "addr0000"),
            log("deposit_amount", Decimal256::percent(TICKET_PRIZE)),
            log(
                "shares_minted",
                Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE)
            ),
        ]
    );

    // TODO: same address, buys second ticket but same sequence - should fail

    // Same address, correctly buys second ticket different sequence
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("23456"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(1023));

    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Check address of sender was stored correctly in the sequence bucket
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("23456")),
        vec![deps
            .api
            .canonical_address(&HumanAddr::from("addr0000"))
            .unwrap()]
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            &deps.storage,
            &deps
                .api
                .canonical_address(&HumanAddr::from("addr0000"))
                .unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE * 2),
            shares: (Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE))
                .mul(Decimal256::percent(200)),
            redeemable_amount: Uint128(0),
            tickets: vec![String::from("13579"), String::from("23456")],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::from(2u64),
            total_reserve: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            total_deposits: Decimal256::percent(TICKET_PRIZE * 2),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE * 2)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: (Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE))
                .mul(Decimal256::percent(200)),
            award_available: Decimal256::zero(),
            spendable_balance: Decimal256::zero(),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block)
        }
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: String::from("uusd"),
                amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "single_deposit"),
            log("depositor", "addr0000"),
            log("deposit_amount", Decimal256::percent(TICKET_PRIZE)),
            log(
                "shares_minted",
                Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE)
            ),
        ]
    );

    // New address, buys the 3rd overall ticket with a new sequence
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("98765"),
    };
    let env = mock_env(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(1023));

    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Check address of sender was stored correctly in the sequence bucket
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("98765")),
        vec![deps
            .api
            .canonical_address(&HumanAddr::from("addr0001"))
            .unwrap()]
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            &deps.storage,
            &deps
                .api
                .canonical_address(&HumanAddr::from("addr0001"))
                .unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE),
            shares: (Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE)),
            redeemable_amount: Uint128(0),
            tickets: vec![String::from("98765")],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::from(3u64),
            total_reserve: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            total_deposits: Decimal256::percent(TICKET_PRIZE * 3),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE * 3)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: (Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE))
                .mul(Decimal256::percent(300)),
            award_available: Decimal256::zero(),
            spendable_balance: Decimal256::zero(),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block)
        }
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: String::from("uusd"),
                amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "single_deposit"),
            log("depositor", "addr0001"),
            log("deposit_amount", Decimal256::percent(TICKET_PRIZE)),
            log(
                "shares_minted",
                Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE)
            ),
        ]
    );

    // New address, buy ticket with existing sequence
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("23456"),
    };
    let env = mock_env(
        "addr0002",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(1023));

    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Check address of sender was stored correctly in the sequence bucket
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("23456")),
        vec![
            deps.api
                .canonical_address(&HumanAddr::from("addr0000"))
                .unwrap(),
            deps.api
                .canonical_address(&HumanAddr::from("addr0002"))
                .unwrap()
        ]
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            &deps.storage,
            &deps
                .api
                .canonical_address(&HumanAddr::from("addr0002"))
                .unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE),
            shares: (Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE)),
            redeemable_amount: Uint128(0),
            tickets: vec![String::from("23456")],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::from(4u64),
            total_reserve: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            total_deposits: Decimal256::percent(TICKET_PRIZE * 4),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE * 4)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: (Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE))
                .mul(Decimal256::percent(400)),
            award_available: Decimal256::zero(),
            spendable_balance: Decimal256::zero(),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block)
        }
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: String::from("uusd"),
                amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "single_deposit"),
            log("depositor", "addr0002"),
            log("deposit_amount", Decimal256::percent(TICKET_PRIZE)),
            log(
                "shares_minted",
                Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE)
            ),
        ]
    );
}

#[test]
fn withdraw() {}

#[test]
fn claim() {}

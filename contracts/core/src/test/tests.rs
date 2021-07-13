use crate::contract::{handle, init, query, INITIAL_DEPOSIT_AMOUNT, SEQUENCE_DIGITS};
use crate::state::{
    read_depositor_info, read_lottery_info, read_sequence_info, read_state, DepositorInfo,
    LotteryInfo, State,
};
use crate::test::mock_querier::mock_dependencies;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::testing::{mock_env, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    from_binary, log, to_binary, Api, BankMsg, Coin, CosmosMsg, Decimal, Env, Extern, HumanAddr,
    InitResponse, Querier, StdError, Storage, Uint128, WasmMsg,
};
use cw20::Cw20HandleMsg;
use glow_protocol::core::{ConfigResponse, HandleMsg, InitMsg, QueryMsg, StateResponse};

use cw0::{Duration, Expiration, HOUR, WEEK};
use glow_protocol::core::Claim;
use moneymarket::market::{Cw20HookMsg, HandleMsg as AnchorMsg};
use moneymarket::querier::deduct_tax;
use std::ops::{Add, Div, Mul};

const TICKET_PRIZE: u64 = 1_000_000_000; // 10_000_000 as %
const SPLIT_FACTOR: u64 = 75; // as a %
const RESERVE_FACTOR: u64 = 5; // as a %
const RATE: u64 = 1023; // as a permille
const WEEK_TIME: u64 = 604800; // in seconds
const HOUR_TIME: u64 = 3600; // in seconds

fn initialize<S: Storage, A: Api, Q: Querier>(
    mut deps: &mut Extern<S, A, Q>,
    env: Env,
) -> InitResponse {
    let msg = InitMsg {
        owner: HumanAddr::from("owner"),
        stable_denom: "uusd".to_string(),
        anchor_contract: HumanAddr::from("anchor"),
        aterra_contract: HumanAddr::from("aterra"),
        lottery_interval: WEEK_TIME,
        block_time: HOUR_TIME,
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
        unbonding_period: WEEK_TIME,
    };

    init(&mut deps, env.clone(), msg).unwrap()
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

    let env = mock_env(
        "owner",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let res = initialize(&mut deps, env.clone());
    assert_eq!(res, InitResponse::default());

    // Register contracts
    let msg = HandleMsg::RegisterContracts {
        collector_contract: HumanAddr::from("collector"),
        distributor_contract: HumanAddr::from("distributor"),
    };
    let env = mock_env("owner", &[]);
    let _res = handle(&mut deps, env, msg).unwrap();

    // Cannot register contracts again
    let msg = HandleMsg::RegisterContracts {
        collector_contract: HumanAddr::from("collector"),
        distributor_contract: HumanAddr::from("distributor"),
    };
    let env = mock_env("owner", &[]);
    let _res = handle(&mut deps, env.clone(), msg).unwrap_err();

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
    assert_eq!(
        HumanAddr::from("distributor"),
        config_res.distributor_contract
    );
    assert_eq!(HumanAddr::from("collector"), config_res.gov_contract);

    // Test query state
    let query_res = query(&deps, QueryMsg::State { block_height: None }).unwrap();
    let state_res: StateResponse = from_binary(&query_res).unwrap();
    assert_eq!(state_res.total_tickets, Uint256::zero());
    assert_eq!(state_res.total_reserve, Decimal256::zero());
    assert_eq!(state_res.lottery_deposits, Decimal256::zero());
    assert_eq!(state_res.shares_supply, Decimal256::zero());
    assert_eq!(
        state_res.award_available,
        Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT)
    );
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

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    // Register contracts
    let msg = HandleMsg::RegisterContracts {
        collector_contract: HumanAddr::from("collector"),
        distributor_contract: HumanAddr::from("distributor"),
    };
    let env = mock_env("owner", &[]);
    let _res = handle(&mut deps, env, msg).unwrap();

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

    // update lottery interval to 30 minutes
    let env = mock_env("owner1", &[]);
    let msg = HandleMsg::UpdateConfig {
        owner: None,
        lottery_interval: Some(1800),
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
    assert_eq!(config_response.lottery_interval, Duration::Time(1800));

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
        lottery_interval: Some(1800),
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

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

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
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("13579")],
            unbonding_info: vec![]
        }
    );

    let minted_shares = Decimal256::percent(TICKET_PRIZE).div(Decimal256::permille(RATE));
    let minted_deposit_shares =
        minted_shares - minted_shares.mul(Decimal256::percent(SPLIT_FACTOR));

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::one(),
            total_reserve: Decimal256::zero(),
            total_deposits: Decimal256::percent(TICKET_PRIZE),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE) * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: minted_shares,
            deposit_shares: minted_deposit_shares,
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    // TODO: how can test pass with and without deduct_tax??
    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![deduct_tax(
                &deps,
                Coin {
                    denom: String::from("uusd"),
                    amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
                }
            )
            .unwrap()],
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
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("13579"), String::from("23456")],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::from(2u64),
            total_reserve: Decimal256::zero(),
            total_deposits: Decimal256::percent(TICKET_PRIZE * 2),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE * 2)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: minted_shares * Decimal256::percent(200),
            deposit_shares: minted_deposit_shares * Decimal256::percent(200),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
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
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("98765")],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::from(3u64),
            total_reserve: Decimal256::zero(),
            total_deposits: Decimal256::percent(TICKET_PRIZE * 3),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE * 3)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: minted_shares * Decimal256::percent(300),
            deposit_shares: minted_deposit_shares * Decimal256::percent(300),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
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
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("23456")],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::from(4u64),
            total_reserve: Decimal256::zero(),
            total_deposits: Decimal256::percent(TICKET_PRIZE * 4),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE * 4)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: minted_shares * Decimal256::percent(400),
            deposit_shares: minted_deposit_shares * Decimal256::percent(400),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
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

    // TODO: deposit fails when current lottery deposit time is expired
}

#[test]
fn batch_deposit() {
    // Initialize contract
    let mut deps = mock_dependencies(
        20,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    // Must deposit stable_denom coins
    let msg = HandleMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("34567")],
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

    // Invalid ticket sequence - more number of digits
    let msg = HandleMsg::Deposit {
        combinations: vec![String::from("135797"), String::from("34567")],
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE * 2u64) * Uint256::one()).into(),
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
    let msg = HandleMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("3457")],
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE * 2u64) * Uint256::one()).into(),
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
    let msg = HandleMsg::Deposit {
        combinations: vec![String::from("135w9"), String::from("34567")],
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE * 2u64) * Uint256::one()).into(),
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
    let msg = HandleMsg::Deposit {
        combinations: vec![String::from("13579"), String::from("34567")],
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::from(2u64)).into(),
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

    // Check address of sender was stored correctly in both sequence buckets
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("13579")),
        vec![deps
            .api
            .canonical_address(&HumanAddr::from("addr0000"))
            .unwrap()]
    );
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("34567")),
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
            deposit_amount: Decimal256::percent(TICKET_PRIZE * 2u64),
            shares: Decimal256::percent(TICKET_PRIZE * 2u64) / Decimal256::permille(RATE),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("13579"), String::from("34567")],
            unbonding_info: vec![]
        }
    );

    let minted_shares = Decimal256::percent(TICKET_PRIZE * 2u64).div(Decimal256::permille(RATE));

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::from(2u64),
            total_reserve: Decimal256::zero(),
            total_deposits: Decimal256::percent(TICKET_PRIZE * 2u64),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE * 2u64)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: minted_shares,
            deposit_shares: minted_shares - minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block),
            last_reward_updated: 12345, //TODO: hardcoded. why this value?
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: String::from("uusd"),
                amount: (Decimal256::percent(TICKET_PRIZE * 2u64) * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "batch_deposit"),
            log("depositor", "addr0000"),
            log("deposit_amount", Decimal256::percent(TICKET_PRIZE * 2u64)),
            log(
                "shares_minted",
                Decimal256::percent(TICKET_PRIZE * 2u64) / Decimal256::permille(RATE)
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
    let mut deps = mock_dependencies(
        20,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    // Must deposit stable_denom coins
    let msg = HandleMsg::Gift {
        combinations: vec![String::from("13579"), String::from("34567")],
        recipient: HumanAddr::from("addr1111"),
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
            assert_eq!(msg, "Deposit amount to gift must be greater than 0 uusd")
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
            assert_eq!(msg, "Deposit amount to gift must be greater than 0 uusd")
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    let wrong_amount = Decimal256::percent(TICKET_PRIZE * 4);

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
                    "Deposit amount required to gift 2 tickets is {} uusd",
                    Decimal256::percent(TICKET_PRIZE * 2u64)
                )
            )
        }
        _ => panic!("DO NOT ENTER HERE"),
    }
    // Invalid recipient - you cannot make a gift to yourself
    let msg = HandleMsg::Gift {
        combinations: vec![String::from("13597"), String::from("34567")],
        recipient: HumanAddr::from("addr0000"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE * 2u64) * Uint256::one()).into(),
        }],
    );
    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(
                msg,
                format!("You cannot gift tickets to yourself, just make a regular deposit",)
            )
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Invalid ticket sequence - more number of digits
    let msg = HandleMsg::Gift {
        combinations: vec![String::from("135797"), String::from("34567")],
        recipient: HumanAddr::from("addr1111"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE * 2u64) * Uint256::one()).into(),
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
    let msg = HandleMsg::Gift {
        combinations: vec![String::from("13579"), String::from("3457")],
        recipient: HumanAddr::from("addr1111"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE * 2u64) * Uint256::one()).into(),
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
    let msg = HandleMsg::Gift {
        combinations: vec![String::from("135w9"), String::from("34567")],
        recipient: HumanAddr::from("addr1111"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE * 2u64) * Uint256::one()).into(),
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
    let msg = HandleMsg::Gift {
        combinations: vec![String::from("13579"), String::from("34567")],
        recipient: HumanAddr::from("addr1111"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::from(2u64)).into(),
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

    // Check address of sender was stored correctly in both sequence buckets
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("13579")),
        vec![deps
            .api
            .canonical_address(&HumanAddr::from("addr1111"))
            .unwrap()]
    );
    assert_eq!(
        read_sequence_info(&deps.storage, &String::from("34567")),
        vec![deps
            .api
            .canonical_address(&HumanAddr::from("addr1111"))
            .unwrap()]
    );

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(
            &deps.storage,
            &deps
                .api
                .canonical_address(&HumanAddr::from("addr1111"))
                .unwrap()
        ),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE * 2u64),
            shares: Decimal256::percent(TICKET_PRIZE * 2u64) / Decimal256::permille(RATE),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("13579"), String::from("34567")],
            unbonding_info: vec![]
        }
    );

    let minted_shares = Decimal256::percent(TICKET_PRIZE * 2u64).div(Decimal256::permille(RATE));

    assert_eq!(
        read_state(&deps.storage).unwrap(),
        State {
            total_tickets: Uint256::from(2u64),
            total_reserve: Decimal256::zero(),
            total_deposits: Decimal256::percent(TICKET_PRIZE * 2u64),
            lottery_deposits: Decimal256::percent(TICKET_PRIZE * 2u64)
                * Decimal256::percent(SPLIT_FACTOR),
            shares_supply: minted_shares,
            deposit_shares: minted_shares - minted_shares.mul(Decimal256::percent(SPLIT_FACTOR)),
            award_available: Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT),
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: String::from("uusd"),
                amount: (Decimal256::percent(TICKET_PRIZE * 2u64) * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "gift_tickets"),
            log("gifter", "addr0000"),
            log("recipient", "addr1111"),
            log("deposit_amount", Decimal256::percent(TICKET_PRIZE * 2u64)),
            log("tickets", 2u64),
            log(
                "shares_minted",
                Decimal256::percent(TICKET_PRIZE * 2u64) / Decimal256::permille(RATE)
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
    let mut deps = mock_dependencies(
        20,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    // Address buys one ticket
    let env = mock_env(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    let msg = HandleMsg::SingleDeposit {
        combination: String::from("23456"),
    };

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(1023));
    let _res = handle(&mut deps, env, msg).unwrap();

    let shares = (Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(1023)) * Uint256::one();

    let env = mock_env("addr0001", &[]);

    let msg = HandleMsg::Withdraw { instant: None };

    deps.querier.with_token_balances(&[(
        &HumanAddr::from("aterra"),
        &[(&HumanAddr::from(MOCK_CONTRACT_ADDR), &shares.into())],
    )]);

    // Correct withdraw, user has 1 ticket to be withdrawn
    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

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
        &deps,
        Coin {
            denom: String::from("uusd"),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        },
    )
    .unwrap()
    .amount;

    // TODO: use below redeem amount instead of hardcoded unbonding info

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
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![Claim {
                amount: Decimal256::from_uint256(Uint256::from(9999999u128)),
                release_at: WEEK.after(&env.block),
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
            current_balance: Uint256::from(INITIAL_DEPOSIT_AMOUNT),
            current_lottery: 0,
            next_lottery_time: WEEK.after(&env.block),
            last_reward_updated: 12345,
            global_reward_index: Decimal256::zero(),
            glow_emission_rate: Decimal256::zero(),
        }
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("aterra"),
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Send {
                contract: HumanAddr::from("anchor"),
                amount: shares.into(),
                msg: Some(to_binary(&Cw20HookMsg::RedeemStable {}).unwrap()),
            })
            .unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "withdraw_ticket"),
            log("depositor", "addr0001"),
            log("tickets_amount", 1u64),
            log("redeem_amount_anchor", shares),
        ]
    );
}

#[test]
fn claim() {
    // Initialize contract
    let mut deps = mock_dependencies(
        20,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    // Address buys one ticket
    let env = mock_env(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    let msg = HandleMsg::SingleDeposit {
        combination: String::from("23456"),
    };

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(1023));
    let _res = handle(&mut deps, env, msg).unwrap();

    // Address withdraws one ticket
    let env = mock_env("addr0001", &[]);
    let msg = HandleMsg::Withdraw { instant: None };

    let shares = (Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(1023)) * Uint256::one();

    deps.querier.with_token_balances(&[(
        &HumanAddr::from("aterra"),
        &[(&HumanAddr::from(MOCK_CONTRACT_ADDR), &shares.into())],
    )]);

    // Correct withdraw, user has 1 ticket to be withdrawn
    let _res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Claim 0 amount, should fail
    let msg = HandleMsg::Claim {
        amount: Some(Uint128::zero()),
    };
    let res = handle(&mut deps, env.clone(), msg.clone());

    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "Claim amount must be greater than zero")
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Claim amount that you don't have, should fail
    let env = mock_env("addr0002", &[]);
    let msg = HandleMsg::Claim {
        amount: Some(Uint128::from(10u64)),
    };

    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "Depositor does not have any amount to claim")
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Claim amount that you have, but still in unbonding state, should fail
    let mut env = mock_env("addr0001", &[]);
    let msg = HandleMsg::Claim {
        amount: Some(Uint128::from(10u64)),
    };

    let res = handle(&mut deps, env.clone(), msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "Depositor does not have any amount to claim")
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    let msg = HandleMsg::Claim { amount: None };

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time += time;
    }
    // TODO: change also the exchange rate here

    // TODO: add case asking for more amount that the one we have (which is non-zero)
    // TODO: add case asking for an amount (not None) that we do have
    // TODO: add case where contract balances are not enough to fulfill claim

    // TODO: this update is not needed (??)
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT + 10000000u128),
        }],
    );

    // Claim amount is already unbonded, so claim execution should work
    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

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
            deposit_amount: Decimal256::zero(),
            shares: Decimal256::zero(),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![],
            unbonding_info: vec![]
        }
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Bank(BankMsg::Send {
            from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
            to_address: HumanAddr::from("addr0001"),
            amount: vec![Coin {
                denom: String::from("uusd"),
                amount: Uint128::from(9_999_999u64), //TODO: should be 10_000_000
            }],
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "claim"),
            log("depositor", "addr0001"),
            log("redeemed_amount", 9_999_999u64),
            log("redeemable_amount_left", Uint128(0)),
        ]
    );
}

#[test]
fn execute_lottery() {
    // Initialize contract
    let mut deps = mock_dependencies(
        20,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    let env = mock_env(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(1),
        }],
    );

    let msg = HandleMsg::ExecuteLottery {};

    let res = handle(&mut deps, env, msg.clone());

    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "Do not send funds when executing the lottery")
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    let mut env = mock_env("addr0001", &[]);
    let res = handle(&mut deps, env.clone(), msg.clone());

    let mut lottery_expiration: u64 = 0;
    if let Duration::Time(time) = WEEK {
        lottery_expiration = env.block.time + time;
    }

    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(
                msg,
                format!(
                    "Lottery is still running, please check again after expiration time: {}",
                    lottery_expiration
                )
            )
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Advance one week in time
    if let Duration::Time(time) = WEEK {
        env.block.time += time;
    }

    // It should not fail, but redeem message is not called
    // TODO: add test case
    /*
    let res = handle(&mut deps, env.clone(), msg.clone());

    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "There is no available funds to execute the lottery",)
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

     */

    // Add 10 aUST to our contract balance
    deps.querier.with_token_balances(&[(
        &HumanAddr::from("aterra"),
        &[(
            &HumanAddr::from(MOCK_CONTRACT_ADDR),
            &Uint128::from(10_000_000u128),
        )],
    )]);

    // Add 100 UST to our contract balance
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(100_000_000u128),
        }],
    );

    let to_redeem = Uint256::from(10_000_000u128);

    // TODO: add test case with deposit_shares != 0

    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Directly check next_lottery_time has been set up for next week
    let next_lottery_time = read_state(&deps.storage).unwrap().next_lottery_time;
    let current_balance = read_state(&deps.storage).unwrap().current_balance;

    assert_eq!(
        next_lottery_time,
        Expiration::AtTime(env.block.time).add(WEEK).unwrap()
    );

    assert_eq!(current_balance, Uint256::from(100_000_000u128));

    assert_eq!(
        res.messages,
        vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("aterra"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Send {
                    contract: HumanAddr::from("anchor"),
                    amount: to_redeem.into(),
                    msg: Some(to_binary(&Cw20HookMsg::RedeemStable {}).unwrap()),
                })
                .unwrap(),
            }),
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                send: vec![],
                msg: to_binary(&HandleMsg::_HandlePrize {}).unwrap(),
            })
        ]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "execute_lottery"),
            log("redeemed_amount", to_redeem),
        ]
    );
}

#[test]
fn handle_prize_no_tickets() {
    // Initialize contract
    let mut deps = mock_dependencies(
        20,
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    let env = mock_env("addr0001", &[]);

    let msg = HandleMsg::_HandlePrize {};
    let res = handle(&mut deps, env, msg.clone());

    match res {
        Err(StdError::Unauthorized { .. }) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let env = mock_env(MOCK_CONTRACT_ADDR, &[]);
    let msg = HandleMsg::_HandlePrize {};

    /* The contract does not have UST balance, should fail
    let res = handle(&mut deps, env.clone(), msg.clone());
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
    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly
    assert_eq!(
        read_lottery_info(&deps.storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: Decimal256::zero(),
            winners: vec![]
        }
    );

    let state = read_state(&deps.storage).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero()); // From the initialization of the contract
    assert_eq!(
        state.award_available,
        Decimal256::from_uint256(Uint256::from(100_000_000u128))
    );

    assert_eq!(res.messages, vec![]);

    assert_eq!(
        res.log,
        vec![
            log("action", "handle_prize"),
            log("accrued_interest", Uint256::zero()),
            log("total_awarded_prize", Decimal256::zero()),
            log("reinvested_amount", Uint256::zero()),
        ]
    );
}

#[test]
fn handle_prize_no_winners() {
    // Initialize contract
    let mut deps = mock_dependencies(20, &[]);

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(150_000_000_000u128),
        }],
    );

    // Users buys a non-winning ticket
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("11111"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    let _res = handle(&mut deps, env, msg).unwrap();

    let address_raw = deps
        .api
        .canonical_address(&HumanAddr::from("addr0000"))
        .unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &address_raw),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE),
            shares: Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("11111")],
            unbonding_info: vec![]
        }
    );

    // Run lottery, one winner (5 hits) - should run correctly
    let env = mock_env(MOCK_CONTRACT_ADDR, &[]);
    let msg = HandleMsg::_HandlePrize {};
    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    // total prize = balance - old_balance - lottery_deposits
    let total_prize = Decimal256::from_uint256(Uint256::from(150_000_000_000u128))
        - (Decimal256::percent(TICKET_PRIZE) * Decimal256::percent(SPLIT_FACTOR));

    let awarded_prize = Decimal256::zero();

    assert_eq!(
        read_lottery_info(&deps.storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            winners: vec![]
        }
    );

    let state = read_state(&deps.storage).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(state.total_reserve, Decimal256::zero());

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

    // reinvest lottery deposits
    let lottery_deposits = Decimal256::percent(TICKET_PRIZE) * Decimal256::percent(SPLIT_FACTOR);

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: "uusd".to_string(),
                amount: (lottery_deposits * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "handle_prize"),
            log("accrued_interest", Uint256::from(149_892_500_000u128)),
            log("total_awarded_prize", awarded_prize),
            log("reinvested_amount", lottery_deposits * Uint256::one()),
        ]
    );
}

#[test]
fn handle_prize_one_winner() {
    // Initialize contract
    let mut deps = mock_dependencies(20, &[]);

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(150_000_000_000u128),
        }],
    );

    // Users buys winning ticket
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("00000"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    let _res = handle(&mut deps, env, msg).unwrap();

    let address_raw = deps
        .api
        .canonical_address(&HumanAddr::from("addr0000"))
        .unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &address_raw),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE),
            shares: Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00000")],
            unbonding_info: vec![]
        }
    );

    // Run lottery, one winner (5 hits) - should run correctly
    let env = mock_env(MOCK_CONTRACT_ADDR, &[]);
    let msg = HandleMsg::_HandlePrize {};
    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    // total prize = balance - lottery_deposits
    let total_prize = Decimal256::from_uint256(Uint256::from(150_000_000_000u128))
        - (Decimal256::percent(TICKET_PRIZE) * Decimal256::percent(SPLIT_FACTOR));

    let awarded_prize = total_prize * Decimal256::percent(50);

    assert_eq!(
        read_lottery_info(&deps.storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            winners: vec![(5, vec![address_raw.clone()])]
        }
    );

    let prize_assigned = read_depositor_info(&deps.storage, &address_raw).redeemable_amount;

    // prize assigned should be (140k - 7500) / 2

    let mock_prize = awarded_prize - (awarded_prize * Decimal256::percent(RESERVE_FACTOR));

    assert_eq!(prize_assigned, (mock_prize * Uint256::one()).into());

    let state = read_state(&deps.storage).unwrap();

    assert_eq!(state.current_lottery, 1u64);
    assert_eq!(
        state.total_reserve,
        awarded_prize * Decimal256::percent(RESERVE_FACTOR)
    );

    // From the initialization of the contract
    assert_eq!(state.award_available, total_prize - awarded_prize);

    // reinvest lottery deposits
    let lottery_deposits = Decimal256::percent(TICKET_PRIZE) * Decimal256::percent(SPLIT_FACTOR);

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: "uusd".to_string(),
                amount: (lottery_deposits * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "handle_prize"),
            log("accrued_interest", Uint256::from(149_892_500_000u128)),
            log("total_awarded_prize", awarded_prize),
            log("reinvested_amount", lottery_deposits * Uint256::one()),
        ]
    );
}

#[test]
fn handle_prize_winners_diff_ranks() {
    // Initialize contract
    let mut deps = mock_dependencies(20, &[]);

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(150_000_000_000u128),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // Users buys winning ticket - 5 hits
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("00000"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    let _res = handle(&mut deps, env, msg).unwrap();

    let address_raw_0 = deps
        .api
        .canonical_address(&HumanAddr::from("addr0000"))
        .unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &address_raw_0),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE),
            shares: Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00000")],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 4 hits
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("00100"),
    };
    let env = mock_env(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    let _res = handle(&mut deps, env, msg).unwrap();

    let address_raw_1 = deps
        .api
        .canonical_address(&HumanAddr::from("addr0001"))
        .unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &address_raw_1),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE),
            shares: Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00100")],
            unbonding_info: vec![]
        }
    );

    // Run lottery, one winner (5 hits), one winner (4 hits) - should run correctly
    let env = mock_env(MOCK_CONTRACT_ADDR, &[]);
    let msg = HandleMsg::_HandlePrize {};
    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    // total prize = balance  - lottery_deposits
    let total_prize = Decimal256::from_uint256(Uint256::from(150_000_000_000u128))
        - (Decimal256::percent(2 * TICKET_PRIZE) * Decimal256::percent(SPLIT_FACTOR));

    let awarded_prize_0 = total_prize * Decimal256::percent(50);
    let awarded_prize_1 = total_prize * Decimal256::percent(30);
    let awarded_prize = awarded_prize_0 + awarded_prize_1;

    assert_eq!(
        read_lottery_info(&deps.storage, 0u64),
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

    let prize_assigned_0 = read_depositor_info(&deps.storage, &address_raw_0).redeemable_amount;
    let prize_assigned_1 = read_depositor_info(&deps.storage, &address_raw_1).redeemable_amount;

    let mock_prize_0 = awarded_prize_0 - (awarded_prize_0 * Decimal256::percent(RESERVE_FACTOR));
    let mock_prize_1 = awarded_prize_1 - (awarded_prize_1 * Decimal256::percent(RESERVE_FACTOR));

    assert_eq!(prize_assigned_0, (mock_prize_0 * Uint256::one()).into());
    assert_eq!(prize_assigned_1, (mock_prize_1 * Uint256::one()).into());

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
        Decimal256::percent(TICKET_PRIZE * 2) * Decimal256::percent(SPLIT_FACTOR);

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: "uusd".to_string(),
                amount: (lottery_deposits * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "handle_prize"),
            log("accrued_interest", Uint256::from(149_885_000_000u128)),
            log("total_awarded_prize", awarded_prize),
            log("reinvested_amount", lottery_deposits * Uint256::one()),
        ]
    );
}

#[test]
fn handle_prize_winners_same_rank() {
    // Initialize contract
    let mut deps = mock_dependencies(20, &[]);

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = initialize(&mut deps, env.clone());
    let _res = initialize(&mut deps, env.clone());

    // Add 150_000 UST to our contract balance
    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(150_000_000_000u128),
        }],
    );

    // Mock aUST-UST exchange rate
    deps.querier.with_exchange_rate(Decimal256::permille(RATE));

    // Users buys winning ticket - 5 hits
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("00000"),
    };
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    let _res = handle(&mut deps, env, msg).unwrap();

    let address_raw_0 = deps
        .api
        .canonical_address(&HumanAddr::from("addr0000"))
        .unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &address_raw_0),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE),
            shares: Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00000")],
            unbonding_info: vec![]
        }
    );

    // Users buys winning ticket - 4 hits
    let msg = HandleMsg::SingleDeposit {
        combination: String::from("00000"),
    };
    let env = mock_env(
        "addr0001",
        &[Coin {
            denom: "uusd".to_string(),
            amount: (Decimal256::percent(TICKET_PRIZE) * Uint256::one()).into(),
        }],
    );

    let _res = handle(&mut deps, env, msg).unwrap();

    let address_raw_1 = deps
        .api
        .canonical_address(&HumanAddr::from("addr0001"))
        .unwrap();

    // Check depositor info was updated correctly
    assert_eq!(
        read_depositor_info(&deps.storage, &address_raw_1),
        DepositorInfo {
            deposit_amount: Decimal256::percent(TICKET_PRIZE),
            shares: Decimal256::percent(TICKET_PRIZE) / Decimal256::permille(RATE),
            redeemable_amount: Uint128(0),
            reward_index: Decimal256::zero(),
            pending_rewards: Decimal256::zero(),
            tickets: vec![String::from("00000")],
            unbonding_info: vec![]
        }
    );

    // Run lottery, one winner (5 hits), one winner (4 hits) - should run correctly
    let env = mock_env(MOCK_CONTRACT_ADDR, &[]);
    let msg = HandleMsg::_HandlePrize {};
    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    // Check lottery info was updated correctly

    // total prize = balance - lottery_deposits
    let total_prize = Decimal256::from_uint256(Uint256::from(150_000_000_000u128))
        - (Decimal256::percent(2 * TICKET_PRIZE) * Decimal256::percent(SPLIT_FACTOR));

    let awarded_prize = total_prize * Decimal256::percent(50);
    let awarded_prize_each = awarded_prize * Decimal256::percent(50); //divide by two

    assert_eq!(
        read_lottery_info(&deps.storage, 0u64),
        LotteryInfo {
            sequence: "00000".to_string(),
            awarded: true,
            total_prizes: awarded_prize,
            winners: vec![(5, vec![address_raw_0.clone(), address_raw_1.clone()])]
        }
    );

    let prize_assigned_0 = read_depositor_info(&deps.storage, &address_raw_0).redeemable_amount;
    let prize_assigned_1 = read_depositor_info(&deps.storage, &address_raw_1).redeemable_amount;

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
        Decimal256::percent(TICKET_PRIZE * 2) * Decimal256::percent(SPLIT_FACTOR);

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("anchor"),
            send: vec![Coin {
                denom: "uusd".to_string(),
                amount: (lottery_deposits * Uint256::one()).into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {}).unwrap(),
        })]
    );

    assert_eq!(
        res.log,
        vec![
            log("action", "handle_prize"),
            log("accrued_interest", Uint256::from(149_885_000_000u128)),
            log("total_awarded_prize", awarded_prize),
            log("reinvested_amount", lottery_deposits * Uint256::one()),
        ]
    );
}

// TODO: Refactor tests
// TODO: Test prize_strategy functions combinations (without wasm)

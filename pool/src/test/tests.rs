use crate::contract::{handle, init, query, query_config, INITIAL_DEPOSIT_AMOUNT};
use crate::state::{read_config, read_state, store_config, store_state, Config, State};

use crate::msg::{ConfigResponse, Cw20HookMsg, HandleMsg, InitMsg, QueryMsg, StateResponse};
use crate::test::mock_querier::mock_dependencies;
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::testing::{mock_env, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    from_binary, log, to_binary, BankMsg, Coin, CosmosMsg, Decimal, HandleResponse, HumanAddr,
    StdError, Uint128, WasmMsg,
};
use cw20::{Cw20CoinHuman, Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};

use std::str::FromStr;
use terraswap::hook::InitHook;
use terraswap::token::InitMsg as TokenInitMsg;

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
        b_terra_code_id: 123u64,
        period_prize: 69u64,
        ticket_exchange_rate: Decimal256::one(),
    };

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let res = init(&mut deps, env.clone(), msg).unwrap();

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
            code_id: 123u64,
            send: vec![],
            label: None,
            msg: to_binary(&TokenInitMsg {
                name: "Barbell Terra USD".to_string(),
                symbol: "bUST".to_string(),
                decimals: 6u8,
                initial_balances: vec![Cw20CoinHuman {
                    address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
                }],
                mint: Some(MinterResponse {
                    minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    cap: None,
                }),
                init_hook: Some(InitHook {
                    contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    msg: to_binary(&HandleMsg::RegisterSTerra {}).unwrap(),
                }),
            })
            .unwrap(),
        })]
    );

    // Register share barbell token contract
    let msg = HandleMsg::RegisterSTerra {}; //TODO: is it not already registered?
    let env = mock_env("BT-uusd", &[]);
    let _res = handle(&mut deps, env, msg).unwrap();

    // Cannot register token contract again
    let msg = HandleMsg::RegisterSTerra {};
    let env = mock_env("BT-uusd", &[]);
    let _res = handle(&mut deps, env, msg).unwrap_err();

    // Test query config
    let query_res = query(&deps, QueryMsg::Config {}).unwrap();
    let config_res: ConfigResponse = from_binary(&query_res).unwrap();
    assert_eq!(HumanAddr::from("owner"), config_res.owner);
    assert_eq!("uusd".to_string(), config_res.stable_denom);
    assert_eq!(HumanAddr::from("anchor"), config_res.anchor_contract);
    assert_eq!(69u64, config_res.period_prize);
    assert_eq!(Decimal256::one(), config_res.ticket_exchange_rate);

    // Test query state
    let query_res = query(&deps, QueryMsg::State { block_height: None }).unwrap();
    let state_res: StateResponse = from_binary(&query_res).unwrap();
    assert_eq!(state_res.total_tickets, Decimal256::zero());
    assert_eq!(
        state_res.total_reserves,
        Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT)
    );
    assert_eq!(state_res.last_interest, Decimal256::zero());
    assert_eq!(state_res.total_accrued_interest, Decimal256::zero());
    assert_eq!(state_res.award_available, Decimal256::zero());
    assert_eq!(
        state_res.total_assets,
        Decimal256::from_uint256(INITIAL_DEPOSIT_AMOUNT)
    );
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
        b_terra_code_id: 123u64,
        period_prize: 69u64,
        ticket_exchange_rate: Decimal256::one(),
    };

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = init(&mut deps, env.clone(), msg).unwrap();

    // Register share barbell token contract
    let msg = HandleMsg::RegisterSTerra {}; //TODO: is it not already registered?
    let env = mock_env("BT-uusd", &[]);
    let _res = handle(&mut deps, env, msg).unwrap();

    // update owner
    let env = mock_env("owner", &[]);
    let msg = HandleMsg::UpdateConfig {
        owner: Some(HumanAddr::from("owner1".to_string())),
        period_prize: None,
    };
    let res = handle(&mut deps, env, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // Check owner has changed
    let res = query(&deps, QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();

    assert_eq!(HumanAddr::from("owner1"), config_response.owner);

    // update period_prize
    let env = mock_env("owner1", &[]);
    let msg = HandleMsg::UpdateConfig {
        owner: None,
        period_prize: Some(23u64),
    };

    let res = handle(&mut deps, env, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // check period_prize has changed
    let res = query(&deps, QueryMsg::Config {}).unwrap();
    let config_response: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!(config_response.period_prize, 23u64);

    // check only owner can update config
    let env = mock_env("owner", &[]);
    let msg = HandleMsg::UpdateConfig {
        owner: None,
        period_prize: Some(24u64),
    };

    let res = handle(&mut deps, env, msg);
    match res {
        Err(StdError::Unauthorized { .. }) => {}
        _ => panic!("Must return unauthorized error"),
    }
}

#[test]
fn deposit_stable() {
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
        b_terra_code_id: 123u64,
        period_prize: 69u64,
        ticket_exchange_rate: Decimal256::one(),
    };

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = init(&mut deps, env.clone(), msg).unwrap();
    // Register share barbell token contract
    let msg = HandleMsg::RegisterSTerra {};
    let env = mock_env("BT-uusd", &[]);
    let _res = handle(&mut deps, env, msg).unwrap();

    // Must deposit stable_denom
    let msg = HandleMsg::DepositStable {};
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "ukrw".to_string(),
            amount: Uint128::from(123u128),
        }],
    );

    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "Deposit amount must be greater than 0 uusd")
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    // base denom, zero deposit
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

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(1000000u128),
        }],
    );

    deps.querier.with_token_balances(&[(
        &HumanAddr::from("BT-uusd"),
        &[(
            &HumanAddr::from(MOCK_CONTRACT_ADDR),
            &Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        )],
    )]);

    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT + 1000000u128),
        }],
    );

    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    assert_eq!(
        res.log,
        vec![
            log("action", "deposit_stable"),
            log("depositor", "addr0000"),
            log("mint_amount", "1000000"),
            log("deposit_amount", "1000000"),
        ]
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("BT-uusd"),
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: HumanAddr::from("addr0000"),
                amount: Uint128::from(1000000u128),
            })
            .unwrap(),
        })]
    );

    let mut config: Config = read_config(&deps.storage).unwrap();

    // Change ticket_exchange_rate to 2 tickets per UST deposited
    config.ticket_exchange_rate = Decimal256::one() + Decimal256::one(); //TODO:lol
    store_config(&mut deps.storage, &config).unwrap();

    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    assert_eq!(
        res.log,
        vec![
            log("action", "deposit_stable"),
            log("depositor", "addr0000"),
            log("mint_amount", "2000000"),
            log("deposit_amount", "1000000"),
        ]
    );

    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("BT-uusd"),
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: HumanAddr::from("addr0000"),
                amount: Uint128::from(2000000u128),
            })
            .unwrap(),
        })]
    );

    // Todo: Check here the global state of the contract (i.e. total_reserves, tickets, etc)
}

#[test]
fn redeem_stable() {
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
        b_terra_code_id: 123u64,
        period_prize: 69u64,
        ticket_exchange_rate: Decimal256::one(),
    };

    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128(INITIAL_DEPOSIT_AMOUNT),
        }],
    );

    let _res = init(&mut deps, env.clone(), msg).unwrap();
    // Register share barbell token contract
    let msg = HandleMsg::RegisterSTerra {};
    let env = mock_env("BT-uusd", &[]);
    let _res = handle(&mut deps, env, msg).unwrap();

    // Deposit stable
    let msg = HandleMsg::DepositStable {};
    let env = mock_env(
        "addr0000",
        &[Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(1000000u128),
        }],
    );

    deps.querier.with_token_balances(&[(
        &HumanAddr::from("BT-uusd"),
        &[(
            &HumanAddr::from(MOCK_CONTRACT_ADDR),
            &Uint128::from(INITIAL_DEPOSIT_AMOUNT),
        )],
    )]);

    deps.querier.update_balance(
        HumanAddr::from(MOCK_CONTRACT_ADDR),
        vec![Coin {
            denom: "uusd".to_string(),
            amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT + 1000000u128),
        }],
    );

    let _res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

    deps.querier.with_token_balances(&[(
        &HumanAddr::from("BT-uusd"),
        &[(
            &HumanAddr::from(MOCK_CONTRACT_ADDR),
            &Uint128::from(2000000u128),
        )],
    )]);

    // Redeem 1000000
    let msg = HandleMsg::Receive(Cw20ReceiveMsg {
        sender: HumanAddr::from("addr0000"),
        amount: Uint128::from(1000000u128),
        msg: Some(to_binary(&Cw20HookMsg::RedeemStable {}).unwrap()),
    });
    let env = mock_env("addr0000", &[]);

    // Can't call redeem function directly (?)
    let res = handle(&mut deps, env, msg.clone());
    match res {
        Err(StdError::Unauthorized { .. }) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Call through sUST contract
    let env = mock_env("BT-uusd", &[]);
    let res = handle(&mut deps, env.clone(), msg.clone()).unwrap();
    assert_eq!(
        res.messages,
        vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("BT-uusd"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Burn {
                    amount: Uint128::from(1000000u128),
                })
                .unwrap()
            }),
            CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                to_address: HumanAddr::from("addr0000"),
                amount: vec![Coin {
                    denom: "uusd".to_string(),
                    amount: Uint128::from(1000000u128),
                }]
            })
        ]
    );

    // TODO: Test with changes in ticket_exchange_rate
}

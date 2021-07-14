use crate::contract::{handle, init, query};

use cosmwasm_std::testing::{mock_dependencies, mock_env};
use cosmwasm_std::{from_binary, to_binary, CosmosMsg, HumanAddr, StdError, Uint128, WasmMsg};
use cw20::Cw20HandleMsg;
use glow_protocol::distributor::{ConfigResponse, HandleMsg, InitMsg, QueryMsg};

#[test]
fn proper_initialization() {
    let mut deps = mock_dependencies(20, &[]);

    let msg = InitMsg {
        gov_contract: HumanAddr("gov".to_string()),
        glow_token: HumanAddr("glow".to_string()),
        whitelist: vec![
            HumanAddr::from("addr1"),
            HumanAddr::from("addr2"),
            HumanAddr::from("addr3"),
        ],
        spend_limit: Uint128::from(1000000u128),
        emission_cap: Default::default(),
        emission_floor: Default::default(),
        increment_multiplier: Default::default(),
        decrement_multiplier: Default::default(),
    };

    let env = mock_env("addr0000", &[]);

    // we can just call .unwrap() to assert this was a success
    let _res = init(&mut deps, env, msg).unwrap();

    // it worked, let's query the state
    let config: ConfigResponse = from_binary(&query(&deps, QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!("gov", config.gov_contract.as_str());
    assert_eq!("glow", config.glow_token.as_str());
    assert_eq!(
        vec![
            HumanAddr::from("addr1"),
            HumanAddr::from("addr2"),
            HumanAddr::from("addr3"),
        ],
        config.whitelist
    );
    assert_eq!(Uint128::from(1000000u128), config.spend_limit);
}

#[test]
fn update_config() {
    let mut deps = mock_dependencies(20, &[]);

    let msg = InitMsg {
        gov_contract: HumanAddr("gov".to_string()),
        glow_token: HumanAddr("glow".to_string()),
        whitelist: vec![
            HumanAddr::from("addr1"),
            HumanAddr::from("addr2"),
            HumanAddr::from("addr3"),
        ],
        spend_limit: Uint128::from(1000000u128),
        emission_cap: Default::default(),
        emission_floor: Default::default(),
        increment_multiplier: Default::default(),
        decrement_multiplier: Default::default(),
    };

    let env = mock_env("addr0000", &[]);

    // we can just call .unwrap() to assert this was a success
    let _res = init(&mut deps, env, msg).unwrap();

    // it worked, let's query the state
    let config: ConfigResponse = from_binary(&query(&deps, QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!("gov", config.gov_contract.as_str());
    assert_eq!("glow", config.glow_token.as_str());
    assert_eq!(
        vec![
            HumanAddr::from("addr1"),
            HumanAddr::from("addr2"),
            HumanAddr::from("addr3"),
        ],
        config.whitelist
    );
    assert_eq!(Uint128::from(1000000u128), config.spend_limit);

    let msg = HandleMsg::UpdateConfig {
        spend_limit: Some(Uint128::from(500000u128)),
        emission_cap: None,
        emission_floor: None,
        increment_multiplier: None,
        decrement_multiplier: None,
    };
    let env = mock_env("addr0000", &[]);
    let res = handle(&mut deps, env, msg.clone());

    match res {
        Err(StdError::Unauthorized { .. }) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    let env = mock_env("gov", &[]);
    let _res = handle(&mut deps, env, msg).unwrap();
    let config: ConfigResponse = from_binary(&query(&deps, QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(
        config,
        ConfigResponse {
            gov_contract: HumanAddr::from("gov"),
            glow_token: HumanAddr::from("glow"),
            whitelist: vec![
                HumanAddr::from("addr1"),
                HumanAddr::from("addr2"),
                HumanAddr::from("addr3"),
            ],
            spend_limit: Uint128::from(500000u128),
            emission_cap: Default::default(),
            emission_floor: Default::default(),
            increment_multiplier: Default::default(),
            decrement_multiplier: Default::default()
        }
    );
}

#[test]
fn test_add_remove_distributor() {
    let mut deps = mock_dependencies(20, &[]);

    let msg = InitMsg {
        gov_contract: HumanAddr("gov".to_string()),
        glow_token: HumanAddr("glow".to_string()),
        whitelist: vec![
            HumanAddr::from("addr1"),
            HumanAddr::from("addr2"),
            HumanAddr::from("addr3"),
        ],
        spend_limit: Uint128::from(1000000u128),
        emission_cap: Default::default(),
        emission_floor: Default::default(),
        increment_multiplier: Default::default(),
        decrement_multiplier: Default::default(),
    };

    let env = mock_env("addr0000", &[]);

    // we can just call .unwrap() to assert this was a success
    let _res = init(&mut deps, env, msg).unwrap();

    // Permission check AddDistributor
    let env = mock_env("addr0000", &[]);
    let msg = HandleMsg::AddDistributor {
        distributor: HumanAddr::from("addr4"),
    };

    let res = handle(&mut deps, env, msg);
    match res {
        Err(StdError::Unauthorized { .. }) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Permission check RemoveDistributor
    let env = mock_env("addr0000", &[]);
    let msg = HandleMsg::RemoveDistributor {
        distributor: HumanAddr::from("addr4"),
    };

    let res = handle(&mut deps, env, msg);
    match res {
        Err(StdError::Unauthorized { .. }) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // AddDistributor
    let env = mock_env("gov", &[]);
    let msg = HandleMsg::AddDistributor {
        distributor: HumanAddr::from("addr4"),
    };

    let _res = handle(&mut deps, env, msg).unwrap();
    let config: ConfigResponse = from_binary(&query(&deps, QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(
        config,
        ConfigResponse {
            gov_contract: HumanAddr::from("gov"),
            glow_token: HumanAddr::from("glow"),
            whitelist: vec![
                HumanAddr::from("addr1"),
                HumanAddr::from("addr2"),
                HumanAddr::from("addr3"),
                HumanAddr::from("addr4"),
            ],
            spend_limit: Uint128::from(1000000u128),
            emission_cap: Default::default(),
            emission_floor: Default::default(),
            increment_multiplier: Default::default(),
            decrement_multiplier: Default::default()
        }
    );

    // RemoveDistributor
    let env = mock_env("gov", &[]);
    let msg = HandleMsg::RemoveDistributor {
        distributor: HumanAddr::from("addr1"),
    };

    let _res = handle(&mut deps, env, msg).unwrap();
    let config: ConfigResponse = from_binary(&query(&deps, QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(
        config,
        ConfigResponse {
            gov_contract: HumanAddr::from("gov"),
            glow_token: HumanAddr::from("glow"),
            whitelist: vec![
                HumanAddr::from("addr2"),
                HumanAddr::from("addr3"),
                HumanAddr::from("addr4"),
            ],
            spend_limit: Uint128::from(1000000u128),
            emission_cap: Default::default(),
            emission_floor: Default::default(),
            increment_multiplier: Default::default(),
            decrement_multiplier: Default::default()
        }
    );
}

#[test]
fn test_spend() {
    let mut deps = mock_dependencies(20, &[]);

    let msg = InitMsg {
        gov_contract: HumanAddr("gov".to_string()),
        glow_token: HumanAddr("glow".to_string()),
        whitelist: vec![
            HumanAddr::from("addr1"),
            HumanAddr::from("addr2"),
            HumanAddr::from("addr3"),
        ],
        spend_limit: Uint128::from(1000000u128),
        emission_cap: Default::default(),
        emission_floor: Default::default(),
        increment_multiplier: Default::default(),
        decrement_multiplier: Default::default(),
    };

    let env = mock_env("addr0000", &[]);

    // we can just call .unwrap() to assert this was a success
    let _res = init(&mut deps, env, msg).unwrap();

    // permission failed
    let msg = HandleMsg::Spend {
        recipient: HumanAddr::from("addr0000"),
        amount: Uint128::from(1000000u128),
    };

    let env = mock_env("addr0000", &[]);
    let res = handle(&mut deps, env, msg);
    match res {
        Err(StdError::Unauthorized { .. }) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // failed due to spend limit
    let msg = HandleMsg::Spend {
        recipient: HumanAddr::from("addr0000"),
        amount: Uint128::from(2000000u128),
    };

    let env = mock_env("addr1", &[]);
    let res = handle(&mut deps, env, msg);
    match res {
        Err(StdError::GenericErr { msg, .. }) => {
            assert_eq!(msg, "Cannot spend more than spend_limit")
        }
        _ => panic!("DO NOT ENTER HERE"),
    }

    let msg = HandleMsg::Spend {
        recipient: HumanAddr::from("addr0000"),
        amount: Uint128::from(1000000u128),
    };

    let env = mock_env("addr2", &[]);
    let res = handle(&mut deps, env, msg).unwrap();
    assert_eq!(
        res.messages,
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: HumanAddr::from("glow"),
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Transfer {
                recipient: HumanAddr::from("addr0000"),
                amount: Uint128::from(1000000u128),
            })
            .unwrap(),
        })]
    );
}

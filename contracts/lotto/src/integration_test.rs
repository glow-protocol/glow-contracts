#![cfg(test)]

use crate::anchor_mock::{contract_anchor_mock, set_aust_addr, MockInstantiateMsg};
use cosmwasm_std::testing::{mock_env, MockApi, MockQuerier, MockStorage};
use cosmwasm_std::{coins, Addr, Coin, Empty, Uint128};
use cw20::{Cw20Coin, MinterResponse};
use terra_multi_test::{App, BankKeeper, Contract, ContractWrapper, Executor, TerraMockQuerier};

use crate::contract::{
    execute as lotto_execute, instantiate as lotto_instantiate, query as lotto_query,
    INITIAL_DEPOSIT_AMOUNT,
};

const DENOM: &str = "uusd";

fn mock_app() -> App {
    let env = mock_env();
    let api = MockApi::default();
    let bank = BankKeeper::new();

    let terra_mock_querier = TerraMockQuerier::new(MockQuerier::new(&[]));
    App::new(api, env.block, bank, MockStorage::new(), terra_mock_querier)
}

pub fn contract_lotto() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(lotto_execute, lotto_instantiate, lotto_query);

    Box::new(contract)
}

pub fn contract_cw20() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    );
    Box::new(contract)
}

#[test]
// Instantiate GLOW token
fn instantiate_glow_token() {
    let mut app = mock_app();

    // set personal balance
    let owner = Addr::unchecked("owner");
    let init_funds = coins(100000000000, "uusd");
    app.init_bank_balance(&owner, init_funds).unwrap();

    // set up cw20 contract with some tokens
    let cw20_id = app.store_code(contract_cw20());
    let msg = cw20_base::msg::InstantiateMsg {
        name: "Glow Token".to_string(),
        symbol: "GLOW".to_string(),
        decimals: 2,
        initial_balances: vec![Cw20Coin {
            address: owner.to_string(),
            amount: Uint128::new(INITIAL_DEPOSIT_AMOUNT),
        }],
        mint: None,
        marketing: None,
    };
    let _cash_addr = app
        .instantiate_contract(cw20_id, owner, &msg, &[], "CASH", None)
        .unwrap();
}

#[test]
// Instantiate GLOW Lotto
fn instantiate_glow_lotto() {
    let mut app = mock_app();

    // set personal balance
    let owner = Addr::unchecked("owner");
    let init_funds = coins(10000000000, "uusd");
    app.init_bank_balance(&owner, init_funds).unwrap();

    let msg = MockInstantiateMsg {};

    // set up anchor
    let anchor_id = app.store_code(contract_anchor_mock());
    let anchor_address = app
        .instantiate_contract(
            anchor_id,
            owner.clone(),
            &msg,
            &[Coin {
                denom: DENOM.to_string(),
                amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
            }],
            "CORE",
            None,
        )
        .unwrap();

    // set up cw20 contract with some tokens
    let cw20_id = app.store_code(contract_cw20());
    let msg = cw20_base::msg::InstantiateMsg {
        name: "Anchor Token".to_string(),
        symbol: "AUST".to_string(),
        decimals: 2,
        initial_balances: vec![Cw20Coin {
            address: owner.to_string(),
            amount: Uint128::new(INITIAL_DEPOSIT_AMOUNT),
        }],
        mint: Some(MinterResponse {
            minter: anchor_address.to_string(),
            cap: None,
        }),
        marketing: None,
    };
    let aust_addr = app
        .instantiate_contract(cw20_id, owner.clone(), &msg, &[], "CASH", None)
        .unwrap();

    set_aust_addr(aust_addr.to_string());

    // set up glow lotto
    let lotto_id = app.store_code(contract_lotto());
    let mut msg = crate::tests::instantiate_msg();

    msg.anchor_contract = anchor_address.to_string();
    println!("{:?}", msg);

    // throwing errors because we need to add support for Anchor
    let _lotto_addr = app
        .instantiate_contract(
            lotto_id,
            owner,
            &msg,
            &[Coin {
                denom: DENOM.to_string(),
                amount: Uint128::from(INITIAL_DEPOSIT_AMOUNT),
            }],
            "CORE",
            None,
        )
        .unwrap();
}

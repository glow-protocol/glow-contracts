#![cfg(test)]

use cosmwasm_std::testing::{mock_env, MockApi, MockStorage};
use cosmwasm_std::{coins, Addr, Coin, Empty, Uint128};
use cw20::Cw20Coin;
use cw_multi_test::{App, BankKeeper, Contract, ContractWrapper, Executor};

use crate::contract::{
    execute as core_execute, instantiate as core_instantiate, query as core_query,
    INITIAL_DEPOSIT_AMOUNT,
};

const DENOM: &str = "uusd";

fn mock_app() -> App {
    let env = mock_env();
    let api = MockApi::default();
    let bank = BankKeeper::new();

    App::new(api, env.block, bank, MockStorage::new())
}

pub fn contract_core() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(core_execute, core_instantiate, core_query);

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
        .instantiate_contract(cw20_id, owner.clone(), &msg, &[], "CASH", None)
        .unwrap();
}

#[test]
// Instantiate GLOW core
fn instantiate_glow_core() {
    let mut app = mock_app();

    // set personal balance
    let owner = Addr::unchecked("owner");
    let init_funds = coins(10000000000, "uusd");
    app.init_bank_balance(&owner, init_funds).unwrap();

    // set up cw20 contract with some tokens
    let core_id = app.store_code(contract_core());
    let msg = crate::tests::instantiate_msg();

    let _core_addr = app
        .instantiate_contract(
            core_id,
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
}

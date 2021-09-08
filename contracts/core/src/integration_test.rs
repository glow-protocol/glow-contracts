#![cfg(test)]

use cosmwasm_std::testing::{mock_env, MockApi, MockStorage};
use cosmwasm_std::{coins, to_binary, Addr, Coin, Empty, Uint128};
use cw20::{Cw20Coin, Cw20Contract, Cw20ExecuteMsg};
use cw_multi_test::{App, BankKeeper, Contract, ContractWrapper, Executor};

use glow_protocol::core::{
    ConfigResponse as CoreConfigResponse, DepositorInfoResponse, DepositorsInfoResponse,
    ExecuteMsg as CoreMsg, InstantiateMsg as CoreInstantiate, LotteryInfoResponse,
    QueryMsg as CoreQuery, StateResponse as CoreStateResponse,
};

use crate::contract::{
    execute as core_execute, instantiate as core_instantiate, query as core_query,
    INITIAL_DEPOSIT_AMOUNT,
};
use cosmwasm_bignumber::Decimal256;

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
    let cash_addr = app
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

    let core_addr = app
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

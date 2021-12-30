#![cfg(test)]

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, from_binary, to_binary, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, Empty,
    QueryRequest, Response, StdError, StdResult, Uint128, WasmMsg, WasmQuery,
};

use cw20::Cw20ExecuteMsg;
use cw20::Cw20ReceiveMsg;
use lazy_static::lazy_static;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use terra_multi_test::{Contract, ContractWrapper};

// This lazy static use allows you the dev to set the aust token addr before you use the anchor mock so that you can mock out AUST as needed.
lazy_static! {

    // This lazily made static uses a ReadWrite lock to ensure some form of safety on setting/getting values and means you dont need to wrap the code in an unsafe block which looks icky
    static ref AUST_ADDR_MOCK: RwLock<String> = RwLock::new("anchor".to_string());
}

// Acquire a write lock on the static value and then update it
pub fn set_aust_addr(new_addr: String) -> String {
    let mut addr = AUST_ADDR_MOCK.write().unwrap();
    *addr = new_addr;
    addr.to_string()
}

pub fn get_aust_addr() -> String {
    return AUST_ADDR_MOCK.read().unwrap().to_string();
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct MockInstantiateMsg {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct PingMsg {
    pub payload: String,
}

// Slimmed down mock ExecuteMsg with an example Receive, deposit and withdraw (redeem)
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MockExecuteMsg {
    Receive(Cw20ReceiveMsg),
    DepositStable {},
    RedeemStable { burn_amount: Uint128 },
}

// A quick AnchorQuery struct which hold only the EpochState query. This could be expanded to have all anchor mocks
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnchorQuery {
    EpochState {
        block_height: Option<u64>,
        distributed_interest: Option<Uint256>,
    },
}

// The response that should be returned by the EpochState query
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct EpochStateResponse {
    pub exchange_rate: Decimal256,
    pub aterra_supply: Uint256,
}

// Helper method used to perform a query for EpochState and then returns the aust exchange rate for a given block
#[allow(dead_code)]
pub fn query_aust_exchange_rate(
    deps: Deps,
    anchor_money_market_address: String,
) -> StdResult<Decimal> {
    let response: EpochStateResponse =
        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: anchor_money_market_address,
            msg: to_binary(&AnchorQuery::EpochState {
                block_height: None,
                distributed_interest: None,
            })?,
        }))?;
    Ok(Decimal::from(response.exchange_rate))
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cw20HookMsg {
    /// Return stable coins to a user
    /// according to exchange rate
    RedeemStable {},
}

pub fn contract_anchor_mock() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        |deps, _, info, msg: MockExecuteMsg| -> StdResult<Response> {
            match msg {
                MockExecuteMsg::Receive(Cw20ReceiveMsg {
                    sender: _,
                    amount,
                    msg,
                }) => match from_binary(&msg) {
                    Ok(Cw20HookMsg::RedeemStable {}) => {
                        let redeem_amount = Uint256::from(amount) * Decimal256::percent(120);
                        Ok(Response::new()
                            .add_messages(vec![
                                CosmosMsg::Wasm(WasmMsg::Execute {
                                    contract_addr: deps
                                        .api
                                        .addr_humanize(
                                            &deps.api.addr_canonicalize(&get_aust_addr())?,
                                        )?
                                        .to_string(),
                                    funds: vec![],
                                    msg: to_binary(&Cw20ExecuteMsg::Burn { amount })?,
                                }),
                                CosmosMsg::Bank(BankMsg::Send {
                                    to_address: info.sender.to_string(),
                                    amount: vec![Coin {
                                        denom: "uusd".to_string(),
                                        amount: redeem_amount.into(),
                                    }],
                                }),
                            ])
                            .add_attributes(vec![
                                attr("action", "redeem_stable"),
                                attr("burn_amount", amount),
                                attr("redeem_amount", redeem_amount),
                            ]))
                    }
                    _ => Err(StdError::generic_err("Unauthorized")),
                },
                MockExecuteMsg::DepositStable {} => {
                    // Check base denom deposit
                    let deposit_amount: Uint256 = info
                        .funds
                        .iter()
                        .find(|c| c.denom == *"uusd")
                        .map(|c| Uint256::from(c.amount))
                        .unwrap_or_else(Uint256::zero);
                    // Get Mint amount
                    let mint_amount = deposit_amount / Decimal256::percent(120);
                    // Perform a mint from the contract
                    Ok(Response::new()
                        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
                            contract_addr: deps
                                .api
                                .addr_humanize(&deps.api.addr_canonicalize(&get_aust_addr())?)?
                                .to_string(),
                            funds: vec![],
                            msg: to_binary(&Cw20ExecuteMsg::Mint {
                                recipient: info.sender.to_string(),
                                amount: mint_amount.into(),
                            })?,
                        }))
                        .add_attributes(vec![
                            attr("action", "deposit_stable"),
                            attr("depositor", info.sender),
                            attr("mint_amount", mint_amount),
                            attr("deposit_amount", deposit_amount),
                        ]))
                }
                MockExecuteMsg::RedeemStable { burn_amount } => {
                    let redeem_amount = Uint256::from(burn_amount) * Decimal256::percent(120);
                    Ok(Response::new()
                        .add_messages(vec![
                            CosmosMsg::Wasm(WasmMsg::Execute {
                                contract_addr: deps
                                    .api
                                    .addr_humanize(&deps.api.addr_canonicalize(&get_aust_addr())?)?
                                    .to_string(),
                                funds: vec![],
                                msg: to_binary(&Cw20ExecuteMsg::Burn {
                                    amount: burn_amount,
                                })?,
                            }),
                            CosmosMsg::Bank(BankMsg::Send {
                                to_address: info.sender.to_string(),
                                amount: vec![Coin {
                                    denom: "uusd".to_string(),
                                    amount: redeem_amount.into(),
                                }],
                            }),
                        ])
                        .add_attributes(vec![
                            attr("action", "redeem_stable"),
                            attr("burn_amount", burn_amount),
                            attr("redeem_amount", redeem_amount),
                        ]))
                }
            }
        },
        |_, _, _, _: MockInstantiateMsg| -> StdResult<Response> { Ok(Response::default()) },
        |_, _, msg: AnchorQuery| -> StdResult<Binary> {
            match msg {
                AnchorQuery::EpochState {
                    distributed_interest: _,
                    block_height: _,
                } => Ok(to_binary(&mock_epoch_state())?),
            }
        },
    );
    Box::new(contract)
}

pub fn mock_epoch_state() -> EpochStateResponse {
    let epoch_state: EpochStateResponse = EpochStateResponse {
        exchange_rate: Decimal256::percent(120),
        aterra_supply: Uint256::from(1000000u64),
    };
    epoch_state
}

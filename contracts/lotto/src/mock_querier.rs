use glow_protocol::ve_token::{StakerResponse, StateResponse};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage};
use cosmwasm_std::{
    from_binary, from_slice, to_binary, Addr, Binary, BlockInfo, Coin, ContractInfo,
    ContractResult, Decimal, Env, MessageInfo, OwnedDeps, Querier, QuerierResult, QueryRequest,
    SystemError, SystemResult, Timestamp, Uint128, WasmQuery,
};
use cw20::{BalanceResponse as Cw20BalanceResponse, Cw20QueryMsg};
use terra_cosmwasm::{TaxCapResponse, TaxRateResponse, TerraQuery, TerraQueryWrapper, TerraRoute};

use cosmwasm_bignumber::{Decimal256, Uint256};
use glow_protocol::distributor::GlowEmissionRateResponse;
use moneymarket::market::EpochStateResponse;
use std::collections::HashMap;

use crate::tests::RATE;

use crate::oracle::OracleResponse;

pub const MOCK_CONTRACT_ADDR: &str = "cosmos2contract";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Query Epoch State to Anchor money market
    EpochState {
        block_height: Option<u64>,
        distributed_interest: Option<Uint256>,
    },

    /// Query GLOW emission rate to distributor model contract
    GlowEmissionRate {
        current_award: Decimal256,
        target_award: Decimal256,
        current_emission_rate: Decimal256,
    },

    Balance {
        address: String,
    },

    State {
        timestamp: Option<u64>,
    },

    Staker {
        address: String,
        timestamp: Option<u64>,
    },

    GetRandomness {
        round: u64,
    },
}

/// mock_dependencies is a drop-in replacement for cosmwasm_std::testing::mock_dependencies
/// this uses our CustomQuerier.
pub fn mock_dependencies(
    contract_balance: &[Coin],
) -> OwnedDeps<MockStorage, MockApi, WasmMockQuerier> {
    let mut custom_querier: WasmMockQuerier =
        WasmMockQuerier::new(MockQuerier::new(&[(MOCK_CONTRACT_ADDR, contract_balance)]));

    // Mock aUST-UST exchange rate
    custom_querier.with_exchange_rate(Decimal256::permille(RATE));

    OwnedDeps {
        storage: MockStorage::default(),
        api: MockApi::default(),
        querier: custom_querier,
    }
}

/// mock_env is a drop-in replacement for cosmwasm_std::testing::mock_env
pub fn mock_env() -> Env {
    Env {
        block: BlockInfo {
            height: 12_345,
            time: Timestamp::from_seconds(1_595_431_050),
            chain_id: "cosmos-testnet-14002".to_string(),
        },
        contract: ContractInfo {
            address: Addr::unchecked(MOCK_CONTRACT_ADDR),
        },
    }
}

/// mock_info is a drop-in replacement for cosmwasm_std::testing::mock_info
pub fn mock_info(sender: &str, funds: &[Coin]) -> MessageInfo {
    MessageInfo {
        sender: Addr::unchecked(sender),
        funds: funds.to_vec(),
    }
}

pub struct WasmMockQuerier {
    base: MockQuerier<TerraQueryWrapper>,
    token_querier: TokenQuerier,
    tax_querier: TaxQuerier,
    exchange_rate_querier: ExchangeRateQuerier,
    emission_rate_querier: EmissionRateQuerier, //TODO: use in tests and replace _ for EmissionRateQuerier
}

#[derive(Clone, Default)]
pub struct TokenQuerier {
    // this lets us iterate over all pairs that match the first string
    balances: HashMap<String, HashMap<String, Uint128>>,
}

impl TokenQuerier {
    pub fn new(balances: &[(&String, &[(&String, &Uint128)])]) -> Self {
        TokenQuerier {
            balances: balances_to_map(balances),
        }
    }
}

pub(crate) fn balances_to_map(
    balances: &[(&String, &[(&String, &Uint128)])],
) -> HashMap<String, HashMap<String, Uint128>> {
    let mut balances_map: HashMap<String, HashMap<String, Uint128>> = HashMap::new();
    for (contract_addr, balances) in balances.iter() {
        let mut contract_balances_map: HashMap<String, Uint128> = HashMap::new();
        for (addr, balance) in balances.iter() {
            contract_balances_map.insert(addr.to_string(), **balance);
        }

        balances_map.insert(contract_addr.to_string(), contract_balances_map);
    }
    balances_map
}

#[derive(Clone, Default)]
pub struct TaxQuerier {
    rate: Decimal,
    // this lets us iterate over all pairs that match the first string
    caps: HashMap<String, Uint128>,
}

impl TaxQuerier {
    pub fn new(rate: Decimal, caps: &[(&String, &Uint128)]) -> Self {
        TaxQuerier {
            rate,
            caps: caps_to_map(caps),
        }
    }
}

pub(crate) fn caps_to_map(caps: &[(&String, &Uint128)]) -> HashMap<String, Uint128> {
    let mut owner_map: HashMap<String, Uint128> = HashMap::new();
    for (denom, cap) in caps.iter() {
        owner_map.insert(denom.to_string(), **cap);
    }
    owner_map
}

#[derive(Clone, Default)]
pub struct ExchangeRateQuerier {
    exchange_rate: Decimal256,
}

impl ExchangeRateQuerier {
    pub fn new(exchange_rate: Decimal256) -> Self {
        ExchangeRateQuerier { exchange_rate }
    }
}

#[derive(Clone, Default)]
#[allow(dead_code)] // TODO: use this fn in tests
pub struct EmissionRateQuerier {
    emission_rate: Decimal256,
}

impl EmissionRateQuerier {
    #[allow(dead_code)] // TODO: use this fn in tests
    pub fn new(emission_rate: Decimal256) -> Self {
        EmissionRateQuerier { emission_rate }
    }
}

impl Querier for WasmMockQuerier {
    fn raw_query(&self, bin_request: &[u8]) -> QuerierResult {
        // MockQuerier doesn't support Custom, so we ignore it completely here
        let request: QueryRequest<TerraQueryWrapper> = match from_slice(bin_request) {
            Ok(v) => v,
            Err(e) => {
                return SystemResult::Err(SystemError::InvalidRequest {
                    error: format!("Parsing query request: {}", e),
                    request: bin_request.into(),
                })
            }
        };
        self.handle_query(&request)
    }
}

impl WasmMockQuerier {
    pub fn handle_query(&self, request: &QueryRequest<TerraQueryWrapper>) -> QuerierResult {
        match &request {
            QueryRequest::Custom(TerraQueryWrapper { route, query_data }) => {
                if route == &TerraRoute::Treasury {
                    match query_data {
                        TerraQuery::TaxRate {} => {
                            let res = TaxRateResponse {
                                rate: self.tax_querier.rate,
                            };
                            SystemResult::Ok(ContractResult::from(to_binary(&res)))
                        }
                        TerraQuery::TaxCap { denom } => {
                            let cap = self
                                .tax_querier
                                .caps
                                .get(denom)
                                .copied()
                                .unwrap_or_default();
                            let res = TaxCapResponse { cap };
                            SystemResult::Ok(ContractResult::from(to_binary(&res)))
                        }
                        _ => panic!("DO NOT ENTER HERE"),
                    }
                } else {
                    panic!("DO NOT ENTER HERE")
                }
            }

            QueryRequest::Wasm(WasmQuery::Smart { contract_addr, msg }) => {
                match from_binary::<QueryMsg>(msg).unwrap() {
                    QueryMsg::EpochState {
                        block_height: _,
                        distributed_interest: _,
                    } => {
                        SystemResult::Ok(ContractResult::from(to_binary(&EpochStateResponse {
                            exchange_rate: self.exchange_rate_querier.exchange_rate, // Current anchor rate,
                            aterra_supply: Uint256::one(),
                        })))
                    }
                    // TODO: revise, currently hard-coded
                    QueryMsg::GlowEmissionRate {
                        current_award: _,
                        target_award: _,
                        current_emission_rate: _,
                    } => SystemResult::Ok(ContractResult::from(to_binary(
                        &GlowEmissionRateResponse {
                            emission_rate: Decimal256::one(),
                        },
                    ))),

                    QueryMsg::GetRandomness { round: _ } => {
                        SystemResult::Ok(ContractResult::from(to_binary(&OracleResponse {
                            randomness: Binary::from_base64(
                                "e74c6cfd99371c817e8c3e0099df9074032eec15189c49e5b4740b084ba5ce2b",
                            )
                            .unwrap(),
                            worker: Addr::unchecked(MOCK_CONTRACT_ADDR),
                        })))
                    }

                    QueryMsg::Staker { address, .. } => {
                        let balances: &HashMap<String, Uint128> =
                            match self.token_querier.balances.get(contract_addr) {
                                Some(balances) => balances,
                                None => {
                                    return SystemResult::Err(SystemError::InvalidRequest {
                                        error: format!(
                                            "No balance info exists for the contract {}",
                                            contract_addr
                                        ),
                                        request: msg.as_slice().into(),
                                    })
                                }
                            };

                        let balance = match balances.get(&address) {
                            Some(v) => *v,
                            None => {
                                return SystemResult::Ok(ContractResult::Ok(
                                    to_binary(&Cw20BalanceResponse {
                                        balance: Uint128::zero(),
                                    })
                                    .unwrap(),
                                ));
                            }
                        };

                        SystemResult::Ok(ContractResult::Ok(
                            to_binary(&StakerResponse {
                                deposited_amount: balance,
                                balance,
                                locked_amount: balance,
                            })
                            .unwrap(),
                        ))
                    }

                    QueryMsg::State { .. } => {
                        let balances: &HashMap<String, Uint128> =
                            match self.token_querier.balances.get(contract_addr) {
                                Some(balances) => balances,
                                None => {
                                    return SystemResult::Err(SystemError::InvalidRequest {
                                        error: format!(
                                            "No balance info exists for the contract {}",
                                            contract_addr
                                        ),
                                        request: msg.as_slice().into(),
                                    })
                                }
                            };

                        // Sum over the entire balance
                        let balance = balances.iter().fold(Uint128::zero(), |sum, x| sum + x.1);

                        SystemResult::Ok(ContractResult::Ok(
                            to_binary(&StateResponse {
                                total_deposited_amount: balance,
                                total_balance: balance,
                                total_locked_amount: balance,
                            })
                            .unwrap(),
                        ))
                    }

                    _ => match from_binary::<Cw20QueryMsg>(msg).unwrap() {
                        Cw20QueryMsg::Balance { address } => {
                            let balances: &HashMap<String, Uint128> =
                                match self.token_querier.balances.get(contract_addr) {
                                    Some(balances) => balances,
                                    None => {
                                        return SystemResult::Err(SystemError::InvalidRequest {
                                            error: format!(
                                                "No balance info exists for the contract {}",
                                                contract_addr
                                            ),
                                            request: msg.as_slice().into(),
                                        })
                                    }
                                };

                            let balance = match balances.get(&address) {
                                Some(v) => *v,
                                None => {
                                    return SystemResult::Ok(ContractResult::Ok(
                                        to_binary(&Cw20BalanceResponse {
                                            balance: Uint128::zero(),
                                        })
                                        .unwrap(),
                                    ));
                                }
                            };

                            SystemResult::Ok(ContractResult::Ok(
                                to_binary(&Cw20BalanceResponse { balance }).unwrap(),
                            ))
                        }

                        _ => panic!("DO NOT ENTER HERE"),
                    },
                }
            }
            _ => self.base.handle_query(request),
        }
    }
}

impl WasmMockQuerier {
    pub fn new(base: MockQuerier<TerraQueryWrapper>) -> Self {
        WasmMockQuerier {
            base,
            token_querier: TokenQuerier::default(),
            tax_querier: TaxQuerier::default(),
            exchange_rate_querier: ExchangeRateQuerier::default(),
            emission_rate_querier: EmissionRateQuerier::default(),
        }
    }

    // set a new balance for the given address and return the old balance
    pub fn update_balance<U: Into<String>>(
        &mut self,
        addr: U,
        balance: Vec<Coin>,
    ) -> Option<Vec<Coin>> {
        self.base.update_balance(addr, balance)
    }

    // configure the mint whitelist mock querier
    pub fn with_token_balances(&mut self, balances: &[(&String, &[(&String, &Uint128)])]) {
        self.token_querier = TokenQuerier::new(balances);
    }

    pub fn increment_token_balance(&mut self, address: String, token_addr: String, diff: Uint128) {
        let contract_balances_map = self
            .token_querier
            .balances
            .entry(address)
            .or_insert_with(HashMap::new);

        let balance = contract_balances_map
            .entry(token_addr)
            .or_insert_with(|| Uint128::from(0u128));

        *balance += diff;
    }

    pub fn decrement_token_balance(&mut self, address: String, token_addr: String, diff: Uint128) {
        let contract_balances_map = self
            .token_querier
            .balances
            .entry(address)
            .or_insert_with(HashMap::new);

        let balance = contract_balances_map
            .entry(token_addr)
            .or_insert_with(|| Uint128::from(0u128));

        *balance -= diff;
    }

    // configure the token owner mock querier
    pub fn with_tax(&mut self, rate: Decimal, caps: &[(&String, &Uint128)]) {
        self.tax_querier = TaxQuerier::new(rate, caps);
    }

    // configure anchor exchange rate
    pub fn with_exchange_rate(&mut self, rate: Decimal256) {
        self.exchange_rate_querier = ExchangeRateQuerier::new(rate);
    }

    // configure glow emission rate
    #[allow(dead_code)] //TODO: Use in tests
    pub fn with_emission_rate(&mut self, rate: Decimal256) {
        self.emission_rate_querier = EmissionRateQuerier::new(rate);
    }
}

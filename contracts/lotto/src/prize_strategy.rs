use crate::error::ContractError;
use crate::querier::query_balance;
use crate::state::{
    read_config, read_depositor_info, read_matching_sequences, read_state, store_depositor_info,
    store_lottery_info, store_state, LotteryInfo,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, to_binary, CanonicalAddr, Coin, CosmosMsg, DepsMut, Env, MessageInfo, Response, Uint128,
    WasmMsg,
};
use cw0::Expiration;
use cw20::Cw20ExecuteMsg::Send as Cw20Send;
use terraswap::querier::query_token_balance;

use glow_protocol::lotto::ExecuteMsg;
use moneymarket::market::{Cw20HookMsg, ExecuteMsg as AnchorMsg};
use std::collections::HashMap;
use std::ops::{Add, Sub};
use std::usize;

pub fn execute_lottery(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut state = read_state(deps.storage)?;
    let config = read_config(deps.storage)?;

    // No sent funds allowed when executing the lottery
    if !info.funds.is_empty() {
        return Err(ContractError::InvalidLotteryExecution {});
    }

    if state.next_lottery_time.is_expired(&env.block) {
        //state.next_lottery_time = state.next_lottery_time.add(config.lottery_interval)?;
        state.next_lottery_time =
            Expiration::AtTime(env.block.time).add(config.lottery_interval)?;
    } else {
        return Err(ContractError::LotteryInProgress {});
    }

    // Get contract current aUST balance
    let total_aterra_balance = query_token_balance(
        &deps.querier,
        deps.api.addr_humanize(&config.a_terra_contract)?,
        env.clone().contract.address,
    )?;

    // Get lottery related deposits of aUST
    let lottery_aterra =
        (Decimal256::from_uint256(total_aterra_balance) - state.deposit_shares) * Uint256::one();

    // Get contract current UST balance (used in _execute_prize)
    let balance = query_balance(
        deps.as_ref(),
        env.contract.address.to_string(),
        "uusd".to_string(),
    )?;

    let mut msgs: Vec<CosmosMsg> = vec![];

    if lottery_aterra.is_zero() {
        if state.award_available.is_zero() {
            return Err(ContractError::InsufficientLotteryFunds {});
        }
    } else {
        // Message for redeem amount operation of aUST
        let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps
                .api
                .addr_humanize(&config.a_terra_contract)?
                .to_string(),
            funds: vec![],
            msg: to_binary(&Cw20Send {
                contract: deps.api.addr_humanize(&config.anchor_contract)?.to_string(),
                amount: lottery_aterra.into(),
                msg: to_binary(&Cw20HookMsg::RedeemStable {})?,
            })?,
        });

        msgs.push(redeem_msg);
    }

    // Store state
    store_state(deps.storage, &state)?;

    // Prepare message for internal call to _execute_prize
    let execute_prize_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        funds: vec![],
        msg: to_binary(&ExecuteMsg::_ExecutePrize { balance })?,
    });

    msgs.push(execute_prize_msg);

    let res = Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "execute_lottery"),
        attr("redeemed_amount", lottery_aterra),
    ]);
    Ok(res)
}

pub fn _execute_prize(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    balance: Uint256,
) -> Result<Response, ContractError> {
    if info.sender != env.contract.address {
        return Err(ContractError::Unauthorized {});
    }

    let mut state = read_state(deps.storage)?;
    let config = read_config(deps.storage)?;

    // TODO: Get random sequence here
    let winning_sequence = String::from("00000");

    // Get contract current uusd balance
    let curr_balance = query_balance(
        deps.as_ref(),
        env.contract.address.to_string(),
        String::from("uusd"),
    )?;

    // Calculate outstanding_interest from the lottery aUST deposits during the lottery interval
    let outstanding_interest = if curr_balance > balance + state.lottery_deposits * Uint256::one() {
        // Redemption of lottery aUST shares is net positive
        let balance_delta = Decimal256::from_uint256(curr_balance - balance);
        // Balance delta minus than the base lottery deposits
        balance_delta - state.lottery_deposits
    } else {
        Decimal256::zero()
    };

    // Add outstanding_interest to previous available award
    state.award_available = state.award_available.add(outstanding_interest);

    let prize = state.award_available;
    if prize.is_zero() {
        return Err(ContractError::InsufficientLotteryFunds {});
    }

    // Get winners and respective sequences
    let lucky_holders: Vec<(u8, Vec<CanonicalAddr>)> =
        read_matching_sequences(deps.as_ref(), &winning_sequence);

    let mut map_winners = HashMap::new();
    for (k, mut v) in lucky_holders.clone() {
        map_winners.entry(k).or_insert_with(Vec::new).append(&mut v)
    }

    // assign prizes
    let mut total_awarded_prize = Decimal256::zero();
    let mut total_reserve_commission = Decimal256::zero();

    for (matches, winners) in map_winners.iter() {
        let number_winners = winners.len() as u64;
        for winner in winners {
            let mut depositor = read_depositor_info(deps.as_ref().storage, winner);

            let assigned =
                assign_prize(prize, *matches, number_winners, &config.prize_distribution);

            total_awarded_prize += assigned;
            let reserve_commission = apply_reserve_factor(assigned, config.reserve_factor);
            total_reserve_commission += reserve_commission;
            let amount_redeem = (assigned - reserve_commission) * Uint256::one();
            depositor.redeemable_amount = depositor
                .redeemable_amount
                .add(Uint128::from(amount_redeem));

            store_depositor_info(deps.storage, winner, &depositor)?;
        }
    }

    let lottery_info = LotteryInfo {
        sequence: winning_sequence,
        awarded: true,
        total_prizes: total_awarded_prize,
        winners: lucky_holders,
    };

    store_lottery_info(deps.storage, state.current_lottery, &lottery_info)?;

    state.current_lottery += 1;
    state.total_reserve = state.total_reserve.add(total_reserve_commission);
    // println!("state: {:?}", state);
    // println!("award_available: {}", state.award_available);
    // println!("total_awarded_prize: {}", total_awarded_prize);
    state.award_available = state.award_available.sub(total_awarded_prize);
    store_state(deps.storage, &state)?;

    let reinvest_amount = state.lottery_deposits * Uint256::one();
    let mut messages: Vec<CosmosMsg> = vec![];

    // The protocol assumes the taxes on the reinvested amount on Anchor contract
    if !reinvest_amount.is_zero() {
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.addr_humanize(&config.anchor_contract)?.to_string(),
            funds: vec![Coin {
                denom: config.stable_denom,
                amount: reinvest_amount.into(),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        }))
    }

    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("action", "execute_prize"),
        attr("accrued_interest", outstanding_interest.to_string()),
        attr("total_awarded_prize", total_awarded_prize.to_string()),
        attr("reinvested_amount", reinvest_amount),
    ]))
}

fn apply_reserve_factor(awarded_amount: Decimal256, reserve_factor: Decimal256) -> Decimal256 {
    awarded_amount * reserve_factor
}

// distribution is a vector [0, 0, 0.025, 0.15, 0.3, 0.5]
fn assign_prize(
    awardable_prize: Decimal256,
    matches: u8,
    winners: u64,
    distribution: &[Decimal256],
) -> Decimal256 {
    let number_winners = Uint256::from(winners as u64);

    // println!("distribution element {:?}", matches);

    awardable_prize * distribution[matches as usize] / Decimal256::from_uint256(number_winners)
}

pub fn is_valid_sequence(sequence: &str, len: u8) -> bool {
    sequence.len() == (len as usize) && sequence.chars().all(|c| ('0'..='9').contains(&c))
}

pub fn count_seq_matches(a: &str, b: &str) -> u8 {
    let mut count = 0;
    for (i, c) in a.chars().enumerate() {
        if c == b.chars().nth(i).unwrap() {
            count += 1;
        }
    }
    count
}

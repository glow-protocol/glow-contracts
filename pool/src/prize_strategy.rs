use itertools::Itertools;

use crate::msg::HandleMsg;
use crate::querier::query_balance;
use crate::state::{
    read_all_sequences, read_config, read_depositor_info, read_matching_sequences,
    read_sequence_info, read_state, store_config, store_depositor_info, store_lottery_info,
    store_sequence_info, store_state, Config, LotteryInfo, State,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    from_binary, log, to_binary, Api, BankMsg, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern,
    HandleResponse, HandleResult, HumanAddr, InitResponse, InitResult, MessageInfo, Querier,
    StdError, StdResult, Storage, Uint128, WasmMsg,
};
use cw20::Cw20HandleMsg::Send as Cw20Send;
use moneymarket::market::{Cw20HookMsg, HandleMsg as AnchorMsg};

pub fn execute_lottery<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> HandleResult {
    let mut state = read_state(&deps.storage)?;
    let config = read_config(&deps.storage)?;

    // No sent funds allowed when executing the lottery
    if !env.message.sent_funds.is_empty() {
        Err(StdError::generic_err(
            "Do not send funds when executing the lottery",
        ))
    };

    if state.next_lottery_time.is_expired(&env.block) {
        state.next_lottery_time += config.lottery_interval;
    } else {
        Err(StdError::generic_err(format!(
            "Lottery is still running, please check again after {}",
            state.next_lottery_time
        )));
    }

    // Get contract current uusd balance
    state.current_balance = query_balance(
        &deps,
        &deps.api.human_address(&config.contract_addr)?,
        String::from("uusd"),
    )?;

    // Get lottery deposits of aUST
    // TODO: is it better to query_token_balance instead of track it with state variables??
    let lottery_deposits = state.total_deposits * config.split_factor;

    // Store state
    store_state(&mut deps.storage, &state)?;

    // Message for redeem amount operation of aUST
    let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps.api.human_address(&config.a_terra_contract)?,
        send: vec![],
        msg: to_binary(&Cw20Send {
            contract: deps.api.human_address(&config.a_terra_contract)?,
            amount: Uint128::from(lottery_deposits),
            msg: Some(to_binary(&Cw20HookMsg::RedeemStable {})?),
        })?,
    });

    // Prepare message for internal call to _handle_prize
    let handle_prize_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps.api.human_address(&config.contract_addr)?,
        send: vec![],
        msg: to_binary(&HandleMsg::HandlePrize {})?,
    })?;

    //Handle Response withdraws from Anchor and call internal _handle_prize
    Ok(HandleResponse {
        messages: vec![redeem_msg, handle_prize_msg],
        log: vec![log("action", "execute_lottery")],
        data: None,
    })
}

pub fn _handle_prize(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    info: MessageInfo, //TODO: no sure if needed
) -> HandleResult {
    if info.sender != env.contract.address {
        return Err(StdError::unauthorized());
    }

    let mut state = read_state(&deps.storage)?;
    let config = read_config(&deps.storage)?;

    // TODO: Get random sequence here
    // TODO: deduct terra taxes and oracle fees
    let winning_sequence = String::from("34280");

    // Get contract current uusd balance
    let curr_balance = query_balance(
        &deps,
        &deps.api.human_address(&config.contract_addr)?,
        String::from("uusd"),
    )?;

    // Get delta after aUST redeem operation
    let balance_delta = (curr_balance - state.current_balance)?;

    // Minus total_lottery_deposits and we get outstanding_interest
    let outstanding_interest = (balance_delta - state.lottery_deposits)?;

    // Add outstanding_interest to previous available award
    state.award_available += outstanding_interest;

    let prize = state.award_available;

    // Get winners and respective sequences
    let lucky_holders: Vec<(u8, CanonicalAddr)> =
        read_matching_sequences(&deps, None, None, &winning_sequence)?;

    let map_winners = lucky_holders.into_iter().into_group_map();

    // assign prizes
    let mut total_awarded_prize = Decimal256::zero();
    let mut total_reserve_commission = Decimal256::zero();

    for (matches, winners) in map_winners.iter() {
        let number_winners = winners.len() as u8;
        for winner in winners {
            let mut depositor = read_depositor_info(&deps.storage, winner);

            let assigned = assign_prize(prize, number_winners, config.prize_distribution[matches]);

            total_awarded_prize += assigned;
            let reserve_commission = apply_reserve_factor(assigned, config.reserve_factor)?;
            total_reserve_commission += reserve_commission;
            depositor.redeemable_amount += (assigned - reserve_commission)?;

            store_depositor_info(&mut deps.storage, winner, &depositor)?;
        }
    }

    let lottery_info = LotteryInfo {
        sequence: Decimal256::from_uint256(winning_sequence),
        awarded: true,
        total_prizes: total_awarded_prize,
        winners: lucky_holders,
    };

    store_lottery_info(&mut deps.storage, state.current_lottery, &lottery_info)?;

    state.next_lottery_time += config.lottery_interval;
    state.current_lottery += Uint256::one();
    state.total_reserve += total_reserve_commission;
    state.award_available -= total_awarded_prize;
    //TODO: update total_assets and spendable_balance??
    store_state(&mut deps.storage, &state)?;

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.anchor_contract)?,
            send: vec![Coin {
                denom: config.stable_denom,
                amount: Uint128::from(state.lottery_deposits),
            }],
            msg: to_binary(&AnchorMsg::DepositStable {})?,
        })],
        log: vec![log("action", "execute_lottery")],
        data: None,
    })
}

fn apply_reserve_factor(awarded_amount: Decimal256, reserve_factor: Decimal256) -> Decimal256 {
    awarded_amount * reserve_factor
}

// distribution is a vector [0, 0, 0.025, 0.15, 0.3, 0.5]
fn assign_prize(awardable_prize: Decimal256, winners: u8, distribution: Decimal256) -> Decimal256 {
    awardable_prize * distribution / Decimal256::from_uint256(winners)
}

pub fn is_valid_sequence(sequence: &str, len: u8) -> bool {
    return if sequence.len() != (len as usize) {
        false
    } else if !sequence.chars().all(|c| ('0'..='9').contains(&c)) {
        false
    } else {
        true
    };
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

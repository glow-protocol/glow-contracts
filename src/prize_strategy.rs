use crate::msg::HandleMsg;
use crate::querier::{query_balance, query_token_balance};
use crate::state::{
    read_all_sequences, read_config, read_depositor_info, read_matching_sequences,
    read_sequence_info, read_state, store_depositor_info, store_lottery_info, store_sequence_info,
    store_state, LotteryInfo,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    log, to_binary, Api, CanonicalAddr, Coin, CosmosMsg, Env, Extern, HandleResponse, HandleResult,
    Querier, StdError, Storage, WasmMsg,
};
use cw0::Expiration;
use cw20::Cw20HandleMsg::Send as Cw20Send;
use moneymarket::market::{Cw20HookMsg, HandleMsg as AnchorMsg};
use std::collections::HashMap;
use std::ops::{Add, Sub};

pub fn execute_lottery<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> HandleResult {
    let mut state = read_state(&deps.storage)?;
    let config = read_config(&deps.storage)?;

    // No sent funds allowed when executing the lottery
    if !env.message.sent_funds.is_empty() {
        return Err(StdError::generic_err(
            "Do not send funds when executing the lottery",
        ));
    }

    if state.next_lottery_time.is_expired(&env.block) {
        //state.next_lottery_time = state.next_lottery_time.add(config.lottery_interval)?;
        state.next_lottery_time =
            Expiration::AtTime(env.block.time).add(config.lottery_interval)?;
    } else {
        return Err(StdError::generic_err(format!(
            "Lottery is still running, please check again after {}",
            state.next_lottery_time
        )));
    }

    // Get contract current aUST balance
    let total_aterra_balance = query_token_balance(
        &deps,
        &deps.api.human_address(&config.a_terra_contract)?,
        &deps.api.human_address(&config.contract_addr)?,
    )?;

    // Get contract current UST balance (used in _handle_prize)
    state.current_balance = query_balance(
        &deps,
        &deps.api.human_address(&config.contract_addr)?,
        "uusd".to_string(),
    )?;

    if total_aterra_balance.is_zero() {
        return Err(StdError::generic_err(
            "No current available aUST funds to execute the lottery",
        ));
    }

    // Get lottery deposits of aUST
    let lottery_aterra =
        (Decimal256::from_uint256(total_aterra_balance) * config.split_factor) * Uint256::one();

    // Store state
    store_state(&mut deps.storage, &state)?;

    // TODO: deduct anchor redemption taxes
    // Message for redeem amount operation of aUST
    let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps.api.human_address(&config.a_terra_contract)?,
        send: vec![],
        msg: to_binary(&Cw20Send {
            contract: deps.api.human_address(&config.anchor_contract)?,
            amount: lottery_aterra.into(),
            msg: Some(to_binary(&Cw20HookMsg::RedeemStable {})?),
        })?,
    });

    // Prepare message for internal call to _handle_prize
    let handle_prize_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps.api.human_address(&config.contract_addr)?,
        send: vec![],
        msg: to_binary(&HandleMsg::_HandlePrize {})?,
    });

    // Handle Response withdraws from Anchor and call internal _handle_prize
    Ok(HandleResponse {
        messages: vec![redeem_msg, handle_prize_msg],
        log: vec![
            log("action", "execute_lottery"),
            log("redeemed_amount", lottery_aterra),
        ],
        data: None,
    })
}

pub fn _handle_prize<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> HandleResult {
    if env.message.sender != env.contract.address {
        return Err(StdError::unauthorized());
    }

    let mut state = read_state(&deps.storage)?;
    let config = read_config(&deps.storage)?;

    // TODO: Get random sequence here
    // TODO: deduct terra taxes and oracle fees
    let winning_sequence = String::from("00000");

    // Get contract current uusd balance
    let curr_balance = query_balance(
        &deps,
        &deps.api.human_address(&config.contract_addr)?,
        String::from("uusd"),
    )?;

    // TODO: make sure balance delta is positive
    // Get delta after aUST redeem operation
    let balance_delta = Decimal256::from_uint256(curr_balance - state.current_balance);

    //println!("award_available {:?}", balance_delta);
    //println!("lottery_deposits {:?}", state.lottery_deposits);

    // TODO: make sure interest is positive
    // Minus total_lottery_deposits and we get outstanding_interest
    let outstanding_interest = balance_delta - state.lottery_deposits;

    // Add outstanding_interest to previous available award
    state.award_available += outstanding_interest;

    let prize = state.award_available;

    if prize.is_zero() {
        return Err(StdError::generic_err(
            "There is no UST balance to fund the prize",
        ));
    }

    // Get winners and respective sequences
    let lucky_holders: Vec<(u8, Vec<CanonicalAddr>)> =
        read_matching_sequences(&deps, None, None, &winning_sequence);

    let mut map_winners = HashMap::new();
    for (k, mut v) in lucky_holders.clone() {
        map_winners.entry(k).or_insert_with(Vec::new).append(&mut v)
    }

    // assign prizes
    let mut total_awarded_prize = Decimal256::zero();
    let mut total_reserve_commission = Decimal256::zero();

    for (matches, winners) in map_winners.iter() {
        let number_winners = winners.len() as u8;
        for winner in winners {
            let mut depositor = read_depositor_info(&deps.storage, winner);

            /*
            println!("prize {:?}", prize);
            println!("number_winners {:?}", number_winners);
            println!("matches {:?}", *matches);
            println!("prize_distribution {:?}", &config.prize_distribution);
             */

            let assigned =
                assign_prize(prize, *matches, number_winners, &config.prize_distribution);

            //println!("assigned {:?}", assigned);

            total_awarded_prize += assigned;
            let reserve_commission = apply_reserve_factor(assigned, config.reserve_factor);
            total_reserve_commission += reserve_commission;
            let amount_redeem = (assigned - reserve_commission) * Uint256::one();
            depositor.redeemable_amount = depositor.redeemable_amount.add(amount_redeem.into());

            store_depositor_info(&mut deps.storage, winner, &depositor)?;
        }
    }

    let lottery_info = LotteryInfo {
        sequence: winning_sequence,
        awarded: true,
        total_prizes: total_awarded_prize,
        winners: lucky_holders,
    };

    store_lottery_info(&mut deps.storage, state.current_lottery, &lottery_info)?;

    // state.next_lottery_time = state.next_lottery_time.add(config.lottery_interval)?;
    state.current_lottery += 1;
    state.total_reserve = state.total_reserve.add(total_reserve_commission);
    state.award_available = state.award_available.sub(total_awarded_prize);
    store_state(&mut deps.storage, &state)?;

    let reinvest_amount = state.lottery_deposits * Uint256::one();

    let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps.api.human_address(&config.anchor_contract)?,
        send: vec![Coin {
            denom: config.stable_denom,
            amount: reinvest_amount.into(),
        }],
        msg: to_binary(&AnchorMsg::DepositStable {})?,
    });

    Ok(HandleResponse {
        messages: vec![redeem_msg],
        log: vec![
            log("action", "handle_prize"),
            log("total_awarded_prize", total_awarded_prize),
            log("reinvested_amount", reinvest_amount),
        ],
        data: None,
    })
}

fn apply_reserve_factor(awarded_amount: Decimal256, reserve_factor: Decimal256) -> Decimal256 {
    awarded_amount * reserve_factor
}

// distribution is a vector [0, 0, 0.025, 0.15, 0.3, 0.5]
fn assign_prize(
    awardable_prize: Decimal256,
    matches: u8,
    winners: u8,
    distribution: &Vec<Decimal256>,
) -> Decimal256 {
    let number_winners = Uint256::from(winners as u64);

    println!("distribution element {:?}", matches);

    awardable_prize * distribution[matches as usize] / Decimal256::from_uint256(number_winners)
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
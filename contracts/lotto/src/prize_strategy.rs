use crate::error::ContractError;
use crate::querier::query_exchange_rate;
use crate::state::{
    read_config, read_depositor_info, read_lottery_info, read_matching_sequences, read_state,
    store_depositor_info, store_lottery_info, store_state, LotteryInfo, TICKETS,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, to_binary, Addr, CanonicalAddr, CosmosMsg, DepsMut, Env, MessageInfo, Order, Response,
    StdResult, Uint128, WasmMsg,
};
use cw0::Expiration;
use cw20::Cw20ExecuteMsg::Send as Cw20Send;
use cw_storage_plus::Bound;
use terraswap::querier::query_token_balance;

use crate::contract::compute_reward;
use moneymarket::market::Cw20HookMsg;
use std::ops::{Add, Sub};
use std::str;
use std::usize;

pub fn execute_lottery(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut state = read_state(deps.storage)?;
    let config = read_config(deps.storage)?;

    // Compute global Glow rewards
    compute_reward(&mut state, env.block.height);

    // No sent funds allowed when executing the lottery
    if !info.funds.is_empty() {
        return Err(ContractError::InvalidLotteryExecution {});
    }

    if state.next_lottery_time.is_expired(&env.block) {
        state.next_lottery_time =
            Expiration::AtTime(env.block.time).add(config.lottery_interval)?;
    } else {
        return Err(ContractError::LotteryInProgress {});
    }

    // TODO: Get random sequence here
    let winning_sequence = String::from("00000");

    let lottery_info = LotteryInfo {
        sequence: winning_sequence,
        awarded: false,
        total_prizes: Decimal256::zero(),
        winners: vec![],
        page: "".to_string(),
    };

    store_lottery_info(deps.storage, state.current_lottery, &lottery_info)?;

    // Get pooled lottery deposits in Anchor
    let aust_balance = query_token_balance(
        &deps.querier,
        deps.api.addr_humanize(&config.a_terra_contract)?,
        env.clone().contract.address,
    )?;

    let aust_lottery_balance = Uint256::from(aust_balance).multiply_ratio(
        (state.shares_supply - state.deposit_shares) * Uint256::one(),
        state.shares_supply * Uint256::one(),
    );
    let rate = query_exchange_rate(
        deps.as_ref(),
        deps.api.addr_humanize(&config.anchor_contract)?.to_string(),
    )?
    .exchange_rate;

    let pooled_lottery_deposits = Decimal256::from_uint256(aust_lottery_balance) * rate;

    let mut msgs: Vec<CosmosMsg> = vec![];
    // Redeem funds if lottery related shares are greater than outstanding lottery deposits
    let mut aust_to_redeem = Decimal256::zero();
    if state.lottery_deposits >= pooled_lottery_deposits {
        if state.award_available.is_zero() {
            return Err(ContractError::InsufficientLotteryFunds {});
        }
    } else {
        let amount_to_redeem = pooled_lottery_deposits - state.lottery_deposits;
        aust_to_redeem = amount_to_redeem / rate;
        state.award_available += amount_to_redeem;

        // Message for redeem amount operation of aUST
        let redeem_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps
                .api
                .addr_humanize(&config.a_terra_contract)?
                .to_string(),
            funds: vec![],
            msg: to_binary(&Cw20Send {
                contract: deps.api.addr_humanize(&config.anchor_contract)?.to_string(),
                amount: (aust_to_redeem * Uint256::one()).into(),
                msg: to_binary(&Cw20HookMsg::RedeemStable {})?,
            })?,
        });
        msgs.push(redeem_msg);
    }

    // TODO: add msg to drand worker fee

    // Store state
    store_state(deps.storage, &state)?;

    let res = Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", "execute_lottery"),
        attr("redeemed_amount", aust_to_redeem.to_string()),
    ]);
    Ok(res)
}

fn calc_limit(request: Option<u32>) -> usize {
    request.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize
}

const MAX_LIMIT: u32 = 120;
const DEFAULT_LIMIT: u32 = 100;

pub fn execute_prize(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut state = read_state(deps.storage)?;
    let config = read_config(deps.storage)?;

    // Compute global Glow rewards
    compute_reward(&mut state, env.block.height);

    // No sent funds allowed when executing the lottery
    if !info.funds.is_empty() {
        return Err(ContractError::InvalidLotteryExecution {});
    }

    let mut lottery_info = read_lottery_info(deps.storage, state.current_lottery);

    if lottery_info.sequence.is_empty() {
        return Err(ContractError::InvalidLotteryPrizeExecution {});
    }

    // Get winners and respective sequences
    let lucky_holders: Vec<(u8, Vec<CanonicalAddr>)> =
        read_matching_sequences(deps.as_ref(), &lottery_info.sequence);

    let limit = calc_limit(None);

    // Get bounds
    let min_bound = &lottery_info.sequence[..2];
    let max_bound_number = min_bound.parse::<i32>().unwrap() + 1;
    let max_bound = &max_bound_number.to_string()[..];

    // TODO: avoid next loop including function in a map
    // Get winning tickets
    /*
    let winning_tickets: Vec<_> = TICKETS
        .range(
            deps.storage,
            Some(Bound::Inclusive(Vec::from(min_bound))),
            Some(Bound::Exclusive(Vec::from(max_bound))),
            Order::Ascending,
        )
        .take(limit)
        .collect()
        .unwrap();

     */

    let winning_tickets: Vec<_> = TICKETS
        .range(deps.storage, None, None, Order::Ascending)
        .collect::<StdResult<(Vec<_>)>>()
        .unwrap();

    // assign prizes
    let mut total_awarded_prize = Decimal256::zero();
    let mut total_reserve_commission = Decimal256::zero();

    for (seq, winners) in winning_tickets.into_iter() {
        let number_winners = winners.len() as u64;
        let matches = count_seq_matches(
            &lottery_info.sequence.clone(),
            str::from_utf8(&*seq).unwrap(),
        ); // TODO: improve this
        for winner in winners {
            // TODO: improve this
            let mut depositor = read_depositor_info(
                deps.as_ref().storage,
                &deps.api.addr_canonicalize(&winner.to_string()).unwrap(),
            );

            let assigned = assign_prize(
                state.award_available,
                matches,
                number_winners,
                &config.prize_distribution,
            );

            total_awarded_prize += assigned;
            let reserve_commission = apply_reserve_factor(assigned, config.reserve_factor);
            total_reserve_commission += reserve_commission;
            let amount_redeem = (assigned - reserve_commission) * Uint256::one();
            depositor.redeemable_amount = depositor
                .redeemable_amount
                .add(Uint128::from(amount_redeem));

            // TODO: improve this
            store_depositor_info(
                deps.storage,
                &deps.api.addr_canonicalize(&winner.to_string()).unwrap(),
                &depositor,
            )?;
        }
    }

    lottery_info.winners = lucky_holders;
    lottery_info.awarded = true;
    lottery_info.total_prizes = total_awarded_prize;

    store_lottery_info(deps.storage, state.current_lottery, &lottery_info)?;

    state.current_lottery += 1;
    state.total_reserve = state.total_reserve.add(total_reserve_commission);
    state.award_available = state.award_available.sub(total_awarded_prize);
    store_state(deps.storage, &state)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "execute_prize"),
        attr("total_awarded_prize", total_awarded_prize.to_string()),
    ]))
}

fn apply_reserve_factor(awarded_amount: Decimal256, reserve_factor: Decimal256) -> Decimal256 {
    awarded_amount * reserve_factor
}

// distribution is a vector e.g. [0, 0, 0.025, 0.15, 0.3, 0.5]
fn assign_prize(
    awardable_prize: Decimal256,
    matches: u8,
    winners: u64,
    distribution: &[Decimal256],
) -> Decimal256 {
    let number_winners = Uint256::from(winners as u64);

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
        } else {
            break;
        }
    }
    count
}

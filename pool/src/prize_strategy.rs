use itertools::Itertools;

use crate::state::{
    read_all_sequences, read_config, read_depositor_info, read_matching_sequences,
    read_sequence_info, read_state, store_config, store_depositor_info, store_lottery_info,
    store_sequence_info, store_state, Config, LotteryInfo, State,
};
use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    from_binary, log, to_binary, Api, BankMsg, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern,
    HandleResponse, HandleResult, HumanAddr, InitResponse, InitResult, Querier, StdError,
    StdResult, Storage, Uint128, WasmMsg,
};

pub fn execute_lottery<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> HandleResult {
    let mut state = read_state(&deps.storage)?;
    let mut config = read_config(&deps.storage)?;

    // No sent funds allowed when executing the lottery
    if !env.message.sent_funds.is_empty() {
        Err(StdError::generic_err(
            "Do not send funds when executing the lottery",
        ))
    };

    if env.block.time > state.next_lottery_time {
        state.next_lottery_time += config.lottery_interval;
    } else {
        Err(StdError::generic_err(format!(
            "Lottery is still running, please check again after {}",
            state.next_lottery_time
        )));
    }

    // TODO: Get random sequence here
    let winning_sequence = String::from("34280");

    // TODO: Get how much interest do we have available in anchor
    let mut current_award = state.award_available;
    let lottery_deposits = state.total_deposits * config.split_factor;

    // totalAnchorDeposit = aUST_lottery_balance * exchangeRate
    // awardable_prize = totalAnchorDeposit - totalLotteryDeposits
    let outstanding_interest = 10_000_000_000.0; // let's do it 10k
    let awardable_prize: Decimal256 = outstanding_interest - state.total_deposits;

    // Deduct reserve fees
    let reserve_fee = awardable_prize * config.reserve_factor;
    let prize = awardable_prize - reserve_fee;

    // TODO: deduct terra taxes and oracle fees

    // Get winners and respective sequences
    let lucky_holders: Vec<(u8, CanonicalAddr)> =
        read_matching_sequences(&deps, None, None, &winning_sequence)?;

    let map_winners = lucky_holders.into_iter().into_group_map();

    // assign prizes
    let mut total_awarded_prize = Decimal256::zero();

    for (matches, winners) in map_winners.iter() {
        let number_winners = winners.len() as u8;
        for winner in winners {
            let mut depositor = read_depositor_info(&deps.storage, winner);

            let assigned = assign_prize(prize, number_winners, config.prize_distribution[matches]);

            // TODO: apply reserve factor to the prizes, not to the awardable_prize
            depositor.redeemable_amount += assigned;
            total_awarded_prize += assigned;

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
    store_state(&mut deps.storage, &state)?;

    // TODO: update assets and deposits related state
    // award_available, spendable...

    Ok(HandleResponse {
        messages: vec![],
        log: vec![log("action", "execute_lottery")],
        data: None,
    })
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

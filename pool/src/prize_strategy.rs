use cosmwasm_std::{
    from_binary, log, to_binary, Api, Binary, CanonicalAddr, HumanAddr, CosmosMsg, Env, Extern,
    HandleResponse, HandleResult, InitResponse, InitResult, Querier, StdError, StdResult, Storage,
    Uint128, WasmMsg, BankMsg, Coin
};
use crate::state::{read_config, read_state, store_config, store_state, Config, State,
                   read_sequence_info, store_sequence_info, read_matching_sequences,
                    read_all_sequences
};
use cosmwasm_bignumber::Decimal256;


pub fn execute_lottery <S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env
) -> HandleResult {

    let mut state = read_state(&deps.storage)?;
    let mut config = read_config(&deps.storage)?;

    // No sent funds allowed when executing the lottery
    if !env.message.sent_funds.is_empty() {
        Err(StdError::generic_err("Do not send funds when executing the lottery"))
    };

    if env.block.time > state.next_lottery_time {
        state.next_lottery_time += config.lottery_interval;
    } else {
        Err(StdError::generic_err(
            format!("Lottery is still running, please check again after {}",
                    state.next_lottery_time
            )));
    }

    // TODO: Get random sequence here
    let winning_sequence = String::from("34280");

    // TODO: Get how much interest do we have available in anchor

    // totalAnchorDeposit = aUST_lottery_balance * exchangeRate
    // awardable_prize = totalAnchorDeposit - totalLotteryDeposits
    let outstanding_interest = 10_000_000_000.0 ;// let's do it 10k
    let awardable_prize: Decimal256 = outstanding_interest - state.total_deposits;

    // Deduct reserve fees
    let reserve_fee = awardable_prize * config.reserve_factor;
    let prize = awardable_prize - reserve_fee;

    // Get winners and respective sequences
    let winners: Vec<String, Vec<CanonicalAddr>> = read_matching_sequences(
        &deps,
        None,
        None,
        &winning_sequence
    )?;

    // TODO: continue here - ranking winners
    // we may do it inside read_matching_seqs


}

pub fn is_valid_sequence(sequence: &str, len: u8) -> bool {
    return if sequence.len() != (len as usize) {
        false
    } else if !sequence.chars().all(|c| ('0'..='9').contains(&c)) {
        false
    } else {
        true
    }
}

pub fn count_seq_matches (a: &str, b: &str) -> u8 {
    let mut count = 0;
    for (i, c) in a.chars().enumerate() {
        if c == b.chars().nth(i).unwrap() { count += 1;}
    }
    count
}

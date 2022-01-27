use crate::contract::{query_config, query_pool};
use crate::mock_querier::MOCK_CONTRACT_ADDR;
use crate::state::STATE;
use crate::tests::{A_UST, RATE};
use glow_protocol::lotto::NUM_PRIZE_BUCKETS;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{coin, Addr, Deps};
use glow_protocol::querier::{deduct_tax, query_token_balance};
use std::convert::TryInto;

pub fn calculate_prize_buckets(deps: Deps) -> [Uint256; NUM_PRIZE_BUCKETS] {
    let pool = query_pool(deps).unwrap();
    let config = query_config(deps).unwrap();
    let state = STATE.load(deps.storage).unwrap();

    let contract_a_balance = query_token_balance(
        deps,
        Addr::unchecked(A_UST),
        Addr::unchecked(MOCK_CONTRACT_ADDR),
    )
    .unwrap();

    // Lottery balance equals aust_balance - total_user_savings_aust
    let aust_lottery_balance = contract_a_balance - pool.total_user_savings_aust;

    // Get the value of the lottery balance
    let pooled_lottery_deposits = aust_lottery_balance * Decimal256::permille(RATE);

    // Calculate the amount of ust to be redeemed for the lottery
    let amount_to_redeem = pooled_lottery_deposits
        - pool.total_user_lottery_deposits
        - pool.total_sponsor_lottery_deposits;

    // Calculate the corresponding amount of aust to redeem
    let aust_to_redeem = amount_to_redeem / Decimal256::permille(RATE);

    // Get the value of the redeemed aust after accounting for rounding errors
    let aust_to_redeem_value = aust_to_redeem * Decimal256::permille(RATE);

    // Get the post tax amount
    let net_amount = Uint256::from(
        deduct_tax(deps, coin((aust_to_redeem_value).into(), "uusd"))
            .unwrap()
            .amount,
    );

    let mut prize_buckets = state.prize_buckets;

    for index in 0..state.prize_buckets.len() {
        // Add the proportional amount of the net redeemed amount to the relevant award bucket.
        prize_buckets[index] += net_amount * config.prize_distribution[index];
    }

    // Return the initial balance plus the post tax redeemed aust value
    prize_buckets
}

pub fn calculate_lottery_prize_buckets(
    state_prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    number_winners: [u32; NUM_PRIZE_BUCKETS],
) -> [Uint256; NUM_PRIZE_BUCKETS] {
    state_prize_buckets
        .iter()
        .zip(&number_winners)
        .map(|(a, b)| if *b == 0 { Uint256::zero() } else { *a })
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

pub fn calculate_remaining_state_prize_buckets(
    state_prize_buckets: [Uint256; NUM_PRIZE_BUCKETS],
    number_winners: [u32; NUM_PRIZE_BUCKETS],
) -> [Uint256; NUM_PRIZE_BUCKETS] {
    state_prize_buckets
        .iter()
        .zip(&number_winners)
        .map(|(a, b)| if *b == 0 { *a } else { Uint256::zero() })
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

pub fn generate_sequential_ticket_combinations(num_combinations: u64) -> Vec<String> {
    (0..num_combinations)
        .map(|x: u64| format!("{:06x}", x))
        .collect::<Vec<String>>()
}

pub fn combinations_to_encoded_tickets(combinations: Vec<String>) -> String {
    // Convert each string to
    // when it's a string its taking 8 bits per char
    // but each char only holds 4 bits of information
    // convert it to just 4 bits, but then thats u4 not u8. u8 is 256

    // Encode the vec of u8 with base64
    base64::encode(
        combinations
            // Iterate over combinations
            .iter()
            // Take each combination and hex decode it
            .flat_map(|s| hex::decode(s).unwrap())
            // Then collect the flat map into a vec of u8
            .collect::<Vec<u8>>(),
    )
}

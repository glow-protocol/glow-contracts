use cosmwasm_std::{Coin, Deps, StdResult, Uint128};
use sha3::{Digest, Keccak256};
use terra_cosmwasm::{SwapResponse, TerraQuerier};

#[allow(dead_code)]
fn my_hash(val: u128) -> u128 {
    let mut hasher = Keccak256::new();
    hasher.update(val.to_string());
    let mut dst: [u8; 16] = [0u8; 16];
    dst.clone_from_slice(&hasher.finalize()[..16]);

    u128::from_be_bytes(dst)
}

#[allow(dead_code)]
pub fn rn_gen(deps: Deps, init_entropy: u128, upperbound: u128) -> StdResult<Uint128> {
    let terra_querier = TerraQuerier::new(&deps.querier);
    let res: SwapResponse = terra_querier.query_swap(
        Coin {
            denom: String::from("uusd"),
            amount: Uint128::from(init_entropy),
        },
        "uluna",
    )?;
    let entropy = res.receive.amount.u128();

    let min: u128 = (u128::MAX - upperbound) % upperbound;
    let mut random: u128 = my_hash(entropy);

    loop {
        if random >= min {
            break;
        }
        random = my_hash(random);
    }

    Ok(Uint128::from(random % upperbound))
}

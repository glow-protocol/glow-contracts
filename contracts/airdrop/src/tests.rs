use crate::contract::{execute, instantiate, query};
use crate::error::ContractError;
use crate::mock_querier::mock_dependencies;
use cosmwasm_std::testing::{mock_env, mock_info};
use cosmwasm_std::{
    attr, from_binary, to_binary, Addr, CosmosMsg, SubMsg, Timestamp, Uint128, WasmMsg,
};
use cw20::Cw20ExecuteMsg;
use glow_protocol::airdrop::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, IsClaimedResponse, LatestStageResponse,
    MerkleRootResponse, QueryMsg,
};

#[test]
fn proper_instantiation() {
    let mut deps = mock_dependencies(&[]);

    let msg = InstantiateMsg {
        owner: "owner0000".to_string(),
        glow_token: "glow0000".to_string(),
    };

    let info = mock_info("addr0000", &[]);

    // we can just call .unwrap() to assert this was a success
    let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    // it worked, let's query the state
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!("owner0000", config.owner.as_str());
    assert_eq!("glow0000", config.glow_token.as_str());

    let res = query(deps.as_ref(), mock_env(), QueryMsg::LatestStage {}).unwrap();
    let latest_stage: LatestStageResponse = from_binary(&res).unwrap();
    assert_eq!(0u8, latest_stage.latest_stage);
}

#[test]
fn update_config() {
    let mut deps = mock_dependencies(&[]);

    let msg = InstantiateMsg {
        owner: "owner0000".to_string(),
        glow_token: "glow0000".to_string(),
    };

    let info = mock_info("addr0000", &[]);
    let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    // update owner
    let info = mock_info("owner0000", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: Some("owner0001".to_string()),
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // it worked, let's query the state
    let res = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let config: ConfigResponse = from_binary(&res).unwrap();
    assert_eq!("owner0001", config.owner.as_str());

    // Unauthorzied err
    let info = mock_info("owner0000", &[]);
    let msg = ExecuteMsg::UpdateConfig { owner: None };

    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(ContractError::Unauthorized {}) => {}
        _ => panic!("Must return unauthorized error"),
    }
}

#[test]
fn register_merkle_root() {
    let seconds1 = 1635256000;
    let seconds2 = 1635256100;

    let mut deps = mock_dependencies(&[]);
    let mut env = mock_env();
    env.block.time = Timestamp::from_seconds(seconds1);

    let msg = InstantiateMsg {
        owner: "owner0000".to_string(),
        glow_token: "glow0000".to_string(),
    };

    let info = mock_info("addr0000", &[]);
    let _res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

    // register new merkle root
    let info = mock_info("owner0000", &[]);
    let msg = ExecuteMsg::RegisterMerkleRoot {
        merkle_root: "634de21cde1044f41d90373733b0f0fb1c1c71f9652b905cdf159e73c4cf0d37".to_string(),
        expiry_at_seconds: seconds2,
    };

    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
    assert_eq!(
        res.attributes,
        vec![
            attr("action", "register_merkle_root"),
            attr("stage", "1"),
            attr(
                "merkle_root",
                "634de21cde1044f41d90373733b0f0fb1c1c71f9652b905cdf159e73c4cf0d37"
            ),
            attr("expiry_at_seconds", seconds2.to_string())
        ]
    );

    let res = query(deps.as_ref(), env.clone(), QueryMsg::LatestStage {}).unwrap();
    let latest_stage: LatestStageResponse = from_binary(&res).unwrap();
    assert_eq!(1u8, latest_stage.latest_stage);

    let res = query(
        deps.as_ref(),
        env.clone(),
        QueryMsg::MerkleRoot {
            stage: latest_stage.latest_stage,
        },
    )
    .unwrap();
    let merkle_root: MerkleRootResponse = from_binary(&res).unwrap();
    assert_eq!(
        "634de21cde1044f41d90373733b0f0fb1c1c71f9652b905cdf159e73c4cf0d37".to_string(),
        merkle_root.merkle_root
    );

    // Verify that only the owner of the contract is able to register airdrop stages
    let info = mock_info("owner0001", &[]);
    let msg = ExecuteMsg::RegisterMerkleRoot {
        merkle_root: "634de21cde1044f41d90373733b0f0fb1c1c71f9652b905cdf159e73c4cf0d37".to_string(),
        expiry_at_seconds: seconds2,
    };
    let res = execute(deps.as_mut(), env.clone(), info, msg);
    match res {
        Err(ContractError::Unauthorized {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    println!("{}, {}", seconds1, env.block.time.seconds());

    // Verify that you can't register an airdrop stage with an expiry in the past
    let info = mock_info("owner0000", &[]);
    let msg = ExecuteMsg::RegisterMerkleRoot {
        merkle_root: "634de21cde1044f41d90373733b0f0fb1c1c71f9652b905cdf159e73c4cf0d37".to_string(),
        expiry_at_seconds: seconds1,
    };
    let res = execute(deps.as_mut(), env, info, msg);
    match res {
        Err(ContractError::InvalidExpiryAtSeconds {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

#[test]
fn claim() {
    let seconds0 = 1635255900;
    let seconds1 = 1635256000;
    let seconds2 = 1635256100;
    let seconds3 = 1635256200;

    let mut deps = mock_dependencies(&[]);
    let mut env = mock_env();

    // start with a block time before all the airdrop expiry times
    env.block.time = Timestamp::from_seconds(seconds0);

    let msg = InstantiateMsg {
        owner: "owner0000".to_string(),
        glow_token: "glow0000".to_string(),
    };

    let info = mock_info("addr0000", &[]);
    let _res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Register merkle roots
    let info = mock_info("owner0000", &[]);
    let msg = ExecuteMsg::RegisterMerkleRoot {
        merkle_root: "85e33930e7a8f015316cb4a53a4c45d26a69f299fc4c83f17357e1fd62e8fd95".to_string(),
        expiry_at_seconds: seconds2,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let info = mock_info("owner0000", &[]);
    let msg = ExecuteMsg::RegisterMerkleRoot {
        merkle_root: "634de21cde1044f41d90373733b0f0fb1c1c71f9652b905cdf159e73c4cf0d37".to_string(),
        expiry_at_seconds: seconds3,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    let info = mock_info("owner0000", &[]);
    let msg = ExecuteMsg::RegisterMerkleRoot {
        merkle_root: "634de21cde1044f41d90373733b0f0fb1c1c71f9652b905cdf159e73c4cf0d37".to_string(),
        expiry_at_seconds: seconds1,
    };
    let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    // update the block time so that the last airdrop is expired

    env.block.time = Timestamp::from_seconds(seconds1);

    let msg = ExecuteMsg::Claim {
        amount: Uint128::new(1000001u128),
        stage: 1u8,
        proof: vec![
            "b8ee25ffbee5ee215c4ad992fe582f20175868bc310ad9b2b7bdf440a224b2df".to_string(),
            "98d73e0a035f23c490fef5e307f6e74652b9d3688c2aa5bff70eaa65956a24e1".to_string(),
            "f328b89c766a62b8f1c768fefa1139c9562c6e05bab57a2af87f35e83f9e9dcf".to_string(),
            "fe19ca2434f87cadb0431311ac9a484792525eb66a952e257f68bf02b4561950".to_string(),
        ],
    };

    let info = mock_info("terra1qfqa2eu9wp272ha93lj4yhcenrc6ymng079nu8", &[]);
    let res = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap();
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: "glow0000".to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: "terra1qfqa2eu9wp272ha93lj4yhcenrc6ymng079nu8".to_string(),
                amount: Uint128::new(1000001u128),
            })
            .unwrap(),
            funds: vec![]
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "claim"),
            attr("stage", "1"),
            attr("address", "terra1qfqa2eu9wp272ha93lj4yhcenrc6ymng079nu8"),
            attr("amount", "1000001")
        ]
    );

    assert!(
        from_binary::<IsClaimedResponse>(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::IsClaimed {
                    stage: 1,
                    address: "terra1qfqa2eu9wp272ha93lj4yhcenrc6ymng079nu8".to_string(),
                }
            )
            .unwrap()
        )
        .unwrap()
        .is_claimed
    );

    let res = execute(deps.as_mut(), env.clone(), info, msg);
    match res {
        Err(ContractError::AlreadyClaimed {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Claim next airdrop
    let msg = ExecuteMsg::Claim {
        amount: Uint128::new(2000001u128),
        stage: 2u8,
        proof: vec![
            "ca2784085f944e5594bb751c3237d6162f7c2b24480b3a37e9803815b7a5ce42".to_string(),
            "5b07b5898fc9aa101f27344dab0737aede6c3aa7c9f10b4b1fda6d26eb669b0f".to_string(),
            "4847b2b9a6432a7bdf2bdafacbbeea3aab18c524024fc6e1bc655e04cbc171f3".to_string(),
            "cad1958c1a5c815f23450f1a2761a5a75ab2b894a258601bf93cd026469d42f2".to_string(),
        ],
    };

    let info = mock_info("terra1qfqa2eu9wp272ha93lj4yhcenrc6ymng079nu8", &[]);
    let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: "glow0000".to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: "terra1qfqa2eu9wp272ha93lj4yhcenrc6ymng079nu8".to_string(),
                amount: Uint128::new(2000001u128),
            })
            .unwrap(),
            funds: vec![]
        }))]
    );

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "claim"),
            attr("stage", "2"),
            attr("address", "terra1qfqa2eu9wp272ha93lj4yhcenrc6ymng079nu8"),
            attr("amount", "2000001")
        ]
    );

    // Attempt to claim the last airdrop for which the expiry_at_seconds has passed
    let msg = ExecuteMsg::Claim {
        amount: Uint128::new(2000001u128),
        stage: 3u8,
        proof: vec![
            "ca2784085f944e5594bb751c3237d6162f7c2b24480b3a37e9803815b7a5ce42".to_string(),
            "5b07b5898fc9aa101f27344dab0737aede6c3aa7c9f10b4b1fda6d26eb669b0f".to_string(),
            "4847b2b9a6432a7bdf2bdafacbbeea3aab18c524024fc6e1bc655e04cbc171f3".to_string(),
            "cad1958c1a5c815f23450f1a2761a5a75ab2b894a258601bf93cd026469d42f2".to_string(),
        ],
    };

    let info = mock_info("terra1qfqa2eu9wp272ha93lj4yhcenrc6ymng079nu8", &[]);
    let res = execute(deps.as_mut(), env, info, msg);

    match res {
        Err(ContractError::AirdropExpired {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }
}

#[test]
fn withdraw_expired_tokens() {
    let seconds0 = 1635255900;
    let seconds1 = 1635256000;
    let seconds3 = 1635256200;

    let mut deps = mock_dependencies(&[]);

    deps.querier.with_token_balances(&[(
        &"glow0000".to_string(),
        &[(&"airdrop0000".to_string(), &Uint128::from(123u128))],
    )]);

    let mut env = mock_env();

    // Start with a block time before all the airdrop expiry times
    env.block.time = Timestamp::from_seconds(seconds0);

    env.contract.address = Addr::unchecked("airdrop0000".to_string());

    // Instantiate the airdrop with an owner of owner0000 ----------
    let msg = InstantiateMsg {
        owner: "owner0000".to_string(),
        glow_token: "glow0000".to_string(),
    };

    let info = mock_info("addr0000", &[]);
    let _res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

    // Owner attempts to withdraw and receives a NoRegisteredAirdrops error
    // because no airdrops have been registered yet

    let info = mock_info("owner0000", &[]);
    let withdraw_msg = ExecuteMsg::WithdrawExpiredTokens {
        recipient: "owner0000".to_string(),
    };
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info.clone(),
        withdraw_msg.clone(),
    );
    match res {
        Err(ContractError::NoRegisteredAirdrops {}) => {}
        _ => panic!("DO NOT ENTER HERE"),
    }

    // Register a merkle root.
    let msg = ExecuteMsg::RegisterMerkleRoot {
        merkle_root: "634de21cde1044f41d90373733b0f0fb1c1c71f9652b905cdf159e73c4cf0d37".to_string(),
        expiry_at_seconds: seconds1,
    };

    execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

    // Attempt to withdraw funds but get an AirdropNotExpired error
    // because the airdrop expiry time is in the future
    let res = execute(
        deps.as_mut(),
        env.clone(),
        info.clone(),
        withdraw_msg.clone(),
    );
    match res {
        Err(ContractError::AirdropNotExpired {}) => {}
        _ => panic!("Must return airdrop not expired error"),
    }

    // Update the block time so that the airdrop expiry time is in the past
    env.block.time = Timestamp::from_seconds(seconds3);

    // Successfully withdraw the glow tokens
    let res = execute(deps.as_mut(), env.clone(), info, withdraw_msg.clone()).unwrap();

    assert_eq!(res.messages.len(), 1);
    assert_eq!(
        res.attributes,
        vec![
            attr("action", "withdraw_expired_tokens"),
            attr("to", "owner0000"),
            attr("amount", Uint128::from(123u128).to_string())
        ]
    );

    // Verify that only the owner of the contract is able to withdraw tokens
    let info = mock_info("owner0001", &[]);
    let res = execute(deps.as_mut(), env, info, withdraw_msg);
    match res {
        Err(ContractError::Unauthorized {}) => {}
        _ => panic!("Must return airdrop not expired error"),
    }
}

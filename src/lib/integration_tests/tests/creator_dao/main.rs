pub mod test_number_of_creator_tokens;
pub mod types;
pub mod utils;

use ic_ledger_types::Memo;
use ic_sns_governance::pb::v1::governance::Version;
use ic_sns_governance::pb::v1::{
    manage_neuron, neuron, Account, GetRunningSnsVersionRequest, GetRunningSnsVersionResponse,
    ListNeurons, ListNeuronsResponse, ManageNeuron, ManageNeuronResponse,
};
use ic_sns_init::pb::v1::{
    sns_init_payload::InitialTokenDistribution, AirdropDistribution, DeveloperDistribution,
    FractionalDeveloperVotingPower, NeuronDistribution, SnsInitPayload, SwapDistribution,
    TreasuryDistribution,
};
use ic_sns_swap::pb::v1::{
    GetInitRequest, GetInitResponse, NeuronBasketConstructionParameters, NewSaleTicketRequest,
    NewSaleTicketResponse, RefreshBuyerTokensRequest, RefreshBuyerTokensResponse,
};
use sha2::{Digest, Sha256};
use shared_utils::canister_specific::individual_user_template::types::error::AirdropError;
use shared_utils::constant::{
    SNS_TOKEN_ARCHIVE_MODULE_HASH, SNS_TOKEN_GOVERNANCE_MODULE_HASH, SNS_TOKEN_INDEX_MODULE_HASH,
    SNS_TOKEN_LEDGER_MODULE_HASH, SNS_TOKEN_ROOT_MODULE_HASH, SNS_TOKEN_SWAP_MODULE_HASH,
};
use std::time::{Duration, UNIX_EPOCH};
use std::{collections::HashMap, fmt::Debug, str::FromStr, time::SystemTime, vec};
use test_utils::setup::test_constants::get_mock_user_bob_principal_id;
use utils::setup_sns_w_canister_for_creator_dao;

use candid::{encode_args, CandidType, Decode, Encode, Nat, Principal};
use ic_base_types::PrincipalId;
use ic_sns_wasm::init::SnsWasmCanisterInitPayload;
use icp_ledger::Subaccount;
use pocket_ic::WasmResult;
use serde::{Deserialize, Serialize};
use shared_utils::{
    canister_specific::individual_user_template::{
        types::cdao::DeployedCdaoCanisters, types::error::CdaoDeployError,
    },
    common::types::known_principal::KnownPrincipalType,
    constant::SNS_WASM_W_PRINCIPAL_ID,
};
use test_utils::setup::{
    env::pocket_ic_env::get_new_pocket_ic_env,
    test_constants::{
        get_global_super_admin_principal_id, get_mock_user_alice_principal_id,
        get_mock_user_charlie_principal_id,
    },
};

pub const ICP_LEDGER_CANISTER_ID: &'static str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
pub const ICP_INDEX_CANISTER_ID: &'static str = "qhbym-qaaaa-aaaaa-aaafq-cai";

#[derive(CandidType, Deserialize, PartialEq, Eq, Hash, Serialize, Clone)]
struct Wasm {
    wasm: Vec<u8>,
    proposal_id: Option<u64>,
    canister_type: i32,
}

impl Debug for Wasm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wasm")
            .field("proposal_id", &self.proposal_id)
            .field("canister_type", &self.canister_type)
            .finish()
    }
}

#[derive(CandidType, Deserialize, PartialEq, Eq, Hash, Serialize, Clone, Debug)]
struct AddWasmPayload {
    hash: Vec<u8>,
    wasm: Option<Wasm>,
}

#[derive(CandidType, Serialize, Deserialize, Debug, Clone)]
pub struct AddWasmResultRecord {
    pub result: Option<ResultVariant>,
}

#[derive(CandidType, Serialize, Deserialize, Debug, Clone)]
pub enum ResultVariant {
    Error(ErrorRecord),
    Hash(Vec<u8>),
}

#[derive(CandidType, Serialize, Deserialize, Debug, Clone)]
pub struct ErrorRecord {
    pub message: String,
}

fn add_wasm(wasm_file: &[u8], canister_type: u32) -> AddWasmPayload {
    let mut hasher = Sha256::new();
    hasher.update(wasm_file);
    let file_hash = hasher.finalize();

    let wasm_data = AddWasmPayload {
        hash: file_hash.to_vec(),
        wasm: Some(Wasm {
            wasm: wasm_file.to_vec(),
            proposal_id: None,
            canister_type: canister_type as i32,
        }),
    };

    ic_cdk::println!(
        "Wasm data: {:?}\nType: {}, Hash: {}",
        wasm_data,
        canister_type,
        hex::encode(file_hash)
    );

    wasm_data
}

#[test]
fn creator_dao_tests() {
    let (pocket_ic, known_principal) = get_new_pocket_ic_env();
    let platform_canister_id = known_principal
        .get(&KnownPrincipalType::CanisterIdPlatformOrchestrator)
        .cloned()
        .unwrap();

    let super_admin = get_global_super_admin_principal_id();

    let application_subnets = pocket_ic.topology().get_app_subnets();

    let charlie_global_admin = get_mock_user_charlie_principal_id();

    pocket_ic
        .update_call(
            platform_canister_id,
            super_admin,
            "add_principal_as_global_admin",
            candid::encode_one(charlie_global_admin).unwrap(),
        )
        .unwrap();

    pocket_ic
        .update_call(
            platform_canister_id,
            super_admin,
            "update_global_known_principal",
            candid::encode_args((
                KnownPrincipalType::CanisterIdSnsWasm,
                Principal::from_text(SNS_WASM_W_PRINCIPAL_ID).unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();

    let subnet_orchestrator_canister_id: Principal = pocket_ic
        .update_call(
            platform_canister_id,
            charlie_global_admin,
            "provision_subnet_orchestrator_canister",
            candid::encode_one(application_subnets[1]).unwrap(),
        )
        .map(|res| {
            let canister_id_result: Result<Principal, String> = match res {
                WasmResult::Reply(payload) => candid::decode_one(&payload).unwrap(),
                _ => panic!("Canister call failed"),
            };
            canister_id_result.unwrap()
        })
        .unwrap();

    for i in 0..50 {
        pocket_ic.tick();
    }

    let alice_principal = get_mock_user_alice_principal_id();
    let alice_canister_id: Principal = pocket_ic
        .update_call(
            subnet_orchestrator_canister_id,
            alice_principal,
            "get_requester_principals_canister_id_create_if_not_exists",
            candid::encode_one(()).unwrap(),
        )
        .map(|reply_payload| {
            let response: Result<Principal, String> = match reply_payload {
                WasmResult::Reply(payload) => candid::decode_one(&payload).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap()
        .unwrap();

    let alice_initial_cycle_balance = pocket_ic.cycle_balance(alice_canister_id);
    let sns_wasm_w_canister_id = Principal::from_text(SNS_WASM_W_PRINCIPAL_ID).unwrap();

    setup_sns_w_canister_for_creator_dao(&pocket_ic, super_admin);

    let res = pocket_ic
        .update_call(
            sns_wasm_w_canister_id,
            Principal::anonymous(),
            "get_latest_sns_version_pretty".into(),
            candid::encode_one(()).unwrap(),
        )
        .map(|res| {
            let response: HashMap<String, String> = match res {
                WasmResult::Reply(payload) => candid::decode_one(&payload).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap();

    ic_cdk::println!("🧪 HASHMAP {:?}", res);
    assert_eq!(res.len(), 6);
    let start = SystemTime::now();

    let tx_fee = 1u64;

    let sns_init_args = SnsInitPayload {
        confirmation_text: Some("GET RICH QUICK".to_string()),
        transaction_fee_e8s: Some(tx_fee),
        token_name: Some("Simulation Governance".to_string()),
        token_symbol: Some("SIMG".to_string()),
        proposal_reject_cost_e8s: Some(1u64),
        neuron_minimum_stake_e8s: Some(2u64),
        fallback_controller_principal_ids: vec![super_admin.to_string().clone()],
        logo: Some("data:image/png;base64,iVBORw0".to_string()),
        url: Some("https://google.com".to_string()),
        name: Some("Simulation Gov".to_string()),
        description: Some("Simulation gov desc".to_string()),
        neuron_minimum_dissolve_delay_to_vote_seconds: Some(1),
        initial_reward_rate_basis_points: Some(30u64),
        final_reward_rate_basis_points: Some(20u64),
        reward_rate_transition_duration_seconds: Some(1u64),
        max_dissolve_delay_seconds: Some(5u64),
        max_neuron_age_seconds_for_age_bonus: Some(1u64),
        max_dissolve_delay_bonus_percentage: Some(10u64),
        max_age_bonus_percentage: Some(10u64),
        initial_voting_period_seconds: Some(86401u64),
        wait_for_quiet_deadline_increase_seconds: Some(1u64),
        restricted_countries: None,
        dapp_canisters: None,
        min_participants: Some(1),
        min_icp_e8s: None,
        max_icp_e8s: None,
        min_direct_participation_icp_e8s: Some(15u64),
        min_participant_icp_e8s: Some(2000u64),
        max_direct_participation_icp_e8s: Some(100_000_000u64),
        max_participant_icp_e8s: Some(100_000_000u64),
        swap_start_timestamp_seconds: Some(start.duration_since(UNIX_EPOCH).unwrap().as_secs()),
        swap_due_timestamp_seconds: Some(start.duration_since(UNIX_EPOCH).unwrap().as_secs() + 300), // year 3000 - hopefully we'll all be gone by then,
        neuron_basket_construction_parameters: Some(NeuronBasketConstructionParameters {
            count: 2,
            dissolve_delay_interval_seconds: 2,
        }),
        nns_proposal_id: Some(1),
        neurons_fund_participation: Some(false),
        token_logo: Some("data:image/png;base64,iVBORw0".to_string()),
        neurons_fund_participation_constraints: None,
        initial_token_distribution: Some(InitialTokenDistribution::FractionalDeveloperVotingPower(
            FractionalDeveloperVotingPower {
                airdrop_distribution: Some(AirdropDistribution {
                    airdrop_neurons: vec![],
                }),
                developer_distribution: Some(DeveloperDistribution {
                    developer_neurons: vec![
                        NeuronDistribution {
                            controller: Some(
                                PrincipalId::from_str(&alice_principal.to_string()).unwrap(),
                            ),
                            stake_e8s: 60_000_000_000,
                            memo: 0,
                            dissolve_delay_seconds: 0,
                            vesting_period_seconds: None,
                        },
                        NeuronDistribution {
                            controller: Some(
                                PrincipalId::from_str(&alice_principal.to_string()).unwrap(),
                            ),
                            stake_e8s: 100_000,
                            memo: 1,
                            dissolve_delay_seconds: 2,
                            vesting_period_seconds: None,
                        },
                    ],
                }),
                treasury_distribution: Some(TreasuryDistribution {
                    total_e8s: 10_000_000,
                }),
                swap_distribution: Some(SwapDistribution {
                    total_e8s: 65_000_000_000,
                    initial_swap_amount_e8s: 5_000_000,
                }),
            },
        )),
    };

    let res = pocket_ic
        .update_call(
            alice_canister_id,
            alice_principal,
            "deploy_cdao_sns",
            candid::encode_args((sns_init_args, 300 as u64)).unwrap(),
        )
        .map(|res| {
            let response: Result<DeployedCdaoCanisters, CdaoDeployError> = match res {
                WasmResult::Reply(payload) => {
                    ic_cdk::println!("🧪 Call made");
                    Decode!(&payload, Result<DeployedCdaoCanisters, CdaoDeployError>).unwrap()
                }
                _ => panic!("\n🛑 deploy cdao failed with {:?}", res),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);

    let res = pocket_ic
        .query_call(
            alice_canister_id,
            alice_principal,
            "get_well_known_principal_value",
            candid::encode_one((KnownPrincipalType::CanisterIdSnsWasm)).unwrap(),
        )
        .map(|res| {
            let response: Option<Principal> = match res {
                WasmResult::Reply(payload) => candid::decode_one(&payload).unwrap(),
                _ => panic!("\n🛑 get_well_known_principal_value failed"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res.unwrap().to_string());

    let res = pocket_ic
        .query_call(
            alice_canister_id,
            alice_principal,
            "deployed_cdao_canisters",
            candid::encode_one(()).unwrap(),
        )
        .map(|res| {
            let response: Vec<DeployedCdaoCanisters> = match res {
                WasmResult::Reply(payload) => candid::decode_one(&payload).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);
    for can in &res {
        ic_cdk::println!("🧪 Gov Canister ID: {:?}", can.governance.to_string());
        ic_cdk::println!("🧪 Ind Canister ID: {:?}", can.index.to_string());
        ic_cdk::println!("🧪 Ldg Canister ID: {:?}", can.ledger.to_string());
        ic_cdk::println!("🧪 Rrt Canister ID: {:?}", can.root.to_string());
        ic_cdk::println!("🧪 Swp Canister ID: {:?}", can.swap.to_string());
    }

    assert!(res.len() == 1);
    let res = res[0].clone();
    let root_canister = res.root;
    let swap_canister = res.swap;
    let gov_canister = res.governance;
    let ledger_canister = res.ledger;

    ic_cdk::println!("🧪🧪🧪 Swap Canister ID: {:?}", swap_canister.to_string());

    let res = pocket_ic
        .query_call(
            Principal::from_text(ICP_LEDGER_CANISTER_ID).unwrap(),
            super_admin,
            "icrc1_total_supply",
            candid::encode_one(()).unwrap(),
        )
        .map(|res| {
            let response = match res {
                WasmResult::Reply(payload) => Decode!(&payload, Nat).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);

    // check super admin icp balance
    let res = pocket_ic
        .query_call(
            Principal::from_text(ICP_LEDGER_CANISTER_ID).unwrap(),
            super_admin,
            "icrc1_balance_of",
            candid::encode_one(types::Icrc1BalanceOfArg {
                owner: super_admin,
                subaccount: None,
            })
            .unwrap(),
        )
        .map(|res| {
            let response = match res {
                WasmResult::Reply(payload) => Decode!(&payload, Nat).unwrap(),
                _ => panic!("\n🛑 icrc_1_balance_of call failed\n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);

    //
    pocket_ic.advance_time(Duration::from_secs(200));
    pocket_ic.tick();

    let res = pocket_ic
        .update_call(
            swap_canister,
            super_admin,
            "new_sale_ticket",
            candid::encode_one(NewSaleTicketRequest {
                amount_icp_e8s: 1000000,
                subaccount: None,
            })
            .unwrap(),
        )
        .map(|res| {
            let response: NewSaleTicketResponse = match res {
                WasmResult::Reply(payload) => Decode!(&payload, NewSaleTicketResponse).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);

    let subaccount = Subaccount::from(&PrincipalId(super_admin));
    let transfer_args = types::Transaction {
        memo: Some(vec![0]),
        amount: Nat::from(1000000 as u64),
        fee: Some(Nat::from(0 as u64)),
        from_subaccount: None,
        to: types::Recipient {
            owner: swap_canister,
            subaccount: Some(subaccount.to_vec()),
        },
        created_at_time: None,
    };
    let res = pocket_ic
        .update_call(
            Principal::from_text(ICP_LEDGER_CANISTER_ID).unwrap(),
            super_admin,
            "icrc1_transfer",
            Encode!(&transfer_args).unwrap(),
        )
        .map(|res| {
            let response: types::TransferResult = match res {
                WasmResult::Reply(payload) => Decode!(&payload, types::TransferResult).unwrap(),
                _ => panic!("\n🛑 icrc1_transfer failed with: {:?}", res),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);

    let res = pocket_ic
        .update_call(
            swap_canister,
            super_admin,
            "refresh_buyer_tokens",
            candid::encode_one(RefreshBuyerTokensRequest {
                buyer: super_admin.to_string(),
                confirmation_text: Some("GET RICH QUICK".to_string()),
            })
            .unwrap(),
        )
        .map(|res| {
            let response: RefreshBuyerTokensResponse = match res {
                WasmResult::Reply(payload) => {
                    Decode!(&payload, RefreshBuyerTokensResponse).unwrap()
                }
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);

    pocket_ic.advance_time(Duration::from_secs(301));
    for _ in 0..500 {
        pocket_ic.tick();
    }

    let res = pocket_ic
        .query_call(
            swap_canister,
            super_admin,
            "get_init",
            candid::encode_one(GetInitRequest {}).unwrap(),
        )
        .map(|res| {
            let response = match res {
                WasmResult::Reply(payload) => Decode!(&payload, GetInitResponse).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);

    let res = pocket_ic
        .update_call(
            gov_canister,
            super_admin,
            "list_neurons",
            candid::encode_one(ListNeurons {
                of_principal: Some(PrincipalId(alice_principal)),
                limit: 2,
                start_page_at: None,
            })
            .unwrap(),
        )
        .map(|res| {
            let response: ListNeuronsResponse = match res {
                WasmResult::Reply(payload) => Decode!(&payload, ListNeuronsResponse).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);

    let neurons = res.neurons;
    let mut ix = 0;
    if neurons[1].dissolve_state.is_some() {
        if let Some(neuron::DissolveState::DissolveDelaySeconds(x)) =
            neurons[1].dissolve_state.as_ref()
        {
            if *x == 0 {
                ix = 1;
            }
        }
    }
    let neuron_id = neurons[ix].id.as_ref().unwrap().id.clone();
    let amount = neurons[ix].cached_neuron_stake_e8s;
    let manage_neuron_arg = ManageNeuron {
        subaccount: neuron_id,
        command: Some(manage_neuron::Command::Disburse(manage_neuron::Disburse {
            to_account: Some(Account {
                owner: Some(PrincipalId(alice_principal)),
                subaccount: None,
            }),
            amount: Some(manage_neuron::disburse::Amount { e8s: amount }),
        })),
    };

    pocket_ic.advance_time(Duration::from_secs(250));
    for _ in 0..10 {
        pocket_ic.tick();
    }

    let res = pocket_ic
        .update_call(
            gov_canister,
            alice_principal,
            "manage_neuron",
            candid::encode_one(manage_neuron_arg).unwrap(),
        )
        .map(|res| {
            let response: ManageNeuronResponse = match res {
                WasmResult::Reply(payload) => Decode!(&payload, ManageNeuronResponse).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", res);

    let res = pocket_ic
        .query_call(
            ledger_canister,
            alice_principal,
            "icrc1_balance_of",
            candid::encode_one(types::Icrc1BalanceOfArg {
                owner: alice_principal,
                subaccount: None,
            })
            .unwrap(),
        )
        .map(|res| {
            let response = match res {
                WasmResult::Reply(payload) => Decode!(&payload, Nat).unwrap(),
                _ => panic!("\n🛑 icrc1_balance_of failed \n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 SNS token Balance of alice: {:?}", res);

    let expected_balance = Nat::from(60_000_000_000 - tx_fee);
    ic_cdk::println!("🧪 Expected Balance: {:?}", expected_balance);

    let alice_canister_final_cycle_balance = pocket_ic.cycle_balance(alice_canister_id);

    assert!(alice_canister_final_cycle_balance > alice_initial_cycle_balance);

    assert!(res == expected_balance);

    let sns_running_version = pocket_ic
        .query_call(
            gov_canister,
            Principal::anonymous(),
            "get_running_sns_version",
            candid::encode_one(GetRunningSnsVersionRequest {}).unwrap(),
        )
        .map(|wasm_result| {
            let result: GetRunningSnsVersionResponse = match wasm_result {
                WasmResult::Reply(payload) => candid::decode_one(&payload).unwrap(),
                _ => panic!("Call to get version of sns failed"),
            };
            result
        })
        .unwrap();

    let deployed_sns_version = Version {
        governance_wasm_hash: hex::decode(SNS_TOKEN_GOVERNANCE_MODULE_HASH).unwrap(),
        root_wasm_hash: hex::decode(SNS_TOKEN_ROOT_MODULE_HASH).unwrap(),
        ledger_wasm_hash: hex::decode(SNS_TOKEN_LEDGER_MODULE_HASH).unwrap(),
        swap_wasm_hash: hex::decode(SNS_TOKEN_SWAP_MODULE_HASH).unwrap(),
        index_wasm_hash: hex::decode(SNS_TOKEN_INDEX_MODULE_HASH).unwrap(),
        archive_wasm_hash: hex::decode(SNS_TOKEN_ARCHIVE_MODULE_HASH).unwrap(),
    };

    assert_eq!(
        sns_running_version.deployed_version,
        Some(deployed_sns_version)
    );
    //Upgrade Governance Canister and check the running version
    let bob = get_mock_user_bob_principal_id();
    let bob_canister_id: Principal = pocket_ic
        .update_call(
            subnet_orchestrator_canister_id,
            bob,
            "get_requester_principals_canister_id_create_if_not_exists",
            candid::encode_one(()).unwrap(),
        )
        .map(|reply_payload| {
            let response: Result<Principal, String> = match reply_payload {
                WasmResult::Reply(payload) => candid::decode_one(&payload).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap()
        .unwrap();

    // simulating off-chain allocation (kinda)
    let decimals = pocket_ic
        .query_call(
            ledger_canister,
            alice_canister_id,
            "icrc1_decimals",
            Encode!(&()).unwrap(),
        )
        .map(|res| {
            let response: u8 = match res {
                WasmResult::Reply(payload) => Decode!(&payload, u8).unwrap(),
                _ => panic!("\n🛑 icrc1_transfer failed with: {:?}", res),
            };
            response
        })
        .unwrap();

    let transfer_args = types::TransferArg {
        from_subaccount: None,
        to: types::Account {
            owner: alice_canister_id,
            subaccount: None,
        },
        fee: None,
        created_at_time: None,
        memo: None,
        amount: Nat::from(200u32) * 10u64.pow(decimals.into()),
    };
    let transfer = pocket_ic
        .update_call(
            ledger_canister,
            alice_principal,
            "icrc1_transfer",
            Encode!(&transfer_args).unwrap(),
        )
        .map(|res| {
            let response: types::TransferResult = match res {
                WasmResult::Reply(payload) => Decode!(&payload, types::TransferResult).unwrap(),
                _ => panic!("\n🛑 icrc1_transfer failed with: {:?}", res),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", transfer);

    // claiming airdrop
    let res = pocket_ic
        .update_call(
            alice_canister_id,
            bob,
            "request_airdrop",
            encode_args((
                root_canister,
                None::<Memo>,
                Nat::from(100u64) * 10u64.pow(decimals.into()),
                bob_canister_id,
            ))
            .unwrap(),
        )
        .map(|reply_payload| {
            let response: Result<(), AirdropError> = match reply_payload {
                WasmResult::Reply(payload) => Decode!(&payload, Result<(), AirdropError>).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        });
    ic_cdk::println!("🧪 Result: {:?}", res);
    assert!(res.as_ref().unwrap().is_ok());

    // trying to claim the airdrop again
    let res: Result<Result<(), AirdropError>, pocket_ic::UserError> = pocket_ic
        .update_call(
            alice_canister_id,
            bob,
            "request_airdrop",
            encode_args((
                root_canister,
                None::<Memo>,
                Nat::from(100u64) * 10u64.pow(decimals.into()),
                bob_canister_id,
            ))
            .unwrap(),
        )
        .map(|reply_payload| {
            let response: Result<(), AirdropError> = match reply_payload {
                WasmResult::Reply(payload) => Decode!(&payload, Result<(), AirdropError>).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        });

    ic_cdk::println!("🧪 Result: {:?}", res);
    assert!(
        res.as_ref().unwrap().is_err() && res.unwrap() == Err(AirdropError::AlreadyClaimedAirdrop)
    );

    // trying to claim the airdrop with the wrong canister id
    let res: Result<Result<(), AirdropError>, pocket_ic::UserError> = pocket_ic
        .update_call(
            alice_canister_id,
            bob,
            "request_airdrop",
            encode_args((
                root_canister,
                None::<Memo>,
                Nat::from(100u64) * 10u64.pow(decimals.into()),
                Principal::anonymous(),
            ))
            .unwrap(),
        )
        .map(|reply_payload| {
            let response: Result<(), AirdropError> = match reply_payload {
                WasmResult::Reply(payload) => Decode!(&payload, Result<(), AirdropError>).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        });

    ic_cdk::println!("🧪 Result: {:?}", res);
    assert!(res.unwrap().is_err());

    let deployed_cdao = pocket_ic
        .query_call(
            alice_canister_id,
            alice_principal,
            "deployed_cdao_canisters",
            candid::encode_one(()).unwrap(),
        )
        .map(|res| {
            let response: Vec<DeployedCdaoCanisters> = match res {
                WasmResult::Reply(payload) => candid::decode_one(&payload).unwrap(),
                _ => panic!("\n🛑 get requester principals canister id failed\n"),
            };
            response
        })
        .unwrap();
    ic_cdk::println!("🧪 Result: {:?}", deployed_cdao);
    assert!(deployed_cdao[0]
        .airdrop_info
        .is_airdrop_claimed(&bob)
        .unwrap());

    let bob_bal = pocket_ic
        .query_call(
            ledger_canister,
            alice_canister_id,
            "icrc1_balance_of",
            candid::encode_one(types::Icrc1BalanceOfArg {
                owner: bob,
                subaccount: None,
            })
            .unwrap(),
        )
        .map(|res| match res {
            WasmResult::Reply(payload) => Decode!(&payload, Nat).unwrap(),
            _ => panic!("\n🛑 get bob principal bal failed\n"),
        })
        .unwrap();
    ic_cdk::println!("🧪 SNS token Balance of bob principal: {:?}", bob_bal);

    let alice_bal = pocket_ic
        .query_call(
            ledger_canister,
            alice_canister_id,
            "icrc1_balance_of",
            candid::encode_one(types::Icrc1BalanceOfArg {
                owner: alice_canister_id,
                subaccount: None,
            })
            .unwrap(),
        )
        .map(|res| match res {
            WasmResult::Reply(payload) => Decode!(&payload, Nat).unwrap(),
            _ => panic!("\n🛑 get alice canister bal failed\n"),
        })
        .unwrap();
    ic_cdk::println!("🧪 SNS token Balance of alice canister: {:?}", alice_bal);

    assert!(bob_bal == Nat::from(100u64) * 10u64.pow(decimals.into()));
}

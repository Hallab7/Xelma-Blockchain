// SPDX-License-Identifier: MIT
//! Economic attacks — fee gaming, exposure cap boundary abuse.

use super::super::config_helpers::{apply_max_stake, apply_max_user_exposure};
use super::{emit_result, setup_contract};
use crate::errors::ContractError;
use crate::types::{BetSide, DataKey, FeeModel, OraclePayload};
use soroban_sdk::{testutils::Ledger, Address, Env};

/// Attacker tries to game fee by having admin change fee config after bets are placed.
/// Defense: settlement still conserves value; treasury delta matches configured fee.
#[test]
fn test_fee_gaming_mid_round_config_conservation() {
    let env = Env::default();
    let (client, contract_id, _admin, _oracle) = setup_contract(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    client.mint_initial(&alice);
    client.mint_initial(&bob);

    client.create_round(&1_000u128, &None);
    client.place_bet(&alice, &60, &BetSide::Up);
    client.place_bet(&bob, &40, &BetSide::Up);

    // Attacker hopes admin fee change mid-round creates arbitrage
    env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .set(&DataKey::ProtocolFeeBps, &1_000u32);
        env.storage()
            .persistent()
            .set(&DataKey::FeeModel, &FeeModel::FeeOnPot);
    });

    env.ledger().with_mut(|li| li.sequence_number = 12);
    let treasury_before = client.get_protocol_fee_treasury();

    client.resolve_round(&OraclePayload {
        price: 2_000u128,
        timestamp: env.ledger().timestamp(),
        round_id: 0,
        nonce: 1u64,
        network_id: env.ledger().network_id(),
        contract_addr: contract_id.clone(),
        confidence: None,
    });

    let alice_pay = client.get_pending_winnings(&alice);
    let bob_pay = client.get_pending_winnings(&bob);
    let treasury_delta = client.get_protocol_fee_treasury() - treasury_before;

    assert_eq!(alice_pay + bob_pay + treasury_delta, 100);
    assert_eq!(treasury_delta, 10);

    emit_result(
        "fee_gaming_mid_round_config",
        "conservation invariant (fee-on-pot)",
        "none — fee applied at settlement",
        "low",
        false,
    );
}

/// Attacker stakes at the exposure cap boundary then tries one stroop more.
#[test]
fn test_exposure_cap_boundary_attack_blocked() {
    let env = Env::default();
    let (client, _cid, _admin, _oracle) = setup_contract(&env);

    apply_max_user_exposure(&env, &client, Some(100_0000000i128));
    apply_max_stake(&env, &client, Some(100_0000000i128));

    let attacker = Address::generate(&env);
    client.mint_initial(&attacker);
    client.create_round(&1_0000000, &None);

    client.place_bet(&attacker, &100_0000000, &BetSide::Up);

    let balance_before = client.balance(&attacker);
    let result = client.try_place_bet(&attacker, &1, &BetSide::Up);
    assert_eq!(result, Err(Ok(ContractError::ExposureCapExceeded)));
    assert_eq!(client.balance(&attacker), balance_before);

    emit_result(
        "exposure_cap_boundary",
        "ExposureCapExceeded",
        "sybil addresses can bypass per-user cap (accepted)",
        "medium",
        false,
    );
}

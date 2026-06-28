// SPDX-License-Identifier: MIT
//! Event coverage and completeness verification tests (Issue #117).

use super::config_helpers::apply_windows;
use crate::contract::{VirtualTokenContract, VirtualTokenContractClient};
use crate::errors::ContractError;
use crate::types::{BetSide, ConfigChangeKind, ConfigChangePayload, OraclePayload};
use soroban_sdk::xdr::ToXdr;
use std::vec::Vec;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger as _},
    Address, Bytes, BytesN, Env, IntoVal, Symbol, TryIntoVal, Val,
};

fn setup() -> (
    Env,
    Address,
    Address,
    Address,
    VirtualTokenContractClient<'static>,
) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VirtualTokenContract, ());
    let client = VirtualTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
    client.initialize(&admin, &oracle);
    (env, contract_id, admin, oracle, client)
}

fn assert_last_config_updated(
    env: &Env,
    kind: ConfigChangeKind,
    old_value: ConfigChangePayload,
    new_value: ConfigChangePayload,
) {
    let events = env.events().all();
    let (_contract, topics, data) = events
        .iter()
        .rev()
        .find(|(_contract, topics, _data)| {
            topics.len() == 2
                && topics.get(0).unwrap().try_into_val(env) == Ok(symbol_short!("config"))
                && topics.get(1).unwrap().try_into_val(env) == Ok(symbol_short!("updated"))
        })
        .expect("config updated event should exist");

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(env),
        Ok(symbol_short!("config"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(env),
        Ok(symbol_short!("updated"))
    );
    assert_eq!(data.try_into_val(env), Ok((kind, old_value, new_value)));
}

#[test]
fn test_event_coverage_direct_config_setters_emit_audit_event() {
    let (env, _, _, _, client) = setup();

    client.set_min_participants(&Some(2));
    assert_last_config_updated(
        &env,
        ConfigChangeKind::MinParticipants,
        ConfigChangePayload::MinParticipants(None),
        ConfigChangePayload::MinParticipants(Some(2)),
    );

    client.set_max_precision_participants(&25);
    assert_last_config_updated(
        &env,
        ConfigChangeKind::MaxPrecisionParticipants,
        ConfigChangePayload::MaxPrecisionParticipants(1_000),
        ConfigChangePayload::MaxPrecisionParticipants(25),
    );

    client.set_mint_limit(&7);
    assert_last_config_updated(
        &env,
        ConfigChangeKind::MintLimit,
        ConfigChangePayload::MintLimit(0),
        ConfigChangePayload::MintLimit(7),
    );

    client.set_archive_retention(&64);
    assert_last_config_updated(
        &env,
        ConfigChangeKind::ArchiveRetention,
        ConfigChangePayload::ArchiveRetention(128),
        ConfigChangePayload::ArchiveRetention(64),
    );
}

#[test]
fn test_event_coverage_timelocked_config_apply_emits_audit_event() {
    let (env, _, _, _, client) = setup();

    client.schedule_windows(&10, &20);
    env.ledger().with_mut(|li| li.sequence_number = 1_441);
    client.apply_scheduled_changes(&ConfigChangeKind::Windows);

    assert_last_config_updated(
        &env,
        ConfigChangeKind::Windows,
        ConfigChangePayload::Windows(6, 12),
        ConfigChangePayload::Windows(10, 20),
    );
}

#[test]
fn test_event_coverage_mint_initial() {
    let (env, _, _, _, client) = setup();
    let user = Address::generate(&env);

    client.mint_initial(&user);

    let events = env.events().all();
    let last_event = events.last().unwrap();
    let (_contract, topics, data) = last_event;

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(&env),
        Ok(symbol_short!("mint"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(&env),
        Ok(symbol_short!("initial"))
    );
    assert_eq!(data.try_into_val(&env), Ok((user, 1000_0000000i128)));
}

#[test]
fn test_event_coverage_create_round() {
    let (env, _, _, _, client) = setup();

    client.create_round(&1_0000000, &None); // UpDown Mode (0)

    let events = env.events().all();
    let last_event = events.last().unwrap();
    let (_contract, topics, data) = last_event;

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(&env),
        Ok(symbol_short!("round"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(&env),
        Ok(symbol_short!("created"))
    );
    assert_eq!(
        data.try_into_val(&env),
        Ok((1u64, 1_0000000u128, 0u32, 6u32, 12u32, 0u32))
    );
}

#[test]
fn test_event_coverage_set_windows() {
    let (env, _, _, _, client) = setup();

    apply_windows(&env, &client, 10, 20);

    let events = env.events().all();
    let windows_event = events.iter().rev().find(|e| {
        let (_contract, topics, _data) = e;
        topics.len() == 2
            && topics.get(0).unwrap().try_into_val(&env) == Ok(symbol_short!("windows"))
            && topics.get(1).unwrap().try_into_val(&env) == Ok(symbol_short!("updated"))
    });
    let (_contract, topics, data) = windows_event.expect("windows updated event should exist");

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(&env),
        Ok(symbol_short!("windows"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(&env),
        Ok(symbol_short!("updated"))
    );
    assert_eq!(data.try_into_val(&env), Ok((10u32, 20u32)));
}

#[test]
fn test_event_coverage_place_bet() {
    let (env, _, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &None);

    client.place_bet(&user, &100_0000000, &BetSide::Up);

    let events = env.events().all();
    let last_event = events.last().unwrap();
    let (_contract, topics, data) = last_event;

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(&env),
        Ok(symbol_short!("bet"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(&env),
        Ok(symbol_short!("placed"))
    );
    assert_eq!(
        data.try_into_val(&env),
        Ok((user, 1u64, 100_0000000i128, 0u32))
    );
}

#[test]
fn test_event_coverage_commit_and_reveal() {
    let (env, _, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &Some(1)); // Precision mode

    let price = 500u128;
    let salt = BytesN::from_array(&env, &[1; 32]);
    let mut preimage = Bytes::new(&env);
    preimage.append(&price.to_xdr(&env));
    preimage.append(&salt.clone().to_xdr(&env));
    let hash = env.crypto().sha256(&preimage);

    let committed_hash: BytesN<32> = hash.into();
    client.commit_prediction(&user, &committed_hash.clone(), &100_0000000);

    let events = env.events().all();
    let last_event = events.last().unwrap();
    let (_contract, topics, data) = last_event;

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(&env),
        Ok(symbol_short!("commit"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(&env),
        Ok(symbol_short!("predict"))
    );
    assert_eq!(
        data.try_into_val(&env),
        Ok((user.clone(), 1u64, committed_hash, 100_0000000i128))
    );

    // Move ledger beyond bet window to allow reveal
    env.ledger().with_mut(|li| {
        li.sequence_number = 7;
    });

    client.reveal_prediction(&user, &price, &salt);

    let events = env.events().all();
    let last_event = events.last().unwrap();
    let (_contract, topics, data) = last_event;

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(&env),
        Ok(symbol_short!("reveal"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(&env),
        Ok(symbol_short!("predict"))
    );
    assert_eq!(
        data.try_into_val(&env),
        Ok((user, 1u64, price, 100_0000000i128))
    );
}

#[test]
fn test_event_coverage_resolve_round() {
    let (env, contract_id, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &None);
    client.place_bet(&user, &100_0000000, &BetSide::Up);

    // Advance ledger to resolve
    env.ledger().with_mut(|li| {
        li.sequence_number = 12;
    });

    client.resolve_round(&OraclePayload {
        price: 1_2000000,
        timestamp: env.ledger().timestamp(),
        round_id: 0,
        nonce: 1,
        network_id: env.ledger().network_id(),
        contract_addr: contract_id.clone(),
    });

    let events = env.events().all();
    let last_event = events.last().unwrap();
    let (_contract, topics, data) = last_event;

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(&env),
        Ok(symbol_short!("round"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(&env),
        Ok(symbol_short!("resolved"))
    );
    assert_eq!(data.try_into_val(&env), Ok((1u64, 1_2000000u128, 0u32)));
}

#[test]
fn test_event_coverage_cancel_round() {
    let (env, _, _, _, client) = setup();
    client.create_round(&1_0000000, &None);

    client.cancel_round(&99u32);

    let events = env.events().all();
    let last_event = events.last().unwrap();
    let (_contract, topics, data) = last_event;

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(&env),
        Ok(symbol_short!("round"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(&env),
        Ok(symbol_short!("cancelled"))
    );
    assert_eq!(data.try_into_val(&env), Ok((1u64, 99u32, 0i128, 0i128)));
}

#[test]
fn test_event_coverage_claim_winnings() {
    let (env, contract_id, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &None);
    client.place_bet(&user, &100_0000000, &BetSide::Up);

    env.ledger().with_mut(|li| {
        li.sequence_number = 12;
    });

    client.resolve_round(&OraclePayload {
        price: 1_2000000, // went up -> win
        timestamp: env.ledger().timestamp(),
        round_id: 0,
        nonce: 1,
        network_id: env.ledger().network_id(),
        contract_addr: contract_id.clone(),
    });

    client.claim_winnings(&user);

    let events = env.events().all();
    let last_event = events.last().unwrap();
    let (_contract, topics, data) = last_event;

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(&env),
        Ok(symbol_short!("claim"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(&env),
        Ok(symbol_short!("winnings"))
    );
    assert_eq!(data.try_into_val(&env), Ok((user, 100_0000000i128)));
}

// ─── Action rejected diagnostic events (Issue #196) ─────────────────────────

fn assert_last_action_rejected(
    env: &Env,
    expected_actor: Address,
    expected_action: Symbol,
    expected_reason: ContractError,
) {
    let events = env.events().all();
    let (_contract, topics, data) = events
        .iter()
        .rev()
        .find(|(_contract, topics, _data)| {
            topics.len() == 2
                && topics.get(0).unwrap().try_into_val(env)
                    == Ok(symbol_short!("action"))
                && topics.get(1).unwrap().try_into_val(env)
                    == Ok(symbol_short!("rejct"))
        })
        .expect("action_rejected event should exist");

    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics.get(0).unwrap().try_into_val(env),
        Ok(symbol_short!("action"))
    );
    assert_eq!(
        topics.get(1).unwrap().try_into_val(env),
        Ok(symbol_short!("rejct"))
    );
    assert_eq!(
        data.try_into_val(env),
        Ok((expected_actor, expected_action, expected_reason as u32))
    );
}

#[test]
fn test_action_rejected_create_round_when_paused() {
    let (env, _, _, admin, client) = setup();
    client.pause_contract();

    let result = client.try_create_round(&1_0000000, &None);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("create"),
        ContractError::ContractPaused,
    );
}

#[test]
fn test_action_rejected_create_round_already_active() {
    let (env, _, _, admin, client) = setup();
    client.create_round(&1_0000000, &None);

    let result = client.try_create_round(&2_0000000, &None);
    assert_eq!(result, Err(Ok(ContractError::RoundAlreadyActive)));

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("create"),
        ContractError::RoundAlreadyActive,
    );
}

#[test]
fn test_action_rejected_cancel_round_no_active() {
    let (env, _, _, admin, client) = setup();

    let result = client.try_cancel_round(&0u32);
    assert_eq!(result, Err(Ok(ContractError::RoundNotCancellable)));

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("cancel"),
        ContractError::RoundNotCancellable,
    );
}

#[test]
fn test_action_rejected_oracle_heartbeat_invalid_status() {
    let (env, _, _, _, client) = setup();
    // Use env.as_contract to read oracle for our own check
    let oracle: Address = env.as_contract(&env.register(VirtualTokenContract, ()), || {
        // We need the actual oracle address — extract from the setup helper
        // which stores it at DataKey::Oracle
        Address::generate(&env)
    });

    let result = client.try_update_oracle_heartbeat(&3u32);
    assert_eq!(result, Err(Ok(ContractError::InvalidOracleStatus)));

    // Verify event exists; the oracle address is the one from setup
    let events = env.events().all();
    let action_rejected_events: Vec<_> = events
        .iter()
        .filter(|(_contract, topics, _data)| {
            topics.len() == 2
                && topics.get(0).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("action"))
                && topics.get(1).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("rejct"))
        })
        .collect();
    assert!(!action_rejected_events.is_empty());

    let (_contract, topics, data) = action_rejected_events.last().unwrap();
    let (_actor, action, reason): (Address, Symbol, u32) =
        data.clone().try_into_val(&env).unwrap();
    assert_eq!(action, symbol_short!("hbeat"));
    assert_eq!(reason, ContractError::InvalidOracleStatus as u32);
}

#[test]
fn test_action_rejected_resolve_round_oracle_nonce_reused() {
    let (env, contract_id, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &None);
    client.place_bet(&user, &100_0000000, &BetSide::Up);

    env.ledger().with_mut(|li| {
        li.sequence_number = 12;
    });

    let payload = OraclePayload {
        price: 1_2000000,
        timestamp: env.ledger().timestamp(),
        round_id: 0,
        nonce: 1,
        network_id: env.ledger().network_id(),
        contract_addr: contract_id.clone(),
    };

    // First resolve succeeds
    client.resolve_round(&payload.clone());

    // Second resolve with same nonce should be rejected
    let result = client.try_resolve_round(&payload);
    assert_eq!(result, Err(Ok(ContractError::OracleNonceReused)));

    // Verify event exists
    let events = env.events().all();
    let action_rejected_events: Vec<_> = events
        .iter()
        .filter(|(_contract, topics, _data)| {
            topics.len() == 2
                && topics.get(0).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("action"))
                && topics.get(1).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("rejct"))
        })
        .collect();
    assert!(!action_rejected_events.is_empty());
}

#[test]
fn test_action_rejected_resolve_round_invalid_round_id() {
    let (env, contract_id, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &None);
    client.place_bet(&user, &100_0000000, &BetSide::Up);

    env.ledger().with_mut(|li| {
        li.sequence_number = 12;
    });

    // Use wrong round_id to trigger InvalidOracleRound
    let payload = OraclePayload {
        price: 1_2000000,
        timestamp: env.ledger().timestamp(),
        round_id: 999,
        nonce: 1,
        network_id: env.ledger().network_id(),
        contract_addr: contract_id.clone(),
    };

    let result = client.try_resolve_round(&payload);
    assert_eq!(result, Err(Ok(ContractError::InvalidOracleRound)));

    // Verify event exists
    let events = env.events().all();
    let action_rejected_events: Vec<_> = events
        .iter()
        .filter(|(_contract, topics, _data)| {
            topics.len() == 2
                && topics.get(0).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("action"))
                && topics.get(1).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("rejct"))
        })
        .collect();
    assert!(!action_rejected_events.is_empty());
}

#[test]
fn test_action_rejected_set_archive_retention_invalid() {
    let (env, _, _, admin, client) = setup();

    let result = client.try_set_archive_retention(&0);
    assert_eq!(result, Err(Ok(ContractError::InvalidArchiveRetention)));

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("set_arch"),
        ContractError::InvalidArchiveRetention,
    );
}

#[test]
fn test_action_rejected_withdraw_when_paused() {
    let (env, _, _, admin, client) = setup();
    let recipient = Address::generate(&env);
    client.mint_initial(&recipient);
    client.pause_contract();

    let result = client.try_withdraw_protocol_fee(&recipient, &100_0000000);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("withdraw"),
        ContractError::ContractPaused,
    );
}

#[test]
fn test_action_rejected_set_min_participants_invalid() {
    let (env, _, _, admin, client) = setup();

    let result = client.try_set_min_participants(&Some(0));
    assert_eq!(result, Err(Ok(ContractError::InvalidMinParticipants)));

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("min_par"),
        ContractError::InvalidMinParticipants,
    );
}

#[test]
fn test_action_rejected_set_max_precision_participants_invalid() {
    let (env, _, _, admin, client) = setup();

    let result = client.try_set_max_precision_participants(&0);
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidPrecisionParticipantCap))
    );

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("max_prec"),
        ContractError::InvalidPrecisionParticipantCap,
    );
}

#[test]
fn test_action_rejected_schedule_config_when_paused() {
    let (env, _, _, admin, client) = setup();
    client.pause_contract();

    let result = client.try_schedule_windows(&10, &20);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("sched"),
        ContractError::ContractPaused,
    );
}

#[test]
fn test_action_rejected_cancel_config_when_paused() {
    let (env, _, _, admin, client) = setup();
    client.schedule_windows(&10, &20);
    client.pause_contract();

    let result = client.try_cancel_config_change(&ConfigChangeKind::Windows);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("cncl_cfg"),
        ContractError::ContractPaused,
    );
}

#[test]
fn test_action_rejected_set_mint_limit_when_paused() {
    let (env, _, _, admin, client) = setup();
    client.pause_contract();

    let result = client.try_set_mint_limit(&5);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));

    assert_last_action_rejected(
        &env,
        admin,
        symbol_short!("mint_lim"),
        ContractError::ContractPaused,
    );
}

#[test]
fn test_action_rejected_resolve_round_future_timestamp() {
    let (env, contract_id, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &None);
    client.place_bet(&user, &100_0000000, &BetSide::Up);

    env.ledger().with_mut(|li| {
        li.sequence_number = 12;
    });

    let payload = OraclePayload {
        price: 1_2000000,
        timestamp: env.ledger().timestamp() + 1000, // future
        round_id: 0,
        nonce: 1,
        network_id: env.ledger().network_id(),
        contract_addr: contract_id.clone(),
    };

    let result = client.try_resolve_round(&payload);
    assert_eq!(result, Err(Ok(ContractError::FutureOracleData)));

    let events = env.events().all();
    let action_rejected_events: Vec<_> = events
        .iter()
        .filter(|(_contract, topics, _data)| {
            topics.len() == 2
                && topics.get(0).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("action"))
                && topics.get(1).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("rejct"))
        })
        .collect();
    assert!(!action_rejected_events.is_empty());
}

#[test]
fn test_action_rejected_resolve_round_stale_data() {
    let (env, contract_id, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &None);
    client.place_bet(&user, &100_0000000, &BetSide::Up);

    env.ledger().with_mut(|li| {
        li.sequence_number = 12;
        li.timestamp = 1_700_000_000;
    });

    let payload = OraclePayload {
        price: 1_2000000,
        timestamp: 1_699_999_000, // >300s old
        round_id: 0,
        nonce: 1,
        network_id: env.ledger().network_id(),
        contract_addr: contract_id.clone(),
    };

    let result = client.try_resolve_round(&payload);
    assert_eq!(result, Err(Ok(ContractError::StaleOracleData)));

    let events = env.events().all();
    let action_rejected_events: Vec<_> = events
        .iter()
        .filter(|(_contract, topics, _data)| {
            topics.len() == 2
                && topics.get(0).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("action"))
                && topics.get(1).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("rejct"))
        })
        .collect();
    assert!(!action_rejected_events.is_empty());
}

#[test]
fn test_action_rejected_resolve_round_wrong_network() {
    let (env, contract_id, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &None);
    client.place_bet(&user, &100_0000000, &BetSide::Up);

    env.ledger().with_mut(|li| {
        li.sequence_number = 12;
    });

    let payload = OraclePayload {
        price: 1_2000000,
        timestamp: env.ledger().timestamp(),
        round_id: 0,
        nonce: 1,
        network_id: BytesN::from_array(&env, &[0; 32]), // wrong network
        contract_addr: contract_id.clone(),
    };

    let result = client.try_resolve_round(&payload);
    assert_eq!(result, Err(Ok(ContractError::OracleNetworkMismatch)));

    let events = env.events().all();
    let action_rejected_events: Vec<_> = events
        .iter()
        .filter(|(_contract, topics, _data)| {
            topics.len() == 2
                && topics.get(0).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("action"))
                && topics.get(1).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("rejct"))
        })
        .collect();
    assert!(!action_rejected_events.is_empty());
}

#[test]
fn test_action_rejected_resolve_round_not_ended() {
    let (env, contract_id, _, _, client) = setup();
    let user = Address::generate(&env);
    client.mint_initial(&user);
    client.create_round(&1_0000000, &None);
    client.place_bet(&user, &100_0000000, &BetSide::Up);

    // Don't advance ledger past end_ledger (default is 12)
    // stay at ledger 0 so end_ledger (12) is not reached
    env.ledger().with_mut(|li| {
        li.sequence_number = 11;
    });

    let payload = OraclePayload {
        price: 1_2000000,
        timestamp: env.ledger().timestamp(),
        round_id: 0,
        nonce: 1,
        network_id: env.ledger().network_id(),
        contract_addr: contract_id.clone(),
    };

    let result = client.try_resolve_round(&payload);
    assert_eq!(result, Err(Ok(ContractError::RoundNotEnded)));

    let events = env.events().all();
    let action_rejected_events: Vec<_> = events
        .iter()
        .filter(|(_contract, topics, _data)| {
            topics.len() == 2
                && topics.get(0).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("action"))
                && topics.get(1).unwrap().try_into_val(&env)
                    == Ok(symbol_short!("rejct"))
        })
        .collect();
    assert!(!action_rejected_events.is_empty());
}

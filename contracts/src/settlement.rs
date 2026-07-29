// SPDX-License-Identifier: MIT
use crate::admin::{_ensure_not_paused, _require_supported_schema};
use crate::common::{
    _accumulate_pending, _emit_action_rejected, _extend_persistent_ttl, _extend_ttl_symbol,
    _legacy_positions_key, _set_balance, balance, payout_add, payout_mul, sort_addresses,
    DEFAULT_ARCHIVE_RETENTION,
};
use crate::config::{_apply_protocol_fee_precision, _apply_protocol_fee_updown};
use crate::errors::ContractError;
use crate::types::{
    ArchivedRoundSummary, BetSide, DataKey, HbGateConfig, OracleHeartbeatRecord, OraclePayload,
    PrecisionCommitment, PrecisionPayoutPolicy, PrecisionPrediction, ResolvedParticipant, Round,
    RoundArchiveStatus, RoundMode, RoundSettlement, UserOutcomeType, UserPosition,
    UserRoundOutcome, UserStats,
};
use soroban_sdk::{symbol_short, Address, Env, Map, Symbol, Vec};

// ─── Symbol-keyed storage helpers (DataKey is at 50-variant XDR limit) ────────
// We store maps keyed by round_id under fixed Symbol keys to avoid dynamic keys.

fn _resolved_at_map_key() -> Symbol {
    Symbol::new(&Env::default(), "RslvAtMap")
}

fn _settlement_map_key() -> Symbol {
    Symbol::new(&Env::default(), "SttlMap")
}

fn _pending_finalize_key() -> Symbol {
    Symbol::new(&Env::default(), "PendFinal")
}

fn _read_resolved_at(env: &Env, round_id: u64) -> Option<u32> {
    let key = _resolved_at_map_key();
    env.storage()
        .persistent()
        .get::<_, Map<u64, u32>>(&key)
        .and_then(|m| m.get(round_id))
}

fn _write_resolved_at(env: &Env, round_id: u64, ledger: u32) {
    let key = _resolved_at_map_key();
    let mut m: Map<u64, u32> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Map::new(env));
    m.set(round_id, ledger);
    env.storage().persistent().set(&key, &m);
    _extend_ttl_symbol(env, &key);
}

fn _remove_resolved_at(env: &Env, round_id: u64) {
    let key = _resolved_at_map_key();
    let mut m: Map<u64, u32> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Map::new(env));
    m.remove(round_id);
    if m.len() == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &m);
        _extend_ttl_symbol(env, &key);
    }
}

fn _read_settlement(env: &Env, round_id: u64) -> Option<RoundSettlement> {
    let key = _settlement_map_key();
    env.storage()
        .persistent()
        .get::<_, Map<u64, RoundSettlement>>(&key)
        .and_then(|m| m.get(round_id))
}

fn _write_settlement(env: &Env, round_id: u64, settlement: &RoundSettlement) {
    let key = _settlement_map_key();
    let mut m: Map<u64, RoundSettlement> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Map::new(env));
    m.set(round_id, settlement.clone());
    env.storage().persistent().set(&key, &m);
    _extend_ttl_symbol(env, &key);
}

fn _remove_settlement(env: &Env, round_id: u64) {
    let key = _settlement_map_key();
    let mut m: Map<u64, RoundSettlement> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Map::new(env));
    m.remove(round_id);
    if m.len() == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &m);
        _extend_ttl_symbol(env, &key);
    }
}

/// Cancels the active round and deterministically refunds all participant stakes.
pub fn cancel_round(env: Env, reason: u32) -> Result<(), ContractError> {
    _require_supported_schema(&env)?;
    let admin: Address = env
        .storage()
        .persistent()
        .get(&DataKeyCore::Admin)
        .ok_or(ContractError::AdminNotSet)?;
    admin.require_auth();

    let round: Round = env
        .storage()
        .persistent()
        .get(&DataKeyCore::ActiveRound)
        .ok_or_else(|| {
            _emit_action_rejected(
                &env,
                &admin,
                symbol_short!("cancel"),
                ContractError::RoundNotCancellable,
            );
            ContractError::RoundNotCancellable
        })?;

    let round_id = round.round_id;

    // Refund all participants based on round mode
    let participants: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKeyScoped::RoundParticipants(round_id))
        .unwrap_or(Vec::new(&env));

    match round.mode {
        RoundMode::UpDown => {
            for i in 0..participants.len() {
                if let Some(user) = participants.get(i) {
                    let pos_key = DataKeyScoped::Position(round_id, user.clone());
                    if let Some(pos) = env.storage().persistent().get::<_, UserPosition>(&pos_key) {
                        _accumulate_pending(&env, user.clone(), pos.amount)?;
                        let prediction_side = match pos.side {
                            BetSide::Up => 0,
                            BetSide::Down => 1,
                        };
                        _persist_user_outcome(
                            &env,
                            round_id,
                            0,
                            &user,
                            prediction_side,
                            0,
                            pos.amount,
                            pos.amount,
                            UserOutcomeType::Cancel,
                        );
                        env.storage().persistent().remove(&pos_key);
                    }
                }
            }
        }
        RoundMode::Precision => {
            for i in 0..participants.len() {
                if let Some(user) = participants.get(i) {
                    let pred_key = DataKeyScoped::PrecisionPosition(round_id, user.clone());
                    let commit_key = DataKeyScoped::PrecisionCommitment(round_id, user.clone());

                    let mut refund_amount = 0;
                    if let Some(pred) = env
                        .storage()
                        .persistent()
                        .get::<_, PrecisionPrediction>(&pred_key)
                    {
                        refund_amount = pred.amount;
                    } else if let Some(commit) = env
                        .storage()
                        .persistent()
                        .get::<_, PrecisionCommitment>(&commit_key)
                    {
                        refund_amount = commit.amount;
                    }

                    if refund_amount > 0 {
                        _accumulate_pending(&env, user.clone(), refund_amount)?;
                    }
                    _persist_user_outcome(
                        &env,
                        round_id,
                        1,
                        &user,
                        2,
                        0,
                        refund_amount,
                        refund_amount,
                        UserOutcomeType::Cancel,
                    );
                    env.storage().persistent().remove(&pred_key);
                    env.storage().persistent().remove(&commit_key);
                }
            }
        }
    }

    // Clean up participant list and mark round as cancelled
    let participant_count = participants.len();
    _archive_round(
        &env,
        &round,
        RoundArchiveStatus::Cancelled,
        0,
        participant_count,
        0,
    );

    env.storage()
        .persistent()
        .remove(&DataKeyScoped::RoundParticipants(round_id));
    env.storage()
        .persistent()
        .set(&DataKeyScoped::CancelledRound(round_id), &true);
    env.storage().persistent().remove(&DataKeyCore::ActiveRound);

    // Emit cancellation event
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("round"), symbol_short!("cancel")),
        (round_id, reason, round.pool_up, round.pool_down),
    );

    Ok(())
}

/// Returns true if the given round_id was cancelled.
pub fn is_round_cancelled(env: Env, round_id: u64) -> bool {
    env.storage()
        .persistent()
        .get(&DataKeyScoped::CancelledRound(round_id))
        .unwrap_or(false)
}

/// Claims pending winnings and adds to balance
pub fn claim_winnings(env: Env, user: Address) -> Result<i128, ContractError> {
    _require_supported_schema(&env)?;
    user.require_auth();
    _ensure_not_paused(&env)?;

    let key = DataKeyScoped::PendingWinnings(user.clone());
    let pending: i128 = env.storage().persistent().get(&key).unwrap_or(0);

    if pending == 0 {
        return Ok(0);
    }

    let current_balance = balance(env.clone(), user.clone());
    let new_balance = payout_add(current_balance, pending)?;

    env.storage().persistent().remove(&key);
    _set_balance(&env, user.clone(), new_balance);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("claim"), symbol_short!("winnings")),
        (user, pending),
    );

    Ok(pending)
}

pub fn resolve_round(env: Env, payload: OraclePayload) -> Result<(), ContractError> {
    _require_supported_schema(&env)?;
    if payload.price == 0 {
        return Err(ContractError::InvalidPrice);
    }

    _extend_persistent_ttl(&env, &DataKeyCore::Oracle);
    let oracle: Address = env
        .storage()
        .persistent()
        .get(&DataKeyCore::Oracle)
        .ok_or(ContractError::OracleNotSet)?;

    oracle.require_auth();
    _ensure_not_paused(&env).inspect_err(|&e| {
        _emit_action_rejected(&env, &oracle, symbol_short!("resolve"), e);
    })?;

    let round: Round = env
        .storage()
        .persistent()
        .get(&DataKeyCore::ActiveRound)
        .ok_or(ContractError::NoActiveRound)?;

    // Verify round ID matches to prevent cross-round replays
    if payload.round_id != round.start_ledger {
        _emit_action_rejected(
            &env,
            &oracle,
            symbol_short!("resolve"),
            ContractError::InvalidOracleRound,
        );
        return Err(ContractError::InvalidOracleRound);
    }

    // Reject payloads targeting a different network or contract deployment.
    if payload.network_id != env.ledger().network_id() {
        _emit_action_rejected(
            &env,
            &oracle,
            symbol_short!("resolve"),
            ContractError::OracleNetworkMismatch,
        );
        return Err(ContractError::OracleNetworkMismatch);
    }
    if payload.contract_addr != env.current_contract_address() {
        _emit_action_rejected(
            &env,
            &oracle,
            symbol_short!("resolve"),
            ContractError::OracleNetworkMismatch,
        );
        return Err(ContractError::OracleNetworkMismatch);
    }

    // Verify data freshness (max 300 seconds / 5 minutes old)
    let current_time = env.ledger().timestamp();

    // Reject future timestamps to prevent time-skew manipulation
    if payload.timestamp > current_time {
        _emit_action_rejected(
            &env,
            &oracle,
            symbol_short!("resolve"),
            ContractError::FutureOracleData,
        );
        return Err(ContractError::FutureOracleData);
    }

    if current_time > payload.timestamp + 300 {
        _emit_action_rejected(
            &env,
            &oracle,
            symbol_short!("resolve"),
            ContractError::StaleOracleData,
        );
        return Err(ContractError::StaleOracleData);
    }

    // Oracle deviation guardrails
    _extend_persistent_ttl(&env, &DataKeyCore::OracleMaxDeviationBps);
    if let Some(max_bps) = env
        .storage()
        .persistent()
        .get::<_, u32>(&DataKeyCore::OracleMaxDeviationBps)
    {
        let start_price = round.price_start;
        if start_price == 0 {
            return Err(ContractError::InvalidPrice);
        }

        let diff = if payload.price >= start_price {
            payload
                .price
                .checked_sub(start_price)
                .ok_or(ContractError::Overflow)?
        } else {
            start_price
                .checked_sub(payload.price)
                .ok_or(ContractError::Overflow)?
        };

        let diff_bps_u128 = diff
            .checked_mul(10_000u128)
            .ok_or(ContractError::Overflow)?
            / start_price;
        let diff_bps: u32 = diff_bps_u128
            .try_into()
            .map_err(|_| ContractError::Overflow)?;

        let override_armed: bool = env
            .storage()
            .persistent()
            .get(&DataKeyCore::OracleDeviationOverrideArmed)
            .unwrap_or(false);

        if diff_bps > max_bps && !override_armed {
            #[allow(deprecated)]
            env.events().publish(
                (symbol_short!("oracle"), symbol_short!("rejected")),
                (
                    round.round_id,
                    start_price,
                    payload.price,
                    diff_bps,
                    max_bps,
                ),
            );
            return Err(ContractError::OracleDeviationExceeded);
        }

        if diff_bps > max_bps && override_armed {
            env.storage()
                .persistent()
                .remove(&DataKeyCore::OracleDeviationOverrideArmed);

            #[allow(deprecated)]
            env.events().publish(
                (symbol_short!("oracle"), symbol_short!("override")),
                (
                    round.round_id,
                    start_price,
                    payload.price,
                    diff_bps,
                    max_bps,
                ),
            );
        }
    }

    // Oracle confidence guardrails
    _extend_persistent_ttl(&env, &DataKeyCore::OracleMinConfidenceBps);
    _extend_persistent_ttl(&env, &DataKeyCore::OracleStrictMode);
    if let Some(min_confidence_bps) = env
        .storage()
        .persistent()
        .get::<_, u32>(&DataKeyCore::OracleMinConfidenceBps)
    {
        match payload.confidence {
            None => {
                let strict_mode: bool = env
                    .storage()
                    .persistent()
                    .get(&DataKeyCore::OracleStrictMode)
                    .unwrap_or(false);
                if strict_mode {
                    return Err(ContractError::InvalidPrice);
                }
            }
            Some(confidence_bps) => {
                if confidence_bps > 10_000 || confidence_bps < min_confidence_bps {
                    #[allow(deprecated)]
                    env.events().publish(
                        (symbol_short!("oracle"), symbol_short!("lowconf")),
                        (round.round_id, confidence_bps, min_confidence_bps),
                    );
                    return Err(ContractError::InvalidPrice);
                }
            }
        }
    }

    let nonce_key = DataKeyScoped::ConsumedOracleNonce(round.round_id, payload.nonce);
    if env.storage().persistent().has(&nonce_key) {
        _emit_action_rejected(
            &env,
            &oracle,
            symbol_short!("resolve"),
            ContractError::OracleNonceReused,
        );
        return Err(ContractError::OracleNonceReused);
    }
    env.storage().persistent().set(&nonce_key, &true);

    // ─── Oracle heartbeat health gate (Issue #264) ──────────────────────────
    //
    // When `HbGateConfig.strict_mode` is enabled, `resolve_round` verifies
    // that the oracle heartbeat is live before allowing settlement.
    let hb_config = crate::admin::_load_hb_config(&env);

    if hb_config.strict_mode {
        let hb_blocked = _check_heartbeat_health_blocked(&env, &hb_config);

        if hb_blocked {
            if hb_config.override_armed {
                // Consume the one-shot override
                crate::admin::_consume_hb_override(&env);

                #[allow(deprecated)]
                env.events().publish(
                    (symbol_short!("oracle"), symbol_short!("hoverride")),
                    (round.round_id,),
                );
            } else {
                #[allow(deprecated)]
                env.events().publish(
                    (symbol_short!("oracle"), symbol_short!("hblocked")),
                    (round.round_id,),
                );
                _emit_action_rejected(
                    &env,
                    &oracle,
                    symbol_short!("resolve"),
                    ContractError::OracleNotLive,
                );
                return Err(ContractError::OracleNotLive);
            }
        }
    }

    let current_ledger = env.ledger().sequence();
    if current_ledger < round.end_ledger {
        _emit_action_rejected(
            &env,
            &oracle,
            symbol_short!("resolve"),
            ContractError::RoundNotEnded,
        );
        return Err(ContractError::RoundNotEnded);
    }

    let round_id = round.round_id;

    // Minimum participants threshold check
    if let Some(min) = env
        .storage()
        .persistent()
        .get::<_, u32>(&DataKeyCore::MinParticipants)
    {
        let threshold_participants: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKeyScoped::RoundParticipants(round_id))
            .unwrap_or(Vec::new(&env));
        let count = threshold_participants.len();
        if count < min {
            _archive_round(
                &env,
                &round,
                RoundArchiveStatus::FallbackRefund,
                payload.price,
                count,
                0,
            );
            _refund_under_threshold(&env, &round, &threshold_participants)?;
            #[allow(deprecated)]
            env.events().publish(
                (symbol_short!("round"), symbol_short!("fallback")),
                (round_id, count, min),
            );
            return Ok(());
        }
    }

    let fee_amount = match round.mode {
        RoundMode::UpDown => {
            let (one_sided, fee) = _resolve_updown_mode(&env, &round, payload.price, false)?;
            if one_sided {
                #[allow(deprecated)]
                env.events().publish(
                    (symbol_short!("pool"), symbol_short!("onesided")),
                    (round_id, round.pool_up, round.pool_down),
                );
            }
            fee
        }
        RoundMode::Precision => {
            _resolve_precision_mode(&env, round_id, payload.price, false)?
        }
    };

    let participants: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKeyScoped::RoundParticipants(round_id))
        .unwrap_or(Vec::new(&env));
    let participant_count = participants.len();

    _archive_round(
        &env,
        &round,
        RoundArchiveStatus::Resolved,
        payload.price,
        participant_count,
        fee_amount,
    );

    for i in 0..participants.len() {
        if let Some(user) = participants.get(i) {
            env.storage()
                .persistent()
                .remove(&DataKeyScoped::Position(round_id, user.clone()));
            env.storage()
                .persistent()
                .remove(&DataKeyScoped::PrecisionPosition(round_id, user.clone()));
            env.storage()
                .persistent()
                .remove(&DataKeyScoped::PrecisionCommitment(round_id, user));
        }
    }
    env.storage()
        .persistent()
        .remove(&DataKeyScoped::RoundParticipants(round_id));

    env.storage().persistent().remove(&DataKeyCore::ActiveRound);
    env.storage().persistent().remove(&DataKeyCore::Positions);
    env.storage().persistent().remove(&DataKeyCore::UpDownPositions);
    env.storage()
        .persistent()
        .remove(&DataKeyCore::PrecisionPositions);

    let mode_value: u32 = match round.mode {
        RoundMode::UpDown => 0,
        RoundMode::Precision => 1,
    };
    let policy: u32 = if round.mode == RoundMode::Precision {
        crate::config::get_precision_payout_policy(env.clone())
    } else {
        0
    };
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("round"), symbol_short!("resolved")),
        (
            round_id,
            payload.price,
            mode_value,
            payload.confidence,
            policy,
        ),
    );

    Ok(())
}

// ─── Internal helpers ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn _resolve_updown_mode(
    env: &Env,
    round: &Round,
    final_price: u128,
    skip_payout: bool,
) -> Result<(bool, i128), ContractError> {
    let participants: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKeyScoped::RoundParticipants(round.round_id))
        .unwrap_or(Vec::new(env));
    let participants = sort_addresses(participants);

    let price_went_up = final_price > round.price_start;
    let price_went_down = final_price < round.price_start;
    let price_unchanged = final_price == round.price_start;

    // One-sided: exactly one pool is empty (XOR).  Regardless of which way
    // price moved, if the winning-side pool is 0 there are no winners to pay,
    // and if the losing-side pool is 0 there is nothing to distribute — in
    // both cases every participant gets a full refund.
    let is_one_sided = (round.pool_up == 0) != (round.pool_down == 0);

    let mut fee_amount = 0;

    if !participants.is_empty() {
        if price_unchanged || is_one_sided {
            _record_refunds_indexed(env, round.round_id, 0, &participants, skip_payout)?;
        } else if price_went_up {
            fee_amount = _record_winnings_indexed(
                env,
                round.round_id,
                &participants,
                BetSide::Up,
                round.pool_up,
                round.pool_down,
                skip_payout,
            )?;
        } else if price_went_down {
            fee_amount = _record_winnings_indexed(
                env,
                round.round_id,
                &participants,
                BetSide::Down,
                round.pool_down,
                round.pool_up,
                skip_payout,
            )?;
        }
    } else {
        let positions: Map<Address, UserPosition> = env
            .storage()
            .persistent()
            .get(&DataKeyCore::UpDownPositions)
            .unwrap_or(Map::new(env));
        if !positions.is_empty() {
            if price_unchanged {
                _record_refunds_legacy(env, round.round_id, &positions, skip_payout)?;
            } else if price_went_up {
                fee_amount = _record_winnings_legacy(
                    env,
                    round.round_id,
                    &positions,
                    BetSide::Up,
                    round.pool_up,
                    round.pool_down,
                    skip_payout,
                )?;
            } else if price_went_down {
                fee_amount = _record_winnings_legacy(
                    env,
                    round.round_id,
                    &positions,
                    BetSide::Down,
                    round.pool_down,
                    round.pool_up,
                    skip_payout,
                )?;
            }
        }
    }

    Ok((is_one_sided, fee_amount))
}

pub fn _record_refunds_legacy(
    env: &Env,
    round_id: u64,
    positions: &Map<Address, UserPosition>,
    skip_payout: bool,
) -> Result<(), ContractError> {
    let keys: Vec<Address> = positions.keys();
    for i in 0..keys.len() {
        if let Some(user) = keys.get(i) {
            if let Some(position) = positions.get(user.clone()) {
                if !skip_payout {
                    _accumulate_pending(env, user.clone(), position.amount)?;
                }
                let prediction_side = match position.side {
                    BetSide::Up => 0,
                    BetSide::Down => 1,
                };
                _persist_user_outcome(
                    env,
                    round_id,
                    0,
                    &user,
                    prediction_side,
                    0,
                    position.amount,
                    position.amount,
                    UserOutcomeType::Refund,
                );
            }
        }
    }
    Ok(())
}

pub fn _record_winnings_legacy(
    env: &Env,
    round_id: u64,
    positions: &Map<Address, UserPosition>,
    winning_side: BetSide,
    winning_pool: i128,
    losing_pool: i128,
    skip_payout: bool,
) -> Result<i128, ContractError> {
    if winning_pool == 0 {
        return Ok(0);
    }

    let original_winning_pool = winning_pool;
    let (dist_winning, dist_losing, fee_amount) =
        _apply_protocol_fee_updown(env, round_id, winning_pool, losing_pool)?;
    // Proportional share of ALL distributable funds (handles fee spillover from winning pool).
    let total_distributable = payout_add(dist_winning, dist_losing)?;

    let keys: Vec<Address> = positions.keys();
    for i in 0..keys.len() {
        if let Some(user) = keys.get(i) {
            if let Some(position) = positions.get(user.clone()) {
                if position.side == winning_side {
                    let payout =
                        payout_mul(position.amount, total_distributable)? / original_winning_pool;

                    if !skip_payout {
                        _accumulate_pending(env, user.clone(), payout)?;
                        _update_stats_win(env, user.clone())?;
                    }

                    let side_value = match position.side {
                        BetSide::Up => 0,
                        BetSide::Down => 1,
                    };
                    _persist_user_outcome(
                        env,
                        round_id,
                        0,
                        &user,
                        side_value,
                        0,
                        position.amount,
                        payout,
                        UserOutcomeType::Win,
                    );
                } else {
                    let side_value: u32 = match position.side {
                        BetSide::Up => 0,
                        BetSide::Down => 1,
                    };
                    #[allow(deprecated)]
                    env.events().publish(
                        (symbol_short!("outcome"), symbol_short!("loss")),
                        (
                            user.clone(),
                            round_id,
                            0u32,
                            position.amount,
                            side_value,
                            0u128,
                        ),
                    );
                    if !skip_payout {
                        _update_stats_loss(env, user.clone())?;
                    }

                    _persist_user_outcome(
                        env,
                        round_id,
                        0,
                        &user,
                        side_value,
                        0,
                        position.amount,
                        0,
                        UserOutcomeType::Loss,
                    );
                }
            }
        }
    }

    Ok(fee_amount)
}

fn _calculate_precision_payouts(
    env: &Env,
    winners: &Vec<PrecisionPrediction>,
    payout_pool: i128,
) -> Result<Vec<i128>, ContractError> {
    let policy = crate::config::_read_precision_payout_policy(env);
    let mut payouts = Vec::new(env);
    let mut total_paid = 0i128;

    match policy {
        PrecisionPayoutPolicy::Equal => {
            let winner_count = winners.len() as i128;
            if winner_count > 0 {
                let payout_per_winner = payout_pool / winner_count;
                for _ in 0..winners.len() {
                    payouts.push_back(payout_per_winner);
                    total_paid = payout_add(total_paid, payout_per_winner)?;
                }
            }
        }
        PrecisionPayoutPolicy::StakeWeighted => {
            let mut total_winner_stakes = 0i128;
            for i in 0..winners.len() {
                if let Some(winner) = winners.get(i) {
                    total_winner_stakes = payout_add(total_winner_stakes, winner.amount)?;
                }
            }

            if total_winner_stakes > 0 {
                for i in 0..winners.len() {
                    if let Some(winner) = winners.get(i) {
                        let payout = payout_mul(winner.amount, payout_pool)? / total_winner_stakes;
                        payouts.push_back(payout);
                        total_paid = payout_add(total_paid, payout)?;
                    }
                }
            } else {
                for _ in 0..winners.len() {
                    payouts.push_back(0);
                }
            }
        }
    }

    let remainder = payout_pool
        .checked_sub(total_paid)
        .ok_or(ContractError::PayoutOverflow)?;

    if !winners.is_empty() {
        if let Some(base_payout_0) = payouts.get(0) {
            let payout_0 = payout_add(base_payout_0, remainder)?;
            payouts.set(0, payout_0);
        }
    }

    Ok(payouts)
}

pub fn _resolve_precision_mode(
    env: &Env,
    round_id: u64,
    final_price: u128,
    skip_payout: bool,
) -> Result<i128, ContractError> {
    let mut participants: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKeyScoped::RoundParticipants(round_id))
        .unwrap_or(Vec::new(env));
    participants = sort_addresses(participants);

    if participants.is_empty() {
        let legacy: Map<Address, PrecisionPrediction> = env
            .storage()
            .persistent()
            .get(&DataKeyCore::PrecisionPositions)
            .unwrap_or(Map::new(env));
        if legacy.is_empty() {
            return Ok(0);
        }
        return _resolve_precision_legacy(env, round_id, &legacy, final_price, skip_payout);
    }

    let mut min_diff: Option<u128> = None;
    let mut winners: Vec<PrecisionPrediction> = Vec::new(env);
    let mut total_pot: i128 = 0;
    let mut participant_amounts: Vec<i128> = Vec::new(env);
    let mut participant_prices: Vec<u128> = Vec::new(env);
    let mut is_winner_mask: Vec<bool> = Vec::new(env);

    for i in 0..participants.len() {
        if let Some(user) = participants.get(i) {
            let pred_key = DataKeyScoped::PrecisionPosition(round_id, user.clone());
            let commit_key = DataKeyScoped::PrecisionCommitment(round_id, user.clone());

            let pred_opt = env
                .storage()
                .persistent()
                .get::<_, PrecisionPrediction>(&pred_key);

            let commitment_opt = env
                .storage()
                .persistent()
                .get::<_, PrecisionCommitment>(&commit_key);

            let amount = if let Some(ref pred) = pred_opt {
                pred.amount
            } else if let Some(ref commit) = commitment_opt {
                commit.amount
            } else {
                0
            };
            let cached_price = pred_opt
                .as_ref()
                .map(|p| p.predicted_price)
                .unwrap_or(0u128);

            total_pot = total_pot
                .checked_add(amount)
                .ok_or(ContractError::Overflow)?;
            participant_amounts.push_back(amount);
            participant_prices.push_back(cached_price);
            is_winner_mask.push_back(false);

            if let Some(pred) = pred_opt {
                let diff = if pred.predicted_price >= final_price {
                    pred.predicted_price
                        .checked_sub(final_price)
                        .ok_or(ContractError::Overflow)?
                } else {
                    final_price
                        .checked_sub(pred.predicted_price)
                        .ok_or(ContractError::Overflow)?
                };

                match min_diff {
                    None => {
                        min_diff = Some(diff);
                        winners.push_back(pred.clone());
                        is_winner_mask.set(i, true);
                    }
                    Some(current_min) => {
                        if diff < current_min {
                            min_diff = Some(diff);
                            winners = Vec::new(env);
                            winners.push_back(pred.clone());
                            for j in 0..i {
                                is_winner_mask.set(j, false);
                            }
                            is_winner_mask.set(i, true);
                        } else if diff == current_min {
                            winners.push_back(pred.clone());
                            is_winner_mask.set(i, true);
                        }
                    }
                }
            }
        }
    }

    let mut fee_amount = 0;
    if !winners.is_empty() && total_pot > 0 {
        let (payout_pool, fee) = _apply_protocol_fee_precision(env, round_id, total_pot)?;
        fee_amount = fee;
        let payouts = _calculate_precision_payouts(env, &winners, payout_pool)?;

        for i in 0..winners.len() {
            if let Some(winner) = winners.get(i) {
                let payout = payouts.get(i).unwrap_or(0);

                if !skip_payout {
                    _accumulate_pending(env, winner.user.clone(), payout)?;
                    _update_stats_win(env, winner.user.clone())?;
                }

                _persist_user_outcome(
                    env,
                    round_id,
                    1,
                    &winner.user,
                    2,
                    winner.predicted_price,
                    winner.amount,
                    payout,
                    UserOutcomeType::Win,
                );
            }
        }

        for i in 0..participants.len() {
            if let Some(user) = participants.get(i) {
                let was_winner = is_winner_mask.get(i).unwrap_or(false);
                if !was_winner {
                    let stake = participant_amounts.get(i).unwrap();
                    let predicted_price = participant_prices.get(i).unwrap();

                    #[allow(dep
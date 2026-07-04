// SPDX-License-Identifier: MIT
//! Core contract implementation for the XLM Price Prediction Market.

use soroban_sdk::{
    contract, contractimpl, Address, BytesN, Env, Map, Symbol, Vec,
};

use crate::errors::ContractError;
use crate::types::{
    ArchivedRoundSummary, BetSide, ConfigChangeKind, ConfigChangePayload, DataKey,
    OracleHeartbeatRecord, OraclePayload, OracleRotationProposal, PendingConfigChange,
    PrecisionCommitment, PrecisionPrediction, ProtocolHealthStatus, Round, RoundArchiveStatus,
    RoundMode, UserPosition, UserStats,
};

// ─── Economic control limits ─────────────────────────────────────────────────
/// Minimum allowed value when setting an economic cap to prevent zero-value lockouts.
const MIN_CAP_VALUE: i128 = 1;
/// Upper bound on the minimum-participants config to prevent unbounded gas in resolution.
const MAX_MIN_PARTICIPANTS: u32 = 10_000;
const DEFAULT_MAX_PRECISION_PARTICIPANTS: u32 = 1_000;
const MAX_PRECISION_PARTICIPANTS_LIMIT: u32 = 10_000;
/// Maximum number of entries returned per page by paginated query methods,
/// regardless of the caller-requested `limit` (Issue #139).
const MAX_PAGE_SIZE: u32 = 100;

// ─── Oracle heartbeat limits ──────────────────────────────────────────────────
const DEFAULT_ORACLE_STALE_THRESHOLD: u64 = 3_600; // 1 hour
const MIN_ORACLE_STALE_THRESHOLD: u64 = 60; // 1 minute
const MAX_ORACLE_STALE_THRESHOLD: u64 = 86_400; // 24 hours

// ─── Oracle rotation expiry ───────────────────────────────────────────────────
const MIN_ROTATION_EXPIRY_SECONDS: u64 = 60; // 1 minute minimum

const DEFAULT_BET_WINDOW_LEDGERS: u32 = 6;
const DEFAULT_RUN_WINDOW_LEDGERS: u32 = 12;
const MAX_BET_WINDOW_LEDGERS: u32 = 1_440;
const MAX_RUN_WINDOW_LEDGERS: u32 = 2_880;

const ROUND_MODE_UPDOWN: u32 = 0;
const ROUND_MODE_PRECISION: u32 = 1;
const PAYOUT_OUTCOME_LOSS: u32 = 0;
const PAYOUT_OUTCOME_WIN: u32 = 1;
const PAYOUT_OUTCOME_REFUND: u32 = 2;
// ─── Oracle deviation guardrails ─────────────────────────────────────────────
/// Maximum allowed basis points for oracle deviation is bounded to avoid absurd configs.
/// 100_000 bp = 1000% deviation (effectively "off", but still explicit).
const MAX_ORACLE_DEVIATION_BPS: u32 = 100_000;

// ─── Protocol fee (Issue #162) ────────────────────────────────────────────────
/// Hard cap on the optional protocol settlement fee, in basis points
/// (1 bp = 0.01%). 1_000 bp = 10% of the round's total pot — the maximum an
/// admin may ever schedule via timelock. Larger values would risk turning
/// the protocol into a de-facto extraction mechanism and are explicitly
/// disallowed to preserve user trust and the conservation invariant.
const MAX_PROTOCOL_FEE_BPS: u32 = 1_000;
/// Denominator for bps math: `fee = total_pot * bps / BPS_DENOMINATOR`.
/// Pinned to 10_000 to match the universal "1 bp = 0.01%" convention.
const BPS_DENOMINATOR: i128 = 10_000;

// ─── Storage schema versioning ───────────────────────────────────────────────
const CURRENT_SCHEMA_VERSION: u32 = 3;
// ─── Start-price bounds (Issue #119) ─────────────────────────────────────────
/// Minimum start price in protocol units — prevents zero-value and dust rounds.
const MIN_START_PRICE: u128 = 1;
/// Maximum start price in protocol units — guards against overflow in payout math.
const MAX_START_PRICE: u128 = 1_000_000_000_000_000_000;
// ─── Storage TTL Lifecycle Limits (Issue #142) ──────────────────────────────
/// Minimum remaining ledgers before a persistent entry is extended.
const TTL_BUMP_THRESHOLD: u32 = 17_280; // ~1 day at 5-second ledgers
/// Amount of ledgers to extend a persistent entry to when below threshold.
const TTL_BUMP_AMOUNT: u32 = 518_400; // ~30 days at 5-second ledgers

/// Default archived round summaries retained on-chain (FIFO pruning).
const DEFAULT_ARCHIVE_RETENTION: u32 = 128;
/// Minimum archive retention limit — prevents accidental pruning of all history.
const MIN_ARCHIVE_RETENTION: u32 = 1;
/// Maximum archive retention limit — prevents unbounded storage growth.
const MAX_ARCHIVE_RETENTION: u32 = 10_000;
/// Ledgers to wait before a scheduled critical config change may be applied (~2 hours).
const CONFIG_TIMELOCK_LEDGERS: u32 = 1440;

use crate::admin;
use crate::config;
use crate::betting;
use crate::settlement;
use crate::queries;
use crate::common;

#[contract]
pub struct VirtualTokenContract;

#[contractimpl]
impl VirtualTokenContract {
    /// Initializes the contract with admin and oracle addresses (one-time only)
    pub fn initialize(env: Env, admin: Address, oracle: Address) -> Result<(), ContractError> {
        admin.require_auth();

        if admin == oracle {
            return Err(ContractError::InvalidMode);
        }

        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(ContractError::AlreadyInitialized);
        }

        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Oracle, &oracle);
        env.storage()
            .persistent()
            .set(&DataKey::Paused, &RuntimeMode::Normal);
        env.storage()
            .persistent()
            .set(&DataKey::SchemaVersion, &CURRENT_SCHEMA_VERSION);

        // Set default window values
        env.storage()
            .persistent()
            .set(&DataKey::BetWindowLedgers, &DEFAULT_BET_WINDOW_LEDGERS);
        env.storage()
            .persistent()
            .set(&DataKey::RunWindowLedgers, &DEFAULT_RUN_WINDOW_LEDGERS);

        Self::_extend_persistent_ttl(&env, &DataKey::Admin);
        Self::_extend_persistent_ttl(&env, &DataKey::Oracle);
        Self::_extend_persistent_ttl(&env, &DataKey::Paused);
        Self::_extend_persistent_ttl(&env, &DataKey::SchemaVersion);
        Self::_extend_persistent_ttl(&env, &DataKey::BetWindowLedgers);
        Self::_extend_persistent_ttl(&env, &DataKey::RunWindowLedgers);

        Ok(())
        admin::initialize(env, admin, oracle)
    }

    /// Returns the stored schema version. If unset, returns legacy version 1.
    pub fn get_schema_version(env: Env) -> u32 {
        admin::get_schema_version(env)
    }

    /// Migrates legacy schema version 1 → version 2 (admin only).
    pub fn migrate_schema_v1_to_v2(env: Env) -> Result<(), ContractError> {
        let admin_key = DataKey::Admin;
        Self::_extend_persistent_ttl(&env, &admin_key);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&admin_key)
            .ok_or(ContractError::AdminNotSet)?;
        admin.require_auth();
        Self::_ensure_not_paused(&env).map_err(|e| {
            Self::_emit_action_rejected(&env, &admin, symbol_short!("migrate"), e);
            e
        })?;

        if env.storage().persistent().has(&DataKey::ActiveRound) {
            Self::_emit_action_rejected(
                &env,
                &admin,
                symbol_short!("migrate"),
                ContractError::MigrationActiveRound,
            );
            return Err(ContractError::MigrationActiveRound);
        }

        let from = Self::_schema_version(&env).unwrap_or(1);
        if from != 1 || CURRENT_SCHEMA_VERSION != 2 {
            return Err(ContractError::UnsupportedSchemaVersion);
        const TARGET_VERSION: u32 = 2;
        if from != 1 {
            Self::_emit_action_rejected(
                &env,
                &admin,
                symbol_short!("migrate"),
                ContractError::InvalidMigrationPath,
            );
            return Err(ContractError::InvalidMigrationPath);
        }

        let schema_key = DataKey::SchemaVersion;
        env.storage().persistent().set(&schema_key, &TARGET_VERSION);
        Self::_extend_persistent_ttl(&env, &schema_key);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("schema"), symbol_short!("migrated")),
            (from, TARGET_VERSION),
        );

        Ok(())
        admin::migrate_schema_v1_to_v2(env)
    }

    /// Migrates schema version 2 → version 3 (admin only).
    pub fn migrate_schema_v2_to_v3(env: Env) -> Result<(), ContractError> {
        admin::migrate_schema_v2_to_v3(env)
    }

    /// Returns whether the contract is currently paused
    pub fn is_paused(env: Env) -> bool {
        admin::is_paused(env)
    }

    /// Pauses the contract for emergency recovery (admin only)
    pub fn pause_contract(env: Env) -> Result<(), ContractError> {
        admin::pause_contract(env)
    }

    /// Unpauses the contract after recovery (admin only)
    pub fn unpause_contract(env: Env) -> Result<(), ContractError> {
        admin::unpause_contract(env)
    }

    /// Returns the current runtime mode (0 = Normal, 1 = ClaimsOnly, 2 = FullyPaused)
    pub fn get_runtime_mode(env: Env) -> u32 {
        admin::get_runtime_mode(env)
    }

    /// Sets the runtime mode of the contract (admin only)
    pub fn set_runtime_mode(env: Env, mode: u32) -> Result<(), ContractError> {
        Self::_require_supported_schema(&env)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ContractError::AdminNotSet)?;

        admin.require_auth();

        let new_mode = match mode {
            0 => RuntimeMode::Normal,
            1 => RuntimeMode::ClaimsOnly,
            2 => RuntimeMode::FullyPaused,
            _ => return Err(ContractError::InvalidMode),
        };

        Self::_set_mode(&env, new_mode)?;

        Ok(())
    }

    /// Creates a new prediction round (admin only)
    /// mode: 0 = Up/Down (default), 1 = Precision (Legends)
    pub fn create_round(
        env: Env,
        start_price: u128,
        mode: Option<u32>,
    ) -> Result<(), ContractError> {
        Self::_require_supported_schema(&env)?;
        if start_price < MIN_START_PRICE {
            return Err(ContractError::InvalidStartPrice);
        }
        if start_price > MAX_START_PRICE {
            return Err(ContractError::InvalidStartPrice);
        }

        // Default to Up/Down mode (0) if not specified
        let mode_value = mode.unwrap_or(0);

        // Validate mode is either 0 or 1
        if mode_value > 1 {
            return Err(ContractError::InvalidMode);
        }

        let round_mode = if mode_value == ROUND_MODE_UPDOWN {
            RoundMode::UpDown
        } else {
            RoundMode::Precision
        };

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ContractError::AdminNotSet)?;

        admin.require_auth();
        Self::_ensure_not_paused(&env).map_err(|e| {
            Self::_emit_action_rejected(&env, &admin, symbol_short!("create"), e);
            e
        })?;
        Self::assert_no_active_round(&env).map_err(|e| {
            Self::_emit_action_rejected(&env, &admin, symbol_short!("create"), e);
            e
        })?;

        // Get configured windows (with defaults)
        Self::_extend_persistent_ttl(&env, &DataKey::BetWindowLedgers);
        let bet_ledgers: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::BetWindowLedgers)
            .unwrap_or(DEFAULT_BET_WINDOW_LEDGERS);
        Self::_extend_persistent_ttl(&env, &DataKey::RunWindowLedgers);
        let run_ledgers: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::RunWindowLedgers)
            .unwrap_or(DEFAULT_RUN_WINDOW_LEDGERS);

        // Generate unique round ID
        Self::_extend_persistent_ttl(&env, &DataKey::LastRoundId);
        let last_round_id: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::LastRoundId)
            .unwrap_or(0);
        let round_id = last_round_id
            .checked_add(1)
            .ok_or(ContractError::Overflow)?;
        env.storage()
            .persistent()
            .set(&DataKey::LastRoundId, &round_id);
        Self::_extend_persistent_ttl(&env, &DataKey::LastRoundId);

        let start_ledger = env.ledger().sequence();
        let bet_end_ledger = start_ledger
            .checked_add(bet_ledgers)
            .ok_or(ContractError::Overflow)?;
        let end_ledger = start_ledger
            .checked_add(run_ledgers)
            .ok_or(ContractError::Overflow)?;

        let round = Round {
            round_id,
            price_start: start_price,
            start_ledger,
            bet_end_ledger,
            end_ledger,
            pool_up: 0,
            pool_down: 0,
            mode: round_mode.clone(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::ActiveRound, &round);
        Self::_extend_persistent_ttl(&env, &DataKey::ActiveRound);

        // Note: individual position keys (DataKey::Position / DataKey::PrecisionPosition)
        // are cleaned up at resolve time; no bulk-map clearing needed here.

        // Emit round creation event with round ID and mode
        // Topic: ("round", "created")
        // Payload: (round_id: u64, start_price: u128, start_ledger: u32, bet_end_ledger: u32, end_ledger: u32, mode: u32)
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("round"), symbol_short!("created")),
            (
                round_id,
                start_price,
                start_ledger,
                bet_end_ledger,
                end_ledger,
                mode_value,
            ),
        );

        Ok(())
    }

    /// Returns the currently active round, if any
    pub fn get_active_round(env: Env) -> Option<Round> {
        env.storage().persistent().get(&DataKey::ActiveRound)
    }
    /// Returns live pool-composition metrics for the currently active round.
    pub fn get_round_pool_stats(env: Env) -> Option<RoundPoolStats> {
        let round: Round = env.storage().persistent().get(&DataKey::ActiveRound)?;
        let participants_key = DataKey::RoundParticipants(round.round_id);
        let participants: Vec<Address> = env
            .storage()
            .persistent()
            .get(&participants_key)
            .unwrap_or(Vec::new(&env));

        let mut stats = RoundPoolStats {
            round_id: round.round_id,
            mode: round.mode.clone(),
            total_up_stake: 0,
            total_down_stake: 0,
            up_participant_count: 0,
            down_participant_count: 0,
            up_stake_ratio_bps: 0,
            down_stake_ratio_bps: 0,
            precision_total_stake: 0,
            precision_participant_count: 0,
            precision_prediction_count: 0,
            precision_commitment_count: 0,
            precision_revealed_count: 0,
        };

        match round.mode {
            RoundMode::UpDown => {
                stats.total_up_stake = round.pool_up;
                stats.total_down_stake = round.pool_down;

                let mut idx = 0;
                while idx < participants.len() {
                    if let Some(user) = participants.get(idx) {
                        if let Some(position) = env
                            .storage()
                            .persistent()
                            .get::<_, UserPosition>(&DataKey::Position(round.round_id, user))
                        {
                            match position.side {
                                BetSide::Up => stats.up_participant_count += 1,
                                BetSide::Down => stats.down_participant_count += 1,
                            }
                        }
                    }
                    idx += 1;
                }

                let total_stake = round.pool_up.checked_add(round.pool_down).unwrap_or(0);
                if total_stake > 0 {
                    stats.up_stake_ratio_bps = ((round.pool_up as u128)
                        .saturating_mul(BPS_DENOMINATOR as u128)
                        / total_stake as u128) as u32;
                    stats.down_stake_ratio_bps = ((round.pool_down as u128)
                        .saturating_mul(BPS_DENOMINATOR as u128)
                        / total_stake as u128) as u32;
                }
            }
            RoundMode::Precision => {
                stats.precision_participant_count = participants.len();

                let mut idx = 0;
                while idx < participants.len() {
                    if let Some(user) = participants.get(idx) {
                        if let Some(prediction) =
                            env.storage().persistent().get::<_, PrecisionPrediction>(
                                &DataKey::PrecisionPosition(round.round_id, user.clone()),
                            )
                        {
                            stats.precision_prediction_count += 1;
                            stats.precision_total_stake += prediction.amount;
                        } else if let Some(commitment) =
                            env.storage().persistent().get::<_, PrecisionCommitment>(
                                &DataKey::PrecisionCommitment(round.round_id, user),
                            )
                        {
                            stats.precision_commitment_count += 1;
                            stats.precision_total_stake += commitment.amount;
                            if commitment.revealed {
                                stats.precision_revealed_count += 1;
                            }
                        }
                    }
                    idx += 1;
                }
            }
        }

        Some(stats)
    }

    /// Returns the current lifecycle phase of the active round.
    ///
    /// Phase boundaries are deterministic:
    /// - `Betting` while `ledger < bet_end_ledger`
    /// - `Running` while `bet_end_ledger ≤ ledger < end_ledger`
    /// - `Resolvable` when `ledger ≥ end_ledger`
    ///
    /// Returns [`ContractError::NoActiveRound`] when no round is active.
    pub fn get_round_phase(env: Env) -> Result<RoundPhase, ContractError> {
        let round = env
            .storage()
            .persistent()
            .get::<_, Round>(&DataKey::ActiveRound)
            .ok_or(ContractError::NoActiveRound)?;
        Ok(Self::_derive_round_phase(env.ledger().sequence(), &round))
    }

    pub fn get_admin(env: Env) -> Option<Address> {
        admin::get_admin(env)
    }

    pub fn get_oracle(env: Env) -> Option<Address> {
        admin::get_oracle(env)
    }

    /// Schedules a timelocked oracle deviation update
    pub fn set_oracle_max_deviation_bps(env: Env, bps: Option<u32>) -> Result<(), ContractError> {
        admin::set_oracle_max_deviation_bps(env, bps)
    }

    /// Returns the configured oracle max deviation bps, if set.
    pub fn get_oracle_max_deviation_bps(env: Env) -> Option<u32> {
        admin::get_oracle_max_deviation_bps(env)
    }

    /// Arms a one-shot override to bypass deviation checks for the next settlement (admin only).
    pub fn arm_oracle_deviation_override(env: Env) -> Result<(), ContractError> {
        admin::arm_oracle_deviation_override(env)
    }

    /// Sets the minimum oracle confidence threshold in basis points (admin only).
    pub fn set_oracle_min_confidence_bps(
        env: Env,
        min_bps: Option<u32>,
    ) -> Result<(), ContractError> {
        admin::set_oracle_min_confidence_bps(env, min_bps)
    }

    /// Enables or disables strict mode for oracle confidence (admin only).
    pub fn set_oracle_strict_mode(env: Env, enabled: bool) -> Result<(), ContractError> {
        admin::set_oracle_strict_mode(env, enabled)
    }

    /// Returns the configured minimum oracle confidence bps, if set.
    pub fn get_oracle_min_confidence_bps(env: Env) -> Option<u32> {
        admin::get_oracle_min_confidence_bps(env)
    }

    /// Returns whether oracle strict mode is enabled.
    pub fn get_oracle_strict_mode(env: Env) -> bool {
        admin::get_oracle_strict_mode(env)
    }

    /// Records an oracle heartbeat (oracle only).
    pub fn update_oracle_heartbeat(env: Env, status: u32) -> Result<(), ContractError> {
        admin::update_oracle_heartbeat(env, status)
    }

    /// Returns the most recent oracle heartbeat record, if any.
    pub fn get_oracle_heartbeat(env: Env) -> Option<OracleHeartbeatRecord> {
        admin::get_oracle_heartbeat(env)
    }

    /// Returns `true` if the oracle has a non-stale heartbeat with status not offline (2).
    pub fn is_oracle_live(env: Env) -> bool {
        admin::is_oracle_live(env)
    }

    /// Schedules a timelocked stale threshold update
    pub fn set_oracle_stale_threshold(env: Env, seconds: u64) -> Result<(), ContractError> {
        admin::set_oracle_stale_threshold(env, seconds)
    }

    /// Returns a composite protocol health status
    pub fn get_protocol_health(env: Env) -> ProtocolHealthStatus {
        admin::get_protocol_health(env)
    }

    /// Returns the configured oracle stale threshold, or the default if not set.
    /// Returns the global status of the protocol.
    ///
    /// This is the canonical single-call status endpoint for frontends and
    /// monitoring dashboards. The returned [`ProtocolStatus`] maps directly to
    /// the three mutually-exclusive states visible to end users:
    ///
    /// | return value      | meaning                                             |
    /// |-------------------|-----------------------------------------------------|
    /// | `Active`      (0) | A round is live; bets or reveals are accepted.      |
    /// | `Paused`      (1) | Emergency pause active; mutations rejected.          |
    /// | `ClaimsOnly`  (2) | No active round; only `claim_winnings` is useful.   |
    ///
    /// **Priority**: `Paused` is always returned first when the contract is
    /// paused, regardless of whether an active round exists.
    pub fn get_protocol_status(env: Env) -> ProtocolStatus {
        if Self::is_paused(env.clone()) {
            ProtocolStatus::Paused
        } else if env.storage().persistent().has(&DataKey::ActiveRound) {
            ProtocolStatus::Active
        } else {
            ProtocolStatus::ClaimsOnly
        }
    }

    /// Returns the status of a specific round identified by `round_id`.
    ///
    /// Lookup strategy (in priority order):
    /// 1. If the round is the **current active round**, derive status from
    ///    ledger position relative to `bet_end_ledger` / `end_ledger`.
    /// 2. If the round appears in the **on-chain archive**, map its
    ///    [`RoundArchiveStatus`] to the corresponding terminal [`RoundStatus`].
    /// 3. If a `CancelledRound` marker exists (archive may be pruned),
    ///    return `Cancelled`.
    /// 4. Otherwise, return `Unknown`.
    ///
    /// | return value          | meaning                                                       |
    /// |-----------------------|---------------------------------------------------------------|
    /// | `Unknown`        (0)  | Round not found; never created or pruned from archive.       |
    /// | `Betting`        (1)  | Active; `ledger < bet_end_ledger`.                           |
    /// | `Running`        (2)  | Active; `bet_end_ledger ≤ ledger < end_ledger`.              |
    /// | `AwaitingResolve`(3)  | Active; `ledger ≥ end_ledger`, oracle not yet called.        |
    /// | `Resolved`       (4)  | Settled normally; pot distributed.                           |
    /// | `Cancelled`      (5)  | Admin-cancelled; stakes refunded.                            |
    /// | `FallbackRefund` (6)  | Settled with insufficient participants; stakes refunded.     |
    ///
    /// Note: `Betting`, `Running`, and `AwaitingResolve` are **derived** from
    /// ledger sequence — they do not involve additional storage writes.
    pub fn get_round_status(env: Env, round_id: u64) -> RoundStatus {
        // First check if it is the active round
        if let Some(active_round) = env
            .storage()
            .persistent()
            .get::<_, Round>(&DataKey::ActiveRound)
        {
            if active_round.round_id == round_id {
                let phase = Self::_derive_round_phase(env.ledger().sequence(), &active_round);
                return match phase {
                    RoundPhase::Betting => RoundStatus::Betting,
                    RoundPhase::Running => RoundStatus::Running,
                    RoundPhase::Resolvable => RoundStatus::AwaitingResolve,
                };
            }
        }

        // Second, check the archived rounds summary
        let archive_key = DataKey::ArchivedRound(round_id);
        if let Some(archive) = env
            .storage()
            .persistent()
            .get::<_, ArchivedRoundSummary>(&archive_key)
        {
            return match archive.status {
                RoundArchiveStatus::Resolved => RoundStatus::Resolved,
                RoundArchiveStatus::Cancelled => RoundStatus::Cancelled,
                RoundArchiveStatus::FallbackRefund => RoundStatus::FallbackRefund,
            };
        }

        // Third, fallback check for cancelled rounds (in case it was pruned but CancelledRound flag remains)
        if Self::is_round_cancelled(env.clone(), round_id) {
            return RoundStatus::Cancelled;
        }

        // Otherwise, it's not active, not in archive, not cancelled.
        RoundStatus::Unknown
    }

    /// Returns the configured oracle stale threshold, or the default (3600 s) if not set.
    pub fn get_oracle_stale_threshold(env: Env) -> u64 {
        admin::get_oracle_stale_threshold(env)
    }

    // ─── Oracle rotation (two-step with expiry) ─────────────────────────────

    /// Proposes a new oracle address with an expiry window (admin only).
    ///
    /// The proposal must be accepted via [`Self::accept_oracle_rotation`] before
    /// `expires_in_seconds` elapses, otherwise acceptance is rejected.
    /// Minimum expiry is 60 seconds.
    ///
    /// Emits `("oracle", "propose")`.
    pub fn propose_oracle_rotation(
        env: Env,
        new_oracle: Address,
        expires_in_seconds: u64,
    ) -> Result<(), ContractError> {
        Self::_require_supported_schema(&env)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ContractError::AdminNotSet)?;
        admin.require_auth();
        Self::_ensure_not_paused(&env)?;

        if expires_in_seconds < MIN_ROTATION_EXPIRY_SECONDS {
            return Err(ContractError::InvalidStaleThreshold);
        }

        let proposed_at = env.ledger().timestamp();
        let expires_at = proposed_at
            .checked_add(expires_in_seconds)
            .ok_or(ContractError::Overflow)?;

        let proposal = OracleRotationProposal {
            new_oracle: new_oracle.clone(),
            proposed_at,
            expires_at,
        };

        let key = DataKey::OracleRotationProposal;
        env.storage().persistent().set(&key, &proposal);
        Self::_extend_persistent_ttl(&env, &key);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("propose")),
            (new_oracle, expires_at),
        );

        Ok(())
    }

    /// Accepts a pending oracle rotation proposal before expiry (any caller).
    ///
    /// If the proposal has expired the call returns `RotationExpired` and the
    /// stale proposal is removed after emitting `("oracle", "expired")`.
    /// On success the stored oracle address is updated and
    /// `("oracle", "accept")` is emitted.
    pub fn accept_oracle_rotation(env: Env) -> Result<(), ContractError> {
        Self::_require_supported_schema(&env)?;
        Self::_ensure_not_paused(&env)?;

        let key = DataKey::OracleRotationProposal;
        let proposal: OracleRotationProposal = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::NoPendingRotation)?;

        let current_ts = env.ledger().timestamp();

        if current_ts > proposal.expires_at {
            env.storage().persistent().remove(&key);
            #[allow(deprecated)]
            env.events().publish(
                (symbol_short!("oracle"), symbol_short!("expired")),
                (
                    proposal.new_oracle,
                    proposal.proposed_at,
                    proposal.expires_at,
                ),
            );
            return Err(ContractError::RotationExpired);
        }

        let oracle_key = DataKey::Oracle;
        let previous: Address = env
            .storage()
            .persistent()
            .get(&oracle_key)
            .ok_or(ContractError::OracleNotSet)?;

        env.storage()
            .persistent()
            .set(&oracle_key, &proposal.new_oracle);
        Self::_extend_persistent_ttl(&env, &oracle_key);
        env.storage().persistent().remove(&key);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("accept")),
            (previous, proposal.new_oracle),
        );

        Ok(())
    }

    /// Cancels a pending oracle rotation proposal before it expires (admin only).
    ///
    /// Emits `("oracle", "cancel")` on success.
    pub fn cancel_oracle_rotation(env: Env) -> Result<(), ContractError> {
        Self::_require_supported_schema(&env)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ContractError::AdminNotSet)?;
        admin.require_auth();
        Self::_ensure_not_paused(&env)?;

        let key = DataKey::OracleRotationProposal;
        let proposal: OracleRotationProposal = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::NoPendingRotation)?;

        env.storage().persistent().remove(&key);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("cancel")),
            (proposal.new_oracle,),
        );

        Ok(())
    }

    /// Returns the pending oracle rotation proposal, if any.
    pub fn get_oracle_rotation_proposal(env: Env) -> Option<OracleRotationProposal> {
        let key = DataKey::OracleRotationProposal;
        Self::_extend_persistent_ttl(&env, &key);
        env.storage().persistent().get(&key)
    }

    /// Schedules a timelocked windows update (alias for [`Self::schedule_windows`]).
    /// bet_ledgers: Number of ledgers users can place bets
    /// run_ledgers: Total number of ledgers before round can be resolved
    pub fn set_windows(env: Env, bet_ledgers: u32, run_ledgers: u32) -> Result<(), ContractError> {
        config::set_windows(env, bet_ledgers, run_ledgers)
    }

    pub fn set_max_stake(env: Env, max_amount: Option<i128>) -> Result<(), ContractError> {
        config::set_max_stake(env, max_amount)
    }

    pub fn get_max_stake(env: Env) -> Option<i128> {
        config::get_max_stake(env)
    }

    pub fn set_max_user_exposure(
        env: Env,
        max_exposure: Option<i128>,
    ) -> Result<(), ContractError> {
        config::set_max_user_exposure(env, max_exposure)
    }

    pub fn get_max_user_exposure(env: Env) -> Option<i128> {
        config::get_max_user_exposure(env)
    }

    pub fn set_max_pending_winnings(
        env: Env,
        max_pending: Option<i128>,
    ) -> Result<(), ContractError> {
        config::set_max_pending_winnings(env, max_pending)
    }

    pub fn schedule_windows(
        env: Env,
        bet_ledgers: u32,
        run_ledgers: u32,
    ) -> Result<(), ContractError> {
        config::schedule_windows(env, bet_ledgers, run_ledgers)
    }

    pub fn schedule_max_stake(env: Env, max_amount: Option<i128>) -> Result<(), ContractError> {
        config::schedule_max_stake(env, max_amount)
    }

    pub fn schedule_max_user_exposure(
        env: Env,
        max_exposure: Option<i128>,
    ) -> Result<(), ContractError> {
        config::schedule_max_user_exposure(env, max_exposure)
    }

    pub fn schedule_max_pending_winnings(
        env: Env,
        max_pending: Option<i128>,
    ) -> Result<(), ContractError> {
        config::schedule_max_pending_winnings(env, max_pending)
    }

    pub fn schedule_oracle_stale_threshold(env: Env, seconds: u64) -> Result<(), ContractError> {
        config::schedule_oracle_stale_threshold(env, seconds)
    }

    pub fn schedule_oracle_deviation_bps(env: Env, bps: Option<u32>) -> Result<(), ContractError> {
        config::schedule_oracle_deviation_bps(env, bps)
    }

    pub fn schedule_protocol_fee_bps(env: Env, bps: Option<u32>) -> Result<(), ContractError> {
        config::schedule_protocol_fee_bps(env, bps)
    }

    pub fn set_protocol_fee_bps(env: Env, bps: Option<u32>) -> Result<(), ContractError> {
        config::set_protocol_fee_bps(env, bps)
    }

    pub fn get_protocol_fee_bps(env: Env) -> Option<u32> {
        config::get_protocol_fee_bps(env)
    }

    pub fn get_protocol_fee_treasury(env: Env) -> i128 {
        config::get_protocol_fee_treasury(env)
    }

    pub fn withdraw_protocol_fee(
        env: Env,
        recipient: Address,
        amount: i128,
    ) -> Result<i128, ContractError> {
        Self::_require_supported_schema(&env)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ContractError::AdminNotSet)?;
        admin.require_auth();
        Self::_ensure_not_paused(&env).map_err(|e| {
            Self::_emit_action_rejected(&env, &admin, symbol_short!("withdraw"), e);
            e
        })?;

        if amount <= 0 {
            return Err(ContractError::InvalidBetAmount);
        }

        let treasury_key = DataKey::ProtocolFeeTreasury;
        let current: i128 = env.storage().persistent().get(&treasury_key).unwrap_or(0);
        let new_treasury = current
            .checked_sub(amount)
            .ok_or(ContractError::Overflow)?;
        env.storage().persistent().set(&treasury_key, &new_treasury);
        Self::_extend_persistent_ttl(&env, &treasury_key);

        // Credit recipient — reuse the existing balance helper. create a
        // balance row if recipient has none yet (treasury recipient may
        // not have minted).
        let recipient_bal: i128 = Self::balance(env.clone(), recipient.clone());
        let new_bal = Self::payout_add(recipient_bal, amount)?;
        Self::_set_balance(&env, recipient.clone(), new_bal);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("protocol"), symbol_short!("withdrawn")),
            (recipient, amount, new_treasury),
        );

        Ok(amount)
        config::withdraw_protocol_fee(env, recipient, amount)
    }

    pub fn get_pending_config_change(
        env: Env,
        kind: ConfigChangeKind,
    ) -> Option<PendingConfigChange> {
        config::get_pending_config_change(env, kind)
    }

    pub fn apply_scheduled_changes(env: Env, kind: ConfigChangeKind) -> Result<(), ContractError> {
        config::apply_scheduled_changes(env, kind)
    }

    pub fn cancel_config_change(env: Env, kind: ConfigChangeKind) -> Result<(), ContractError> {
        config::cancel_config_change(env, kind)
    }

    pub fn get_max_pending_winnings(env: Env) -> Option<i128> {
        config::get_max_pending_winnings(env)
    }

    pub fn set_min_participants(env: Env, min: Option<u32>) -> Result<(), ContractError> {
        config::set_min_participants(env, min)
    }

    pub fn get_min_participants(env: Env) -> Option<u32> {
        config::get_min_participants(env)
    }

    pub fn set_max_precision_participants(env: Env, max: u32) -> Result<(), ContractError> {
        config::set_max_precision_participants(env, max)
    }

    pub fn get_max_precision_participants(env: Env) -> u32 {
        config::get_max_precision_participants(env)
    }

    pub fn set_mint_limit(env: Env, limit: u32) -> Result<(), ContractError> {
        config::set_mint_limit(env, limit)
    }

    pub fn get_mint_limit(env: Env) -> u32 {
        config::get_mint_limit(env)
    }

    pub fn set_archive_retention(env: Env, limit: u32) -> Result<(), ContractError> {
        config::set_archive_retention(env, limit)
    }

    pub fn get_archive_retention(env: Env) -> u32 {
        config::get_archive_retention(env)
    }

    /// Creates a new prediction round (admin only)
    pub fn create_round(
        env: Env,
        start_price: u128,
        mode: Option<u32>,
    ) -> Result<(), ContractError> {
        betting::create_round(env, start_price, mode)
    }

    pub fn place_bet(
        env: Env,
        user: Address,
        amount: i128,
        side: BetSide,
    ) -> Result<(), ContractError> {
        betting::place_bet(env, user, amount, side)
    }

    pub fn place_precision_prediction(
        env: Env,
        user: Address,
        amount: i128,
        predicted_price: u128,
    ) -> Result<(), ContractError> {
        Self::_require_supported_schema(&env)?;
        user.require_auth();
        Self::_ensure_normal_mode(&env)?;

        if amount <= 0 {
            return Err(ContractError::InvalidBetAmount);
        }

        // Enforce max stake cap (Issue #113)
        if let Some(max_stake) = env
            .storage()
            .persistent()
            .get::<_, i128>(&DataKey::MaxStake)
        {
            if amount > max_stake {
                return Err(ContractError::StakeExceedsMax);
            }
        }

        // Validate price scale (must be 4 decimal places, max value 9999 for 0.9999)
        // Reasonable max: 99999999 (9999.9999 XLM)
        if predicted_price > 99_999_999 {
            return Err(ContractError::InvalidPrice);
        }

        // Single read of the active round — cache in call scope
        let round: Round = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveRound)
            .ok_or(ContractError::NoActiveRound)?;

        // Enforce per-user round exposure cap (Issue #113)
        if let Some(max_exposure) = env
            .storage()
            .persistent()
            .get::<_, i128>(&DataKey::MaxUserRoundExposure)
        {
            if amount > max_exposure {
                return Err(ContractError::ExposureCapExceeded);
            }
        }

        // Verify round is in Precision mode
        if round.mode != RoundMode::Precision {
            return Err(ContractError::WrongModeForPrediction);
        }

        let current_ledger = env.ledger().sequence();
        if current_ledger >= round.bet_end_ledger {
            return Err(ContractError::RoundEnded);
        }

        // O(1) duplicate-prediction check — single composite key read
        let pred_key = DataKey::PrecisionPosition(round.round_id, user.clone());
        let commit_key = DataKey::PrecisionCommitment(round.round_id, user.clone());
        if env.storage().persistent().has(&pred_key) || env.storage().persistent().has(&commit_key)
        {
            return Err(ContractError::AlreadyBet);
        }

        let participants_key = DataKey::RoundParticipants(round.round_id);
        let mut participants: Vec<Address> = env
            .storage()
            .persistent()
            .get(&participants_key)
            .unwrap_or(Vec::new(&env));
        let max_precision_participants = Self::get_max_precision_participants(env.clone());
        if participants.len() >= max_precision_participants {
            return Err(ContractError::PrecisionParticipantCapExceeded);
        }

        let user_balance = Self::balance(env.clone(), user.clone());
        if user_balance < amount {
            return Err(ContractError::InsufficientBalance);
        }

        // Deduct balance
        let new_balance = user_balance
            .checked_sub(amount)
            .ok_or(ContractError::Overflow)?;
        Self::_set_balance(&env, user.clone(), new_balance);

        // Write single-user prediction key — O(1), constant-size entry
        let prediction = PrecisionPrediction {
            user: user.clone(),
            predicted_price,
            amount,
        };
        env.storage().persistent().set(&pred_key, &prediction);

        // Append to shared participant list
        participants.push_back(user.clone());
        env.storage()
            .persistent()
            .set(&participants_key, &participants);

        // Emit event for precision prediction
        // Topic: ("predict", "price")
        // Payload: (user: Address, round_id: u64, predicted_price: u128, amount: i128)
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("predict"), symbol_short!("price")),
            (user, round.round_id, predicted_price, amount),
        );

        Ok(())
        betting::place_precision_prediction(env, user, amount, predicted_price)
    }

    pub fn predict_price(
        env: Env,
        user: Address,
        guessed_price: u128,
        amount: i128,
    ) -> Result<(), ContractError> {
        betting::predict_price(env, user, guessed_price, amount)
    }

    pub fn commit_prediction(
        env: Env,
        user: Address,
        hash: BytesN<32>,
        amount: i128,
    ) -> Result<(), ContractError> {
        betting::commit_prediction(env, user, hash, amount)
    }

    pub fn reveal_prediction(
        env: Env,
        user: Address,
        predicted_price: u128,
        salt: BytesN<32>,
    ) -> Result<(), ContractError> {
        betting::reveal_prediction(env, user, predicted_price, salt)
    }

    /// Mints 1000 vXLM for new users (one-time only)
    pub fn mint_initial(env: Env, user: Address) -> i128 {
        betting::mint_initial(env, user)
    }

    pub fn resolve_round(env: Env, payload: OraclePayload) -> Result<(), ContractError> {
        settlement::resolve_round(env, payload)
    }

    pub fn cancel_round(env: Env, reason: u32) -> Result<(), ContractError> {
        settlement::cancel_round(env, reason)
    }

    pub fn is_round_cancelled(env: Env, round_id: u64) -> bool {
        settlement::is_round_cancelled(env, round_id)
    }

    pub fn claim_winnings(env: Env, user: Address) -> Result<i128, ContractError> {
        settlement::claim_winnings(env, user)
    }

    pub fn get_active_round(env: Env) -> Option<Round> {
        queries::get_active_round(env)
    }

    pub fn get_round_pool_stats(env: Env) -> Option<RoundPoolStats> {
        queries::get_round_pool_stats(env)
    }

    pub fn get_round_phase(env: Env) -> Result<RoundPhase, ContractError> {
        queries::get_round_phase(env)
    }

    pub fn get_last_round_id(env: Env) -> u64 {
        queries::get_last_round_id(env)
    }

    pub fn get_archived_round(env: Env, round_id: u64) -> Option<ArchivedRoundSummary> {
        queries::get_archived_round(env, round_id)
    }

    pub fn get_recent_archived_rounds(env: Env, limit: u32) -> Vec<ArchivedRoundSummary> {
        queries::get_recent_archived_rounds(env, limit)
    }

    pub fn get_user_archived_participation(
        env: Env,
        user: Address,
        round_id: u64,
    ) -> Option<UserRoundOutcome> {
        queries::get_user_archived_participation(env, user, round_id)
    }

    pub fn get_user_stats(env: Env, user: Address) -> UserStats {
        queries::get_user_stats(env, user)
    }

    pub fn get_pending_winnings(env: Env, user: Address) -> i128 {
        queries::get_pending_winnings(env, user)
    }

    pub fn get_user_position(env: Env, user: Address) -> Option<UserPosition> {
        queries::get_user_position(env, user)
    }

    pub fn get_user_precision_prediction(env: Env, user: Address) -> Option<PrecisionPrediction> {
        queries::get_user_precision_prediction(env, user)
    }

    pub fn get_precision_predictions(env: Env) -> Vec<PrecisionPrediction> {
        queries::get_precision_predictions(env)
    }

    pub fn get_updown_positions(env: Env) -> Map<Address, UserPosition> {
        queries::get_updown_positions(env)
    }

    pub fn get_precision_predictions_page(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Vec<PrecisionPrediction> {
        queries::get_precision_predictions_page(env, offset, limit)
    }

    pub fn get_updown_positions_page(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Vec<(Address, UserPosition)> {
        queries::get_updown_positions_page(env, offset, limit)
    }

    /// Returns user's vXLM balance
    pub fn balance(env: Env, user: Address) -> i128 {
        common::balance(env, user)
    }
}

        let participants: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::RoundParticipants(round.round_id))
            .unwrap_or(Vec::new(&env));
        let participants = Self::sort_addresses(participants);

        let total = participants.len();
        if offset >= total {
            return Vec::new(&env);
        }

        let end = offset.saturating_add(limit).min(total);

        let mut result: Vec<(Address, UserPosition)> = Vec::new(&env);
        for i in offset..end {
            if let Some(user) = participants.get(i) {
                let pos_key = DataKey::Position(round.round_id, user.clone());
                if let Some(pos) = env.storage().persistent().get(&pos_key) {
                    result.push_back((user, pos));
                }
            }
        }

        result
    }

    /// Resolves the round with oracle payload (oracle only)
    /// Mode 0 (Up/Down): Winners split losers' pool proportionally; ties get refunds
    /// Mode 1 (Precision/Legends): Closest guess wins full pot; ties split evenly
    pub(crate) fn _set_balance(env: &Env, user: Address, amount: i128) {
        let key = DataKey::Balance(user);
        env.storage().persistent().set(&key, &amount);
        Self::_extend_persistent_ttl(env, &key);
    }

    fn _ensure_not_paused(env: &Env) -> Result<(), ContractError> {
        let key = DataKey::Paused;
        Self::_extend_persistent_ttl(env, &key);
        let mode = env
            .storage()
            .persistent()
            .get::<_, RuntimeMode>(&key)
            .unwrap_or(RuntimeMode::Normal);
        if mode == RuntimeMode::FullyPaused {
            return Err(ContractError::ContractPaused);
        }
        Ok(())
    }

    fn _ensure_normal_mode(env: &Env) -> Result<(), ContractError> {
        let key = DataKey::Paused;
        Self::_extend_persistent_ttl(env, &key);
        let mode = env
            .storage()
            .persistent()
            .get::<_, RuntimeMode>(&key)
            .unwrap_or(RuntimeMode::Normal);
        if mode != RuntimeMode::Normal {
            return Err(ContractError::ContractPaused);
        }
        Ok(())
    }

    fn _set_mode(env: &Env, new_mode: RuntimeMode) -> Result<(), ContractError> {
        let key = DataKey::Paused;
        let old_mode = env
            .storage()
            .persistent()
            .get::<_, RuntimeMode>(&key)
            .unwrap_or(RuntimeMode::Normal);
        if old_mode != new_mode {
            env.storage().persistent().set(&key, &new_mode);
            Self::_extend_persistent_ttl(env, &key);
            #[allow(deprecated)]
            env.events().publish(
                (symbol_short!("mode"), Symbol::new(env, "transition")),
                (old_mode as u32, new_mode as u32),
            );
        }
        Ok(())
    }

    /// Derives the round lifecycle phase for `round` at `ledger_sequence`.
    fn _derive_round_phase(ledger_sequence: u32, round: &Round) -> RoundPhase {
        if ledger_sequence < round.bet_end_ledger {
            RoundPhase::Betting
        } else if ledger_sequence < round.end_ledger {
            RoundPhase::Running
        } else {
            RoundPhase::Resolvable
        }
    }

    fn _schema_version(env: &Env) -> Option<u32> {
        env.storage().persistent().get(&DataKey::SchemaVersion)
    }

    fn _require_supported_schema(env: &Env) -> Result<u32, ContractError> {
        Self::_extend_persistent_ttl(env, &DataKey::SchemaVersion);
        if env.storage().persistent().has(&DataKey::Admin) {
            Self::_extend_persistent_ttl(env, &DataKey::Admin);
        }
        let v = Self::_schema_version(env).unwrap_or(1);
        if v == 0 || v > CURRENT_SCHEMA_VERSION {
            return Err(ContractError::UnsupportedSchemaVersion);
        }
        Ok(v)
    }

    fn assert_no_active_round(env: &Env) -> Result<(), ContractError> {
        if env.storage().persistent().has(&DataKey::ActiveRound) {
            return Err(ContractError::RoundAlreadyActive);
        }

        Ok(())
    }

    /// Checked addition for payout accumulation.
    ///
    /// All payout aggregation (refunds, winnings, precision payouts) routes
    /// through this helper so overflow always maps to the stable
    /// `PayoutOverflow` variant rather than a generic `Overflow`. This makes
    /// the failure mode auditable and distinguishable from non-financial
    /// overflow (e.g. round-ID counter, ledger arithmetic).
    ///
    /// All-or-nothing guarantee: callers must not mutate storage before all
    /// payout math is complete and checked. The functions below enforce this
    /// by computing the new value first and only writing it afterward.
    #[inline(always)]
    fn payout_add(a: i128, b: i128) -> Result<i128, ContractError> {
        a.checked_add(b).ok_or(ContractError::PayoutOverflow)
    }

    #[inline(always)]
    fn payout_mul(a: i128, b: i128) -> Result<i128, ContractError> {
        a.checked_mul(b).ok_or(ContractError::PayoutOverflow)
    }

    fn _emit_payout_outcome(
        env: &Env,
        round_id: u64,
        mode: u32,
        user: Address,
        gross_payout: i128,
        outcome_type: u32,
    ) {
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("payout"), symbol_short!("outcome")),
            (round_id, mode, user, gross_payout, outcome_type),
        );
    }

    /// Accumulates `amount` into a user's pending winnings, enforcing the cap if set (Issue #120).
    ///
    /// Reads and writes `DataKey::PendingWinnings(user)` in one place, ensuring the cap
    /// check and overflow protection are applied consistently across all payout paths.
    fn _accumulate_pending(env: &Env, user: Address, amount: i128) -> Result<(), ContractError> {
        let key = DataKey::PendingWinnings(user);
        let existing: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        let new_pending = Self::payout_add(existing, amount)?;

        // Enforce pending winnings cap if configured
        if let Some(cap) = env
            .storage()
            .persistent()
            .get::<_, i128>(&DataKey::MaxPendingWinnings)
        {
            if new_pending > cap {
                return Err(ContractError::PendingWinningsCapExceeded);
            }
        }

        env.storage().persistent().set(&key, &new_pending);
        Self::_extend_persistent_ttl(env, &key);
        Ok(())
    }

    fn _validate_windows(bet_ledgers: u32, run_ledgers: u32) -> Result<(), ContractError> {
        if bet_ledgers == 0 || run_ledgers == 0 {
            return Err(ContractError::InvalidDuration);
        }
        if bet_ledgers > MAX_BET_WINDOW_LEDGERS || run_ledgers > MAX_RUN_WINDOW_LEDGERS {
            return Err(ContractError::WindowOutOfRange);
        }
        if bet_ledgers >= run_ledgers {
            return Err(ContractError::InvalidDuration);
        }
        Ok(())
    }

    fn _validate_max_stake(max_amount: Option<i128>) -> Result<(), ContractError> {
        if let Some(v) = max_amount {
            if v < MIN_CAP_VALUE {
                return Err(ContractError::InvalidBetAmount);
            }
        }
        Ok(())
    }

    fn _validate_oracle_stale_threshold(seconds: u64) -> Result<(), ContractError> {
        if !(MIN_ORACLE_STALE_THRESHOLD..=MAX_ORACLE_STALE_THRESHOLD).contains(&seconds) {
            return Err(ContractError::InvalidStaleThreshold);
        }
        Ok(())
    }

    fn _validate_oracle_max_deviation_bps(bps: Option<u32>) -> Result<(), ContractError> {
        if let Some(v) = bps {
            if v == 0 || v > MAX_ORACLE_DEVIATION_BPS {
                return Err(ContractError::InvalidOracleDeviationBps);
            }
        }
        Ok(())
    }

    /// Validates a requested protocol-fee bps (Issue #162).
    /// `None` always allowed (disables fee entirely, restoring pre-#162
    /// byte-for-byte behaviour). `Some(0)` is rejected — only explicit `None`
    /// is the legitimate way to express "fee disabled". `Some(bps)` must
    /// satisfy `1 <= bps <= MAX_PROTOCOL_FEE_BPS`.
    fn _validate_protocol_fee_bps(bps: Option<u32>) -> Result<(), ContractError> {
        if let Some(v) = bps {
            if v == 0 || v > MAX_PROTOCOL_FEE_BPS {
                return Err(ContractError::InvalidProtocolFeeBps);
            }
        }
        Ok(())
    }

    /// Reads the currently-configured protocol fee in bps (Issue #162).
    /// Bumps TTL only when the key is present (avoids extra storage writes
    /// on the hot "fee disabled" path through every competitive settlement).
    fn _read_protocol_fee_bps(env: &Env) -> Option<u32> {
        let key = DataKey::ProtocolFeeBps;
        let v: Option<u32> = env.storage().persistent().get(&key);
        if v.is_some() {
            Self::_extend_persistent_ttl(env, &key);
        }
        v
    }

    /// Credits `fee_amount` stroops to the protocol fee treasury and emits
    /// `("protocol", "fee_collected")` (Issue #162). TTL on the treasury
    /// key is extended on every write so the cumulative balance never
    /// falls into archival. Payload mirrors the active bps so indexers
    /// do not need an extra storage read.
    fn _collect_protocol_fee(
        env: &Env,
        round_id: u64,
        fee_amount: i128,
        bps_active: Option<u32>,
    ) -> Result<(), ContractError> {
        if fee_amount <= 0 {
            return Ok(());
        }
        let treasury_key = DataKey::ProtocolFeeTreasury;
        let current: i128 = env.storage().persistent().get(&treasury_key).unwrap_or(0);
        let new_treasury = current
            .checked_add(fee_amount)
            .ok_or(ContractError::Overflow)?;
        env.storage().persistent().set(&treasury_key, &new_treasury);
        Self::_extend_persistent_ttl(env, &treasury_key);

        let bps_value: u32 = bps_active.unwrap_or(0);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("protocol"), symbol_short!("collected")),
            (round_id, fee_amount, new_treasury, bps_value),
        );

        Ok(())
    }

    /// Splits a `(winning_pool, losing_pool)` pair into the post-fee pools
    /// and the treasury's cut, used by both UpDown settlement paths
    /// (Issue #162). Conservation invariant
    ///   dist_winning + dist_losing + fee == winning + losing
    /// holds ALWAYS, even in the pathological case `fee > losing_pool`
    /// (very thin losing-side liquidity near the bps cap): the spillover
    /// is then deducted from `winning_pool`, so winners lose a portion
    /// of their principal rather than the fee being silently dropped.
    /// Behaviour is documented in `docs/EVENT_SCHEMA.md` and exercised
    /// by `test_protocol_fee_thin_losing_pool`.
    fn _apply_protocol_fee_updown(
        env: &Env,
        round_id: u64,
        winning_pool: i128,
        losing_pool: i128,
    ) -> Result<(i128, i128, i128), ContractError> {
        let bps = Self::_read_protocol_fee_bps(env);
        if bps.is_none() {
            return Ok((winning_pool, losing_pool, 0));
        }
        let bps_value = bps.unwrap();
        let total_pot = Self::payout_add(winning_pool, losing_pool)?;
        let fee_amount = total_pot
            .checked_mul(bps_value as i128)
            .ok_or(ContractError::Overflow)?
            / BPS_DENOMINATOR;
        if fee_amount == 0 {
            return Ok((winning_pool, losing_pool, 0));
        }
        let fee_from_losing = fee_amount.min(losing_pool);
        let fee_from_winning = fee_amount
            .checked_sub(fee_from_losing)
            .ok_or(ContractError::Overflow)?;
        let dist_winning = winning_pool
            .checked_sub(fee_from_winning)
            .ok_or(ContractError::Overflow)?;
        let dist_losing = losing_pool
            .checked_sub(fee_from_losing)
            .ok_or(ContractError::Overflow)?;
        Self::_collect_protocol_fee(env, round_id, fee_amount, Some(bps_value))?;
        Ok((dist_winning, dist_losing, fee_amount))
    }

    /// Splits a precision-mode `total_pot` into the distributable amount
    /// (split among winners per the existing remainder policy) and the
    /// treasury's cut (Issue #162). Returns `(distributable, fee_amount)`.
    fn _apply_protocol_fee_precision(
        env: &Env,
        round_id: u64,
        total_pot: i128,
    ) -> Result<(i128, i128), ContractError> {
        let bps = Self::_read_protocol_fee_bps(env);
        if bps.is_none() || total_pot <= 0 {
            return Ok((total_pot, 0));
        }
        let bps_value = bps.unwrap();
        let fee_amount = total_pot
            .checked_mul(bps_value as i128)
            .ok_or(ContractError::Overflow)?
            / BPS_DENOMINATOR;
        let distributable = total_pot
            .checked_sub(fee_amount)
            .ok_or(ContractError::Overflow)?;
        if fee_amount > 0 {
            Self::_collect_protocol_fee(env, round_id, fee_amount, Some(bps_value))?;
        }
        Ok((distributable, fee_amount))
    }

    fn _emit_action_rejected(env: &Env, actor: &Address, action: Symbol, reason: ContractError) {
        // Privacy: event payload contains only the actor Address, an action
        // symbol, and a numeric reason code. No personally identifiable
        // information, financial amounts, or internal state is exposed.
        // Operators can match reason codes against ContractError variants.
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("action"), symbol_short!("rejct")),
            (actor.clone(), action, reason as u32),
        );
    }

    fn _emit_config_updated(
        env: &Env,
        kind: ConfigChangeKind,
        old_value: ConfigChangePayload,
        new_value: ConfigChangePayload,
    ) {
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("config"), symbol_short!("updated")),
            (kind, old_value, new_value),
        );
    }

    fn _current_config_payload(env: &Env, kind: &ConfigChangeKind) -> ConfigChangePayload {
        match kind {
            ConfigChangeKind::Windows => {
                let bet: u32 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::BetWindowLedgers)
                    .unwrap_or(DEFAULT_BET_WINDOW_LEDGERS);
                let run: u32 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::RunWindowLedgers)
                    .unwrap_or(DEFAULT_RUN_WINDOW_LEDGERS);
                ConfigChangePayload::Windows(bet, run)
            }
            ConfigChangeKind::MaxStake => {
                ConfigChangePayload::MaxStake(env.storage().persistent().get(&DataKey::MaxStake))
            }
            ConfigChangeKind::MaxUserRoundExposure => ConfigChangePayload::MaxUserRoundExposure(
                env.storage()
                    .persistent()
                    .get(&DataKey::MaxUserRoundExposure),
            ),
            ConfigChangeKind::MaxPendingWinnings => ConfigChangePayload::MaxPendingWinnings(
                env.storage().persistent().get(&DataKey::MaxPendingWinnings),
            ),
            ConfigChangeKind::OracleStaleThreshold => ConfigChangePayload::OracleStaleThreshold(
                env.storage()
                    .persistent()
                    .get(&DataKey::OracleStaleThreshold)
                    .unwrap_or(DEFAULT_ORACLE_STALE_THRESHOLD),
            ),
            ConfigChangeKind::OracleMaxDeviationBps => ConfigChangePayload::OracleMaxDeviationBps(
                env.storage()
                    .persistent()
                    .get(&DataKey::OracleMaxDeviationBps),
            ),
            ConfigChangeKind::ProtocolFeeBps => ConfigChangePayload::ProtocolFeeBps(
                env.storage().persistent().get(&DataKey::ProtocolFeeBps),
            ),
            ConfigChangeKind::MinParticipants => ConfigChangePayload::MinParticipants(
                env.storage().persistent().get(&DataKey::MinParticipants),
            ),
            ConfigChangeKind::MaxPrecisionParticipants => {
                ConfigChangePayload::MaxPrecisionParticipants(
                    env.storage()
                        .persistent()
                        .get(&DataKey::MaxPrecisionParticipants)
                        .unwrap_or(DEFAULT_MAX_PRECISION_PARTICIPANTS),
                )
            }
            ConfigChangeKind::MintLimit => ConfigChangePayload::MintLimit(
                env.storage()
                    .instance()
                    .get(&DataKey::MintLimitConfig)
                    .unwrap_or(0),
            ),
            ConfigChangeKind::ArchiveRetention => ConfigChangePayload::ArchiveRetention(
                env.storage()
                    .persistent()
                    .get(&DataKey::ArchiveRetention)
                    .unwrap_or(DEFAULT_ARCHIVE_RETENTION),
            ),
        }
    }

    fn _schedule_config_change(
        env: &Env,
        kind: ConfigChangeKind,
        payload: ConfigChangePayload,
    ) -> Result<(), ContractError> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ContractError::AdminNotSet)?;
        admin.require_auth();
        Self::_ensure_not_paused(env).map_err(|e| {
            Self::_emit_action_rejected(env, &admin, symbol_short!("sched"), e);
            e
        })?;

        let key = DataKey::PendingConfigChange(kind.clone());
        if env.storage().persistent().has(&key) {
            Self::_emit_action_rejected(
                env,
                &admin,
                symbol_short!("sched"),
                ContractError::RoundAlreadyActive,
            );
            return Err(ContractError::RoundAlreadyActive);
        }

        let scheduled_at_ledger = env.ledger().sequence();
        let activation_ledger = scheduled_at_ledger
            .checked_add(CONFIG_TIMELOCK_LEDGERS)
            .ok_or(ContractError::Overflow)?;

        let pending = PendingConfigChange {
            payload,
            activation_ledger,
            scheduled_at_ledger,
        };
        env.storage().persistent().set(&key, &pending);
        Self::_extend_persistent_ttl(env, &key);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("config"), symbol_short!("sched")),
            (kind, activation_ledger),
        );

        Ok(())
    }

    fn _apply_config_payload(
        env: &Env,
        kind: &ConfigChangeKind,
        payload: &ConfigChangePayload,
    ) -> Result<(), ContractError> {
        let old_value = Self::_current_config_payload(env, kind);
        match (kind, payload) {
            (ConfigChangeKind::Windows, ConfigChangePayload::Windows(bet, run)) => {
                Self::_validate_windows(*bet, *run)?;
                env.storage()
                    .persistent()
                    .set(&DataKey::BetWindowLedgers, bet);
                Self::_extend_persistent_ttl(env, &DataKey::BetWindowLedgers);
                env.storage()
                    .persistent()
                    .set(&DataKey::RunWindowLedgers, run);
                Self::_extend_persistent_ttl(env, &DataKey::RunWindowLedgers);
                #[allow(deprecated)]
                env.events().publish(
                    (symbol_short!("windows"), symbol_short!("updated")),
                    (*bet, *run),
                );
            }
            (ConfigChangeKind::MaxStake, ConfigChangePayload::MaxStake(max)) => {
                Self::_validate_max_stake(*max)?;
                let key = DataKey::MaxStake;
                if let Some(v) = max {
                    env.storage().persistent().set(&key, v);
                    Self::_extend_persistent_ttl(env, &key);
                } else {
                    env.storage().persistent().remove(&key);
                }
            }
            (
                ConfigChangeKind::MaxUserRoundExposure,
                ConfigChangePayload::MaxUserRoundExposure(max),
            ) => {
                Self::_validate_max_stake(*max)?;
                let key = DataKey::MaxUserRoundExposure;
                if let Some(v) = max {
                    env.storage().persistent().set(&key, v);
                    Self::_extend_persistent_ttl(env, &key);
                } else {
                    env.storage().persistent().remove(&key);
                }
            }
            (
                ConfigChangeKind::MaxPendingWinnings,
                ConfigChangePayload::MaxPendingWinnings(max),
            ) => {
                Self::_validate_max_stake(*max)?;
                let key = DataKey::MaxPendingWinnings;
                if let Some(v) = max {
                    env.storage().persistent().set(&key, v);
                    Self::_extend_persistent_ttl(env, &key);
                } else {
                    env.storage().persistent().remove(&key);
                }
            }
            (
                ConfigChangeKind::OracleStaleThreshold,
                ConfigChangePayload::OracleStaleThreshold(seconds),
            ) => {
                Self::_validate_oracle_stale_threshold(*seconds)?;
                let key = DataKey::OracleStaleThreshold;
                env.storage().persistent().set(&key, seconds);
                Self::_extend_persistent_ttl(env, &key);
            }
            (
                ConfigChangeKind::OracleMaxDeviationBps,
                ConfigChangePayload::OracleMaxDeviationBps(bps),
            ) => {
                Self::_validate_oracle_max_deviation_bps(*bps)?;
                let key = DataKey::OracleMaxDeviationBps;
                if let Some(v) = bps {
                    env.storage().persistent().set(&key, v);
                    Self::_extend_persistent_ttl(env, &key);
                } else {
                    env.storage().persistent().remove(&key);
                }
            }

            (ConfigChangeKind::ProtocolFeeBps, ConfigChangePayload::ProtocolFeeBps(bps)) => {
                Self::_validate_protocol_fee_bps(*bps)?;
                let key = DataKey::ProtocolFeeBps;
                if let Some(v) = bps {
                    env.storage().persistent().set(&key, v);
                    Self::_extend_persistent_ttl(env, &key);
                } else {
                    env.storage().persistent().remove(&key);
                }
                #[allow(deprecated)]
                env.events().publish(
                    (symbol_short!("protocol"), symbol_short!("bps_set")),
                    (bps.clone(),),
                );
            }
            _ => return Err(ContractError::InvalidMode),
        }
        Self::_emit_config_updated(env, kind.clone(), old_value, payload.clone());
        Ok(())
    }

    /// Bumps/extends the TTL of the given persistent storage key if its remaining TTL
    /// is less than the threshold. Enforces rent policy (Issue #142).
    fn _extend_persistent_ttl(env: &Env, key: &DataKey) {
        if env.storage().persistent().has(key) {
            env.storage()
                .persistent()
                .extend_ttl(key, TTL_BUMP_THRESHOLD, TTL_BUMP_AMOUNT);
        }
impl VirtualTokenContract {
    pub fn _update_stats_win(env: &Env, user: Address) -> Result<(), ContractError> {
        settlement::_update_stats_win(env, user)
    }

    pub fn _update_stats_loss(env: &Env, user: Address) -> Result<(), ContractError> {
        settlement::_update_stats_loss(env, user)
    }
}

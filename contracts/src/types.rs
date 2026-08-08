// SPDX-License-Identifier: MIT
//! Type definitions for the XLM Price Prediction Market.

use soroban_sdk::{contracttype, Address, BytesN, Vec};

/// Round mode for prediction type
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundMode {
    UpDown = 0,    // Simple up/down predictions
    Precision = 1, // Exact price predictions (Legends mode)
}

/// Runtime mode for the contract lifecycle
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum RuntimeMode {
    Normal = 0,
    ClaimsOnly = 1,
    FullyPaused = 2,
}

/// Lifecycle phase of an active round, derived from ledger windows.
///
/// Semantics (given `start_ledger`, `bet_end_ledger`, `end_ledger`):
/// - `Betting`: `ledger < bet_end_ledger` — bets and precision predictions accepted
/// - `Running`: `bet_end_ledger ≤ ledger < end_ledger` — reveal window (precision)
/// - `Resolvable`: `ledger ≥ end_ledger` — round may be settled via oracle payload
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundPhase {
    Betting = 1,
    Running = 2,
    Resolvable = 3,
}

/// Storage keys for contract data
///
/// ## Indexed position keys (variants 13–15)
///
/// `Position(round_id, address)` and `PrecisionPosition(round_id, address)` store
/// a single user's record under a composite key, enabling O(1) read/write per user
/// instead of deserializing the full participant map on every bet.
///
/// `RoundParticipants(round_id)` holds the ordered `Vec<Address>` used for
/// iteration at resolution time. Appending one address is cheaper than
/// re-serialising an N-entry `Map<Address, T>` for every bet placed.
///
/// Legacy single-key maps (`UpDownPositions`, `PrecisionPositions`) are kept for
/// backward-compatible reads during a migration window; they are no longer written.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Balance(Address),
    Admin,
    Oracle,
    /// On-chain storage schema version for migration safety.
    /// If missing, the contract treats it as legacy schema version 1.
    SchemaVersion,
    ActiveRound,
    Positions,          // Legacy key — read-only migration compat
    UpDownPositions,    // Legacy key — read-only migration compat
    PrecisionPositions, // Legacy key — read-only migration compat
    PendingWinnings(Address),
    UserStats(Address),
    Paused,
    BetWindowLedgers,
    RunWindowLedgers,
    CloseBufferLedgers,
    LastRoundId,
    /// Per-user UpDown position: (round_id, address) → UserPosition
    Position(u64, Address),
    /// Per-user Precision prediction: (round_id, address) → PrecisionPrediction
    PrecisionPosition(u64, Address),
    /// Per-user Precision commitment: (round_id, address) → PrecisionCommitment
    PrecisionCommitment(u64, Address),
    /// Ordered participant list for a round: round_id → Vec<Address>
    RoundParticipants(u64),
    /// Maximum stake allowed per individual bet (None = unlimited)
    MaxStake,
    /// Maximum cumulative exposure per user per round (None = unlimited)
    MaxUserRoundExposure,
    /// Maximum pending winnings allowed per account (None = unlimited)
    MaxPendingWinnings,
    /// Marker for a cancelled round: round_id → true
    CancelledRound(u64),
    /// Per-round consumed oracle nonce: (round_id, nonce) → true.
    /// Used to reject duplicate oracle payload submissions for the same round.
    ConsumedOracleNonce(u64, u64),
    /// Minimum participant count for competitive settlement; unset = no minimum enforced
    MinParticipants,
    /// Oracle heartbeat: last recorded timestamp and status
    OracleHeartbeat,
    /// Stale-heartbeat threshold in seconds (admin-configurable); unset = 3600 s default
    OracleStaleThreshold,
    /// Maximum participants accepted in a Precision round; unset = protocol default
    MaxPrecisionParticipants,
    /// Oracle max deviation threshold in basis points (1 bp = 0.01%).
    /// If unset, deviation guardrails are disabled.
    OracleMaxDeviationBps,
    /// One-shot admin override allowing the next settlement to bypass deviation checks.
    /// Automatically cleared after use.
    OracleDeviationOverrideArmed,
    /// Minimum oracle confidence threshold in basis points (0–10000).
    /// If unset, confidence guardrails are disabled.
    OracleMinConfidenceBps,
    /// When true, payloads with missing confidence are rejected in strict mode.
    OracleStrictMode,
    /// Compact post-settlement summary keyed by round id for historical queries.
    ArchivedRound(u64),
    /// Ordered round ids for archive retention (oldest at index 0).
    RecentArchivedRoundIds,
    /// Per-user outcome record for a specific archived round (round_id, user).
    /// Persisted at settlement for user history queries without event replay.
    UserRoundOutcome(u64, Address),
    /// Per-user index of archived round IDs the user participated in.
    /// Written during archiving; read for paginated history queries.
    /// Not pruned when archived rounds are evicted — stale entries are
    /// filtered at query time by checking ArchivedRound existence.
    UserArchivedRoundIds(Address),
    /// Marker written by migrate_schema_v2_to_v3 to prove the migration ran.
    MigratedToV3,
    /// Timelocked pending critical config change keyed by change kind.
    PendingConfigChange(ConfigChangeKind),
    /// Optional protocol settlement fee in basis points (1 bp = 0.01%).
    /// `None` (key absent) means fee disabled — no behaviour change.
    /// Hard cap on fee is enforced at the contract layer, not by storage shape.
    ProtocolFeeBps,
    /// On-chain accumulated protocol fee balance in stroops (i128).
    /// Admin withdraws via the dedicated withdrawal method; does NOT mix
    /// into the per-user balance ledger.
    ProtocolFeeTreasury,
    /// Per-ledger mint counter: wraps the explicit ledger sequence number.
    LedgerMintCounter(u32),
    /// Mint limit configuration: maximum number of mints allowed per ledger.
    MintLimitConfig,
    /// Pending two-step oracle rotation proposal with expiry.
    OracleRotationProposal,
    /// Configurable archive retention limit: maximum number of ArchivedRound entries
    /// retained on-chain before the oldest are pruned (FIFO). If unset, the protocol
    /// default is used.
    ArchiveRetention,
    /// Admin-configured blueprint used by `create_next_from_template` to spin
    /// up the next round without re-specifying `start_price` / `mode` each
    /// time. Absent means no template is configured.
    RoundTemplate,
    /// Bounded index of user addresses sorted by lifetime total wins
    /// descending (all-time leaderboard, independent of seasons).
    LeaderboardWins,
    /// Bounded index of user addresses sorted by lifetime best streak
    /// descending (all-time leaderboard, independent of seasons).
    LeaderboardStreak,
    /// Monotonically increasing id of the currently-active leaderboard
    /// season. Absent is treated as season 1.
    SeasonId,
    /// Per-season, per-user win/loss/streak stats: (season_id, address) →
    /// UserStats, scoped independently of the lifetime `UserStats` totals so
    /// a season reset never touches lifetime history.
    SeasonUserStats(u32, Address),
    /// Bounded index of user addresses in the *active* season sorted by
    /// season-scoped total wins descending.
    SeasonLeaderboardWins,
    /// Bounded index of user addresses in the *active* season sorted by
    /// season-scoped best streak descending.
    SeasonLeaderboardStreak,
    /// Frozen snapshot of a season's final rankings, written when the season
    /// is reset. Seasons are never deleted — this is a permanent archive.
    SeasonArchive(u32),
}

/// Identifies which critical risk setting is pending timelocked activation.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ConfigChangeKind {
    Windows = 0,
    MaxStake = 1,
    MaxUserRoundExposure = 2,
    MaxPendingWinnings = 3,
    OracleStaleThreshold = 4,
    OracleMaxDeviationBps = 5,
    /// Optional protocol settlement fee in bps (Issue #162).
    /// `None` disables the fee entirely, restoring pre-fee behaviour.
    ProtocolFeeBps = 6,
    MinParticipants = 7,
    MaxPrecisionParticipants = 8,
    MintLimit = 9,
    ArchiveRetention = 10,
    CloseBufferLedgers = 11,
}

/// Payload for a scheduled critical config change.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigChangePayload {
    Windows(u32, u32),
    MaxStake(Option<i128>),
    MaxUserRoundExposure(Option<i128>),
    MaxPendingWinnings(Option<i128>),
    OracleStaleThreshold(u64),
    OracleMaxDeviationBps(Option<u32>),
    ProtocolFeeBps(Option<u32>),
    MinParticipants(Option<u32>),
    MaxPrecisionParticipants(u32),
    MintLimit(u32),
    ArchiveRetention(u32),
    CloseBufferLedgers(u32),
}

/// Pending timelocked config change with activation ledger for on-chain observability.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingConfigChange {
    pub payload: ConfigChangePayload,
    pub activation_ledger: u32,
    pub scheduled_at_ledger: u32,
}

/// Represents which side a user bet on
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum BetSide {
    Up,
    Down,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserPosition {
    pub amount: i128,
    pub side: BetSide,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserStats {
    pub total_wins: u32,
    pub total_losses: u32,
    pub current_streak: u32,
    pub best_streak: u32,
}

/// Precision prediction entry (user address + predicted price)
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PrecisionPrediction {
    pub user: Address,
    pub predicted_price: u128, // Price scaled to 4 decimals (e.g., 0.2297 → 2297)
    pub amount: i128,          // Bet amount
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PrecisionCommitment {
    pub hash: BytesN<32>,
    pub amount: i128,
    pub revealed: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OraclePayload {
    pub price: u128,
    pub timestamp: u64,
    /// Round identifier that should match `Round.start_ledger`
    pub round_id: u32,
    /// Per-round replay-protection nonce.
    ///
    /// The oracle service must generate a unique value per submission for a
    /// given round (e.g. a monotonic counter or random 64-bit value). The
    /// contract records each consumed nonce under
    /// `DataKey::ConsumedOracleNonce(round_id, nonce)` and rejects any reuse,
    /// making resolution idempotent against accidental duplicate submissions.
    pub nonce: u64,
    /// SHA-256 hash of the network passphrase this payload targets.
    /// Validated against `env.ledger().network_id()` to prevent cross-network replay.
    pub network_id: BytesN<32>,
    /// Contract address this payload is intended for.
    /// Validated against `env.current_contract_address()` to prevent cross-contract replay.
    pub contract_addr: Address,
    /// Optional confidence score from the price feed (0–10000 bps, where 10000 = 100%).
    /// When `None`, the payload is treated as a legacy submission.
    /// When strict mode is enabled, `None` is rejected.
    pub confidence: Option<u32>,
}

/// Oracle liveness record, updated by the oracle service on each heartbeat call.
/// `status`: 0 = active, 1 = degraded, 2 = offline.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OracleHeartbeatRecord {
    pub timestamp: u64,
    pub status: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    pub round_id: u64,       // Unique monotonically increasing round identifier
    pub price_start: u128,   // Starting XLM price in stroops
    pub start_ledger: u32,   // Ledger when round was created
    pub bet_end_ledger: u32, // Ledger when betting closes
    pub end_ledger: u32,     // Ledger when round ends (~5s per ledger)
    pub pool_up: i128,       // Total vXLM bet on UP
    pub pool_down: i128,     // Total vXLM bet on DOWN
    pub mode: RoundMode,     // Round mode: UpDown (0) or Precision (1)
}

/// Aggregated active-round pool composition for frontend transparency.
///
/// Up/Down rounds populate the up/down pools, counts, and stake ratios.
/// Precision rounds populate the precision totals and participant counters while
/// leaving side-specific Up/Down fields at zero. Ratios are basis points of
/// the mode's total visible stake (10_000 = 100%).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RoundPoolStats {
    pub round_id: u64,
    pub mode: RoundMode,
    pub total_up_stake: i128,
    pub total_down_stake: i128,
    pub up_participant_count: u32,
    pub down_participant_count: u32,
    pub up_stake_ratio_bps: u32,
    pub down_stake_ratio_bps: u32,
    pub precision_total_stake: i128,
    pub precision_participant_count: u32,
    pub precision_prediction_count: u32,
    pub precision_commitment_count: u32,
    pub precision_revealed_count: u32,
}

/// Terminal outcome recorded when a round leaves the active state.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundArchiveStatus {
    /// Oracle settlement completed (normal resolution path).
    Resolved = 0,
    /// Admin cancelled the round and refunded participants.
    Cancelled = 1,
    /// Settlement aborted due to insufficient participants; stakes refunded.
    FallbackRefund = 2,
}

/// Composite protocol health status returned by `get_protocol_health`.
///
/// Designed for operators to poll a single endpoint instead of stitching
/// together multiple read-only calls.
///
/// ## Status code → alert severity mapping
///
/// | code | label           | severity | meaning                                   |
/// |------|-----------------|----------|-------------------------------------------|
/// | 0    | HEALTHY         | none     | All subsystems nominal                    |
/// | 1    | PAUSED          | critical | Contract is emergency-paused               |
/// | 2    | ORACLE_STALE    | warning  | Oracle heartbeat is stale or offline      |
/// | 3    | ROUND_STALE     | warning  | Round is past its end ledger but unresolved|
/// | 4    | NO_ACTIVE_ROUND | info     | No round currently active (idle protocol) |
/// | 5    | MULTIPLE_ISSUES | critical | Two or more issues detected simultaneously|
///
/// ## Phase codes (`active_round_phase`)
///
/// | phase | meaning                                           |
/// |-------|---------------------------------------------------|
/// | 0     | No active round                                   |
/// | 1     | Betting open (`ledger < bet_end_ledger`)           |
/// | 2     | Running / reveal window (`bet_end_ledger ≤ ledger < end_ledger`) |
/// | 3     | Resolvable (`ledger ≥ end_ledger`)                |
///
/// ## Oracle status codes (`oracle_status`)
///
/// | code | meaning                                |
/// |------|----------------------------------------|
/// | 0    | Active (healthy heartbeat)             |
/// | 1    | Degraded (heartbeat marked degraded)   |
/// | 2    | Offline (heartbeat marked offline)     |
/// | 3    | Unknown (no heartbeat record stored)   |
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolHealthStatus {
    /// Whether the contract is emergency-paused (`Paused == true`)
    pub paused: bool,
    /// Whether the oracle heartbeat is non-stale and not offline
    pub oracle_live: bool,
    /// Raw oracle heartbeat status (0=active, 1=degraded, 2=offline, 3=unknown)
    pub oracle_status: u32,
    /// Whether a round is currently active
    pub has_active_round: bool,
    /// Current round phase (0=no_round, 1=betting, 2=running, 3=resolvable)
    pub active_round_phase: u32,
    /// On-chain storage schema version
    pub schema_version: u32,
    /// Ledger sequence at which this health snapshot was taken
    pub ledger_sequence: u32,
    /// Ledger timestamp at which this health snapshot was taken
    pub ledger_timestamp: u64,
    /// Composite status code (see mapping table above)
    pub status_code: u32,
}

/// Compact historical round summary persisted after resolve or cancel.
///
/// Designed for explorer/analytics queries without replaying events.
/// `price_final` is `0` for admin cancellations (no oracle settlement price).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ArchivedRoundSummary {
    pub round_id: u64,
    pub price_start: u128,
    pub price_final: u128,
    pub mode: RoundMode,
    pub status: RoundArchiveStatus,
    pub pool_up: i128,
    pub pool_down: i128,
    pub participant_count: u32,
    pub settled_at_ledger: u32,
}

/// Pending two-step oracle rotation proposal.
///
/// The admin proposes a new oracle address with a timestamp-based expiry window.
/// After `expires_at` (ledger timestamp) the proposal is stale and acceptance
/// is rejected until the admin submits a fresh proposal.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OracleRotationProposal {
    pub new_oracle: Address,
    pub proposed_at: u64,
    pub expires_at: u64,
}

/// Global status of the protocol, returned by `get_protocol_status`.
///
/// Designed for frontend state machines that need a single, stable code
/// instead of combining multiple boolean flags.
///
/// ## Status codes
///
/// | value | variant      | description                                                             |
/// |-------|--------------|-------------------------------------------------------------------------|
/// | 0     | `Active`     | Not paused; a round is currently active (bets open or running).          |
/// | 1     | `Paused`     | Emergency-paused by the admin; no mutations accepted except unpause.     |
/// | 2     | `ClaimsOnly` | Not paused; no active round. Only `claim_winnings` is meaningful.        |
///
/// ## Transition rules
///
/// - `ClaimsOnly` → `Active` when `create_round()` succeeds.
/// - `Active` → `ClaimsOnly` when `resolve_round()` or `cancel_round()` completes.
/// - Any state → `Paused` when `pause_contract()` is called.
/// - `Paused` → `Active` when `unpause_contract()` is called *and* an active round still exists.
/// - `Paused` → `ClaimsOnly` when `unpause_contract()` is called *and* no active round exists.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ProtocolStatus {
    /// The contract is not paused and has a currently active round.
    Active = 0,
    /// The contract is emergency-paused by the admin.
    Paused = 1,
    /// The contract is not paused, but no round is active.
    /// Mutating actions are limited to claiming pending winnings.
    ClaimsOnly = 2,
}

/// Status of a specific round, returned by `get_round_status(round_id)`.
///
/// Queries a round by its monotonic `round_id`. Covers all lifecycle
/// stages from creation through terminal settlement.
///
/// ## Status codes
///
/// | value | variant          | description                                                                      |
/// |-------|------------------|-----------------------------------------------------------------------------------|
/// | 0     | `Unknown`        | Round does not exist or has been pruned from the on-chain archive.               |
/// | 1     | `Betting`        | Round is active; bets and predictions accepted (`ledger < bet_end_ledger`).      |
/// | 2     | `Running`        | Betting closed; reveal window open (`bet_end_ledger ≤ ledger < end_ledger`).    |
/// | 3     | `AwaitingResolve`| Round ended; awaiting oracle settlement (`ledger ≥ end_ledger`).                |
/// | 4     | `Resolved`       | Oracle settled the round; pot distributed to winners.                            |
/// | 5     | `Cancelled`      | Admin cancelled the round; all stakes refunded.                                  |
/// | 6     | `FallbackRefund` | Insufficient participants at settlement; all stakes refunded.                    |
///
/// ## Transition rules
///
/// - `Unknown` → `Betting` when `create_round()` succeeds.
/// - `Betting` → `Running` when `ledger ≥ bet_end_ledger` (derived; no on-chain write).
/// - `Running` → `AwaitingResolve` when `ledger ≥ end_ledger` (derived; no on-chain write).
/// - `{Betting | Running | AwaitingResolve}` → `Cancelled` when `cancel_round()` is called.
/// - `AwaitingResolve` → `Resolved` when `resolve_round()` settles with enough participants.
/// - `AwaitingResolve` → `FallbackRefund` when `resolve_round()` finds fewer than `min_participants`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundStatus {
    /// Round does not exist or has been pruned from the on-chain archive.
    Unknown = 0,
    /// Round is active; bets and predictions accepted (`ledger < bet_end_ledger`).
    Betting = 1,
    /// Betting is closed; reveal window is open (`bet_end_ledger ≤ ledger < end_ledger`).
    Running = 2,
    /// Round has ended and is waiting for oracle settlement (`ledger ≥ end_ledger`).
    AwaitingResolve = 3,
    /// Oracle settled the round normally; pot distributed to winners.
    Resolved = 4,
    /// Admin cancelled the round; all stakes refunded.
    Cancelled = 5,
    /// Settlement triggered but insufficient participants; all stakes refunded.
    FallbackRefund = 6,
}

/// Terminal outcome persisted per user per archived round.
///
/// Allows `get_user_archived_participation` to answer profile/history
/// queries without replaying the full event stream.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum UserOutcomeType {
    Win = 0,
    Loss = 1,
    Refund = 2,
    Cancel = 3,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserRoundOutcome {
    pub user: Address,
    pub round_mode: u32,
    pub prediction_side: u32,
    pub predicted_price: u128,
    pub stake: i128,
    pub payout: i128,
    pub outcome: UserOutcomeType,
}

/// Simulated payout result for a specific hypothetical final price.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SimulationResult {
    pub mode: RoundMode,
    pub pool_up: i128,
    pub pool_down: i128,
    pub precision_total_stake: i128,
    pub fee_amount: i128,
    pub outcomes: Vec<UserRoundOutcome>,
}

/// Admin-configured blueprint for `create_next_from_template`.
///
/// Mirrors the arguments accepted by `create_round` (`start_price`, `mode`)
/// so a keeper can spin up the next round after a settle/cancel without an
/// operator re-specifying parameters each time. Validated with the exact
/// same rules `create_round` applies at creation time.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RoundTemplate {
    pub start_price: u128,
    pub mode: Option<u32>,
}

/// A single entry in the lifetime (all-time) leaderboard.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LeaderboardEntry {
    pub user: Address,
    pub stats: UserStats,
}

/// A single entry in a season-scoped leaderboard, live or archived.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonLeaderboardEntry {
    pub user: Address,
    pub wins: u32,
    pub best_streak: u32,
}

/// Frozen snapshot of a season's final bounded rankings, written by
/// `reset_leaderboard_season`. `participant_count` is the number of distinct
/// addresses that appeared in either bounded index at reset time (a lower
/// bound on total season participants beyond the tracked top
/// `LEADERBOARD_LIMIT`, mirroring the same bound the live indexes enforce).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonArchive {
    pub season_id: u32,
    pub ended_at_ledger: u32,
    pub wins: Vec<SeasonLeaderboardEntry>,
    pub streak: Vec<SeasonLeaderboardEntry>,
    pub participant_count: u32,
}
a
// SPDX-License-Identifier: MIT
//! Type definitions for the XLM Price Prediction Market.

use soroban_sdk::{contracttype, Address, BytesN, Vec};

/// Round mode for prediction type
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundMode {
    UpDown = 0,    // Simple up/down predictions
    Precision = 1, // Exact price predictions (Legends mode)
}

/// Runtime mode for the contract lifecycle
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum RuntimeMode {
    Normal = 0,
    ClaimsOnly = 1,
    FullyPaused = 2,
}

/// Lifecycle phase of an active round, derived from ledger windows.
///
/// Semantics (given `start_ledger`, `bet_end_ledger`, `end_ledger`):
/// - `Betting`: `ledger < bet_end_ledger` — bets and precision predictions accepted
/// - `Running`: `bet_end_ledger ≤ ledger < end_ledger` — reveal window (precision)
/// - `Resolvable`: `ledger ≥ end_ledger` — round may be settled via oracle payload
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundPhase {
    Betting = 1,
    Running = 2,
    Resolvable = 3,
}

/// Storage keys for contract data
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Balance(Address),
    Admin,
    Oracle,
    SchemaVersion,
    ActiveRound,
    Positions,
    UpDownPositions,
    PrecisionPositions,
    PendingWinnings(Address),
    UserStats(Address),
    Paused,
    BetWindowLedgers,
    RunWindowLedgers,
    CloseBufferLedgers,
    LastRoundId,
    Position(u64, Address),
    PrecisionPosition(u64, Address),
    PrecisionCommitment(u64, Address),
    RoundParticipants(u64),
    MaxStake,
    MaxUserRoundExposure,
    MaxPendingWinnings,
    CancelledRound(u64),
    ConsumedOracleNonce(u64, u64),
    MinParticipants,
    OracleHeartbeat,
    OracleStaleThreshold,
    MaxPrecisionParticipants,
    OracleMaxDeviationBps,
    OracleDeviationOverrideArmed,
    OracleMinConfidenceBps,
    OracleStrictMode,
    ArchivedRound(u64),
    RecentArchivedRoundIds,
    UserRoundOutcome(u64, Address),
    MigratedToV3,
    PendingConfigChange(ConfigChangeKind),
    ProtocolFeeBps,
    ProtocolFeeTreasury,
    LedgerMintCounter(u32),
    MintLimitConfig,
    OracleRotationProposal,
    ArchiveRetention,
    RoundTemplate,
    Ext(DataKeyExt),
}

#[contracttype]
#[derive(Clone)]
pub enum DataKeyExt {
    LeaderboardWins,
    LeaderboardStreak,
    SeasonId,
    SeasonUserStats(u32, Address),
    SeasonLeaderboardWins,
    SeasonLeaderboardStreak,
    SeasonArchive(u32),
}

/// Identifies which critical risk setting is pending timelocked activation.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ConfigChangeKind {
    Windows = 0,
    MaxStake = 1,
    MaxUserRoundExposure = 2,
    MaxPendingWinnings = 3,
    OracleStaleThreshold = 4,
    OracleMaxDeviationBps = 5,
    /// Optional protocol settlement fee in bps (Issue #162).
    /// `None` disables the fee entirely, restoring pre-fee behaviour.
    ProtocolFeeBps = 6,
    MinParticipants = 7,
    MaxPrecisionParticipants = 8,
    MintLimit = 9,
    ArchiveRetention = 10,
    CloseBufferLedgers = 11,
}

/// Payload for a scheduled critical config change.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigChangePayload {
    Windows(u32, u32),
    MaxStake(Option<i128>),
    MaxUserRoundExposure(Option<i128>),
    MaxPendingWinnings(Option<i128>),
    OracleStaleThreshold(u64),
    OracleMaxDeviationBps(Option<u32>),
    ProtocolFeeBps(Option<u32>),
    MinParticipants(Option<u32>),
    MaxPrecisionParticipants(u32),
    MintLimit(u32),
    ArchiveRetention(u32),
    CloseBufferLedgers(u32),
}

/// Pending timelocked config change with activation ledger for on-chain observability.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingConfigChange {
    pub payload: ConfigChangePayload,
    pub activation_ledger: u32,
    pub scheduled_at_ledger: u32,
}

/// Represents which side a user bet on
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum BetSide {
    Up,
    Down,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserPosition {
    pub amount: i128,
    pub side: BetSide,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserStats {
    pub total_wins: u32,
    pub total_losses: u32,
    pub current_streak: u32,
    pub best_streak: u32,
}

/// Precision prediction entry (user address + predicted price)
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PrecisionPrediction {
    pub user: Address,
    pub predicted_price: u128, // Price scaled to 4 decimals (e.g., 0.2297 → 2297)
    pub amount: i128,          // Bet amount
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PrecisionCommitment {
    pub hash: BytesN<32>,
    pub amount: i128,
    pub revealed: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OraclePayload {
    pub price: u128,
    pub timestamp: u64,
    /// Round identifier that should match `Round.start_ledger`
    pub round_id: u32,
    /// Per-round replay-protection nonce.
    ///
    /// The oracle service must generate a unique value per submission for a
    /// given round (e.g. a monotonic counter or random 64-bit value). The
    /// contract records each consumed nonce under
    /// `DataKey::ConsumedOracleNonce(round_id, nonce)` and rejects any reuse,
    /// making resolution idempotent against accidental duplicate submissions.
    pub nonce: u64,
    /// SHA-256 hash of the network passphrase this payload targets.
    /// Validated against `env.ledger().network_id()` to prevent cross-network replay.
    pub network_id: BytesN<32>,
    /// Contract address this payload is intended for.
    /// Validated against `env.current_contract_address()` to prevent cross-contract replay.
    pub contract_addr: Address,
    /// Optional confidence score from the price feed (0–10000 bps, where 10000 = 100%).
    /// When `None`, the payload is treated as a legacy submission.
    /// When strict mode is enabled, `None` is rejected.
    pub confidence: Option<u32>,
}

/// Oracle liveness record, updated by the oracle service on each heartbeat call.
/// `status`: 0 = active, 1 = degraded, 2 = offline.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OracleHeartbeatRecord {
    pub timestamp: u64,
    pub status: u32,
}

/// Heartbeat health gate configuration (Issue #264).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HbGateConfig {
    pub strict_mode: bool,
    pub override_armed: bool,
    pub grace_seconds: u64,
}

/// Storage key for heartbeat gate config (separate from DataKey to stay within variant limits, Issue #264).
#[contracttype]
#[derive(Clone)]
pub enum HbGateKey {
    Config,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    pub round_id: u64,       // Unique monotonically increasing round identifier
    pub price_start: u128,   // Starting XLM price in stroops
    pub start_ledger: u32,   // Ledger when round was created
    pub bet_end_ledger: u32, // Ledger when betting closes
    pub end_ledger: u32,     // Ledger when round ends (~5s per ledger)
    pub pool_up: i128,       // Total vXLM bet on UP
    pub pool_down: i128,     // Total vXLM bet on DOWN
    pub mode: RoundMode,     // Round mode: UpDown (0) or Precision (1)
}

/// Aggregated active-round pool composition for frontend transparency.
///
/// Up/Down rounds populate the up/down pools, counts, and stake ratios.
/// Precision rounds populate the precision totals and participant counters while
/// leaving side-specific Up/Down fields at zero. Ratios are basis points of
/// the mode's total visible stake (10_000 = 100%).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RoundPoolStats {
    pub round_id: u64,
    pub mode: RoundMode,
    pub total_up_stake: i128,
    pub total_down_stake: i128,
    pub up_participant_count: u32,
    pub down_participant_count: u32,
    pub up_stake_ratio_bps: u32,
    pub down_stake_ratio_bps: u32,
    pub precision_total_stake: i128,
    pub precision_participant_count: u32,
    pub precision_prediction_count: u32,
    pub precision_commitment_count: u32,
    pub precision_revealed_count: u32,
}

/// Terminal outcome recorded when a round leaves the active state.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundArchiveStatus {
    /// Oracle settlement completed (normal resolution path).
    Resolved = 0,
    /// Admin cancelled the round and refunded participants.
    Cancelled = 1,
    /// Settlement aborted due to insufficient participants; stakes refunded.
    FallbackRefund = 2,
}

/// Composite protocol health status returned by `get_protocol_health`.
///
/// Designed for operators to poll a single endpoint instead of stitching
/// together multiple read-only calls.
///
/// ## Status code → alert severity mapping
///
/// | code | label           | severity | meaning                                   |
/// |------|-----------------|----------|-------------------------------------------|
/// | 0    | HEALTHY         | none     | All subsystems nominal                    |
/// | 1    | PAUSED          | critical | Contract is emergency-paused               |
/// | 2    | ORACLE_STALE    | warning  | Oracle heartbeat is stale or offline      |
/// | 3    | ROUND_STALE     | warning  | Round is past its end ledger but unresolved|
/// | 4    | NO_ACTIVE_ROUND | info     | No round currently active (idle protocol) |
/// | 5    | MULTIPLE_ISSUES | critical | Two or more issues detected simultaneously|
///
/// ## Phase codes (`active_round_phase`)
///
/// | phase | meaning                                           |
/// |-------|---------------------------------------------------|
/// | 0     | No active round                                   |
/// | 1     | Betting open (`ledger < bet_end_ledger`)           |
/// | 2     | Running / reveal window (`bet_end_ledger ≤ ledger < end_ledger`) |
/// | 3     | Resolvable (`ledger ≥ end_ledger`)                |
///
/// ## Oracle status codes (`oracle_status`)
///
/// | code | meaning                                |
/// |------|----------------------------------------|
/// | 0    | Active (healthy heartbeat)             |
/// | 1    | Degraded (heartbeat marked degraded)   |
/// | 2    | Offline (heartbeat marked offline)     |
/// | 3    | Unknown (no heartbeat record stored)   |
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolHealthStatus {
    /// Whether the contract is emergency-paused (`Paused == true`)
    pub paused: bool,
    /// Whether the oracle heartbeat is non-stale and not offline
    pub oracle_live: bool,
    /// Raw oracle heartbeat status (0=active, 1=degraded, 2=offline, 3=unknown)
    pub oracle_status: u32,
    /// Whether a round is currently active
    pub has_active_round: bool,
    /// Current round phase (0=no_round, 1=betting, 2=running, 3=resolvable)
    pub active_round_phase: u32,
    /// On-chain storage schema version
    pub schema_version: u32,
    /// Ledger sequence at which this health snapshot was taken
    pub ledger_sequence: u32,
    /// Ledger timestamp at which this health snapshot was taken
    pub ledger_timestamp: u64,
    /// Composite status code (see mapping table above)
    pub status_code: u32,
}

/// Compact historical round summary persisted after resolve or cancel.
///
/// Designed for explorer/analytics queries without replaying events.
/// `price_final` is `0` for admin cancellations (no oracle settlement price).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ArchivedRoundSummary {
    pub round_id: u64,
    pub price_start: u128,
    pub price_final: u128,
    pub mode: RoundMode,
    pub status: RoundArchiveStatus,
    pub pool_up: i128,
    pub pool_down: i128,
    pub participant_count: u32,
    pub settled_at_ledger: u32,
}

/// Pending two-step oracle rotation proposal.
///
/// The admin proposes a new oracle address with a timestamp-based expiry window.
/// After `expires_at` (ledger timestamp) the proposal is stale and acceptance
/// is rejected until the admin submits a fresh proposal.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OracleRotationProposal {
    pub new_oracle: Address,
    pub proposed_at: u64,
    pub expires_at: u64,
}

/// Global status of the protocol, returned by `get_protocol_status`.
///
/// Designed for frontend state machines that need a single, stable code
/// instead of combining multiple boolean flags.
///
/// ## Status codes
///
/// | value | variant      | description                                                             |
/// |-------|--------------|-------------------------------------------------------------------------|
/// | 0     | `Active`     | Not paused; a round is currently active (bets open or running).          |
/// | 1     | `Paused`     | Emergency-paused by the admin; no mutations accepted except unpause.     |
/// | 2     | `ClaimsOnly` | Not paused; no active round. Only `claim_winnings` is meaningful.        |
///
/// ## Transition rules
///
/// - `ClaimsOnly` → `Active` when `create_round()` succeeds.
/// - `Active` → `ClaimsOnly` when `resolve_round()` or `cancel_round()` completes.
/// - Any state → `Paused` when `pause_contract()` is called.
/// - `Paused` → `Active` when `unpause_contract()` is called *and* an active round still exists.
/// - `Paused` → `ClaimsOnly` when `unpause_contract()` is called *and* no active round exists.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ProtocolStatus {
    /// The contract is not paused and has a currently active round.
    Active = 0,
    /// The contract is emergency-paused by the admin.
    Paused = 1,
    /// The contract is not paused, but no round is active.
    /// Mutating actions are limited to claiming pending winnings.
    ClaimsOnly = 2,
}

/// Status of a specific round, returned by `get_round_status(round_id)`.
///
/// Queries a round by its monotonic `round_id`. Covers all lifecycle
/// stages from creation through terminal settlement.
///
/// ## Status codes
///
/// | value | variant          | description                                                                      |
/// |-------|------------------|-----------------------------------------------------------------------------------|
/// | 0     | `Unknown`        | Round does not exist or has been pruned from the on-chain archive.               |
/// | 1     | `Betting`        | Round is active; bets and predictions accepted (`ledger < bet_end_ledger`).      |
/// | 2     | `Running`        | Betting closed; reveal window open (`bet_end_ledger ≤ ledger < end_ledger`).    |
/// | 3     | `AwaitingResolve`| Round ended; awaiting oracle settlement (`ledger ≥ end_ledger`).                |
/// | 4     | `Resolved`       | Oracle settled the round; pot distributed to winners.                            |
/// | 5     | `Cancelled`      | Admin cancelled the round; all stakes refunded.                                  |
/// | 6     | `FallbackRefund` | Insufficient participants at settlement; all stakes refunded.                    |
///
/// ## Transition rules
///
/// - `Unknown` → `Betting` when `create_round()` succeeds.
/// - `Betting` → `Running` when `ledger ≥ bet_end_ledger` (derived; no on-chain write).
/// - `Running` → `AwaitingResolve` when `ledger ≥ end_ledger` (derived; no on-chain write).
/// - `{Betting | Running | AwaitingResolve}` → `Cancelled` when `cancel_round()` is called.
/// - `AwaitingResolve` → `Resolved` when `resolve_round()` settles with enough participants.
/// - `AwaitingResolve` → `FallbackRefund` when `resolve_round()` finds fewer than `min_participants`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundStatus {
    /// Round does not exist or has been pruned from the on-chain archive.
    Unknown = 0,
    /// Round is active; bets and predictions accepted (`ledger < bet_end_ledger`).
    Betting = 1,
    /// Betting is closed; reveal window is open (`bet_end_ledger ≤ ledger < end_ledger`).
    Running = 2,
    /// Round has ended and is waiting for oracle settlement (`ledger ≥ end_ledger`).
    AwaitingResolve = 3,
    /// Oracle settled the round normally; pot distributed to winners.
    Resolved = 4,
    /// Admin cancelled the round; all stakes refunded.
    Cancelled = 5,
    /// Settlement triggered but insufficient participants; all stakes refunded.
    FallbackRefund = 6,
}

/// Terminal outcome persisted per user per archived round.
///
/// Allows `get_user_archived_participation` to answer profile/history
/// queries without replaying the full event stream.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum UserOutcomeType {
    Win = 0,
    Loss = 1,
    Refund = 2,
    Cancel = 3,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserRoundOutcome {
    pub user: Address,
    pub round_mode: u32,
    pub prediction_side: u32,
    pub predicted_price: u128,
    pub stake: i128,
    pub payout: i128,
    pub outcome: UserOutcomeType,
}

/// Simulated payout result for a specific hypothetical final price.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SimulationResult {
    pub mode: RoundMode,
    pub pool_up: i128,
    pub pool_down: i128,
    pub precision_total_stake: i128,
    pub fee_amount: i128,
    pub outcomes: Vec<UserRoundOutcome>,
}

/// Admin-configured blueprint for `create_next_from_template`.
///
/// Mirrors the arguments accepted by `create_round` (`start_price`, `mode`)
/// so a keeper can spin up the next round after a settle/cancel without an
/// operator re-specifying parameters each time. Validated with the exact
/// same rules `create_round` applies at creation time.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RoundTemplate {
    pub start_price: u128,
    pub mode: Option<u32>,
}

/// A single entry in the lifetime (all-time) leaderboard.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LeaderboardEntry {
    pub user: Address,
    pub stats: UserStats,
}

/// A single entry in a season-scoped leaderboard, live or archived.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonLeaderboardEntry {
    pub user: Address,
    pub wins: u32,
    pub best_streak: u32,
}

/// Frozen snapshot of a season's final bounded rankings, written by
/// `reset_leaderboard_season`. `participant_count` is the number of distinct
/// addresses that appeared in either bounded index at reset time (a lower
/// bound on total season participants beyond the tracked top
/// `LEADERBOARD_LIMIT`, mirroring the same bound the live indexes enforce).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonArchive {
    pub season_id: u32,
    pub ended_at_ledger: u32,
    pub wins: Vec<SeasonLeaderboardEntry>,
    pub streak: Vec<SeasonLeaderboardEntry>,
    pub participant_count: u32,
}
// SPDX-License-Identifier: MIT
//! Type definitions for the XLM Price Prediction Market.

use soroban_sdk::{contracttype, Address, BytesN, Env, IntoVal, Symbol, Val, Vec};

/// Round mode for prediction type
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundMode {
    UpDown = 0,    // Simple up/down predictions
    Precision = 1, // Exact price predictions (Legends mode)
}

/// Payout policy for Precision mode
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum PrecisionPayoutPolicy {
    Equal = 0,         // Split payout pool equally among winners (default)
    StakeWeighted = 1, // Split payout pool proportionally to winner stakes
}

/// Runtime mode for the contract lifecycle
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum RuntimeMode {
    Normal = 0,
    ClaimsOnly = 1,
    FullyPaused = 2,
}

/// Lifecycle phase of an active round, derived from ledger windows.
///
/// Semantics (given `start_ledger`, `bet_end_ledger`, `end_ledger`):
/// - `Betting`: `ledger < bet_end_ledger` — bets and precision predictions accepted
/// - `Running`: `bet_end_ledger ≤ ledger < end_ledger` — reveal window (precision)
/// - `Resolvable`: `ledger ≥ end_ledger` — round may be settled via oracle payload
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundPhase {
    Betting = 1,
    Running = 2,
    Resolvable = 3,
}

/// Storage keys for contract data
///
/// ## Indexed position keys (variants 13–15)
///
/// `Position(round_id, address)` and `PrecisionPosition(round_id, address)` store
/// a single user's record under a composite key, enabling O(1) read/write per user
/// instead of deserializing the full participant map on every bet.
///
/// `RoundParticipants(round_id)` holds the ordered `Vec<Address>` used for
/// iteration at resolution time. Appending one address is cheaper than
/// re-serialising an N-entry `Map<Address, T>` for every bet placed.
///
/// Legacy single-key maps (`UpDownPositions`, `PrecisionPositions`) are kept for
/// backward-compatible reads during a migration window; they are no longer written.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Balance(Address),
    Admin,
    Oracle,
    /// On-chain storage schema version for migration safety.
    /// If missing, the contract treats it as legacy schema version 1.
    SchemaVersion,
    ActiveRound,
    Positions,          // Legacy key — read-only migration compat
    UpDownPositions,    // Legacy key — read-only migration compat
    PrecisionPositions, // Legacy key — read-only migration compat
    PendingWinnings(Address),
    UserStats(Address),
    Paused,
    BetWindowLedgers,
    RunWindowLedgers,
    CloseBufferLedgers,
    LastRoundId,
    /// Per-user UpDown position: (round_id, address) → UserPosition
    Position(u64, Address),
    /// Per-user Precision prediction: (round_id, address) → PrecisionPrediction
    PrecisionPosition(u64, Address),
    /// Per-user Precision commitment: (round_id, address) → PrecisionCommitment
    PrecisionCommitment(u64, Address),
    /// Ordered participant list for a round: round_id → Vec<Address>
    RoundParticipants(u64),
    /// Maximum stake allowed per individual bet (None = unlimited)
    MaxStake,
    /// Maximum cumulative exposure per user per round (None = unlimited)
    MaxUserRoundExposure,
    /// Maximum pending winnings allowed per account (None = unlimited)
    MaxPendingWinnings,
    /// Marker for a cancelled round: round_id → true
    CancelledRound(u64),
    /// Per-round consumed oracle nonce: (round_id, nonce) → true.
    /// Used to reject duplicate oracle payload submissions for the same round.
    ConsumedOracleNonce(u64, u64),
    /// Minimum participant count for competitive settlement; unset = no minimum enforced
    MinParticipants,
    /// Oracle heartbeat: last recorded timestamp and status
    OracleHeartbeat,
    /// Stale-heartbeat threshold in seconds (admin-configurable); unset = 3600 s default
    OracleStaleThreshold,
    /// Maximum participants accepted in a Precision round; unset = protocol default
    MaxPrecisionParticipants,
    /// Oracle max deviation threshold in basis points (1 bp = 0.01%).
    /// If unset, deviation guardrails are disabled.
    OracleMaxDeviationBps,
    /// One-shot admin override allowing the next settlement to bypass deviation checks.
    /// Automatically cleared after use.
    OracleDeviationOverrideArmed,
    /// Minimum oracle confidence threshold in basis points (0–10000).
    /// If unset, confidence guardrails are disabled.
    OracleMinConfidenceBps,
    /// When true, payloads with missing confidence are rejected in strict mode.
    OracleStrictMode,
    /// Compact post-settlement summary keyed by round id for historical queries.
    ArchivedRound(u64),
    /// Ordered round ids for archive retention (oldest at index 0).
    RecentArchivedRoundIds,
    /// Per-user outcome record for a specific archived round (round_id, user).
    /// Persisted at settlement for user history queries without event replay.
    UserRoundOutcome(u64, Address),
    /// Marker written by migrate_schema_v2_to_v3 to prove the migration ran.
    MigratedToV3,
    /// Timelocked pending critical config change keyed by change kind.
    PendingConfigChange(ConfigChangeKind),
    /// Optional protocol settlement fee in basis points (1 bp = 0.01%).
    /// `None` (key absent) means fee disabled — no behaviour change.
    /// Hard cap on fee is enforced at the contract layer, not by storage shape.
    ProtocolFeeBps,
    /// On-chain accumulated protocol fee balance in stroops (i128).
    /// Admin withdraws via the dedicated withdrawal method; does NOT mix
    /// into the per-user balance ledger.
    ProtocolFeeTreasury,
    /// Per-ledger mint counter: wraps the explicit ledger sequence number.
    LedgerMintCounter(u32),
    /// Mint limit configuration: maximum number of mints allowed per ledger.
    MintLimitConfig,
    /// Pending two-step oracle rotation proposal with expiry.
    OracleRotationProposal,
    /// Configurable archive retention limit: maximum number of ArchivedRound entries
    /// retained on-chain before the oldest are pruned (FIFO). If unset, the protocol
    /// default is used.
    ArchiveRetention,
    /// Admin-configured blueprint used by `create_next_from_template` to spin
    /// up the next round without re-specifying `start_price` / `mode` each
    /// time. Absent means no template is configured.
    RoundTemplate,
    /// Bounded index of user addresses sorted by lifetime total wins
    /// descending (all-time leaderboard, independent of seasons).
    LeaderboardWins,
    /// Bounded index of user addresses sorted by lifetime best streak
    /// descending (all-time leaderboard, independent of seasons).
    LeaderboardStreak,
    /// Monotonically increasing id of the currently-active leaderboard
    /// season. Absent is treated as season 1.
    SeasonId,
    /// Per-season, per-user win/loss/streak stats: (season_id, address) →
    /// UserStats, scoped independently of the lifetime `UserStats` totals so
    /// a season reset never touches lifetime history.
    SeasonUserStats(u32, Address),
    /// Bounded index of user addresses in the *active* season sorted by
    /// season-scoped total wins descending.
    SeasonLeaderboardWins,
    /// Bounded index of user addresses in the *active* season sorted by
    /// season-scoped best streak descending.
    SeasonLeaderboardStreak,
    /// Frozen snapshot of a season's final rankings, written when the season
    /// is reset. Seasons are never deleted — this is a permanent archive.
    SeasonArchive(u32),
    /// Admin-configured multi-feed oracle quorum parameters.
    /// When set, `resolve_round_multi` is enabled.
    OracleQuorum,
    /// Announced next schema version for migration preview (v-next template).
    /// When set, operators can inspect this value before executing a real migration.
    /// Absent means no next migration has been announced.
    NextSchemaVersion,
    /// Minimum bet amount (dust protection). Unset = no minimum.
    MinBet,
    /// Epoch mint budget: total mints allowed per epoch.
    EpochMintBudget,
    /// Early cash-out penalty in basis points. Unset = early cash-out disabled.
    EarlyCashoutBps,
    /// Fee incidence model: FeeOnPot (default) or FeeOnWinnings.
    FeeModel,
    /// Dispute window length in ledgers. 0 = no dispute window.
    DisputeLedgers,
}

/// Identifies which critical risk setting is pending timelocked activation.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ConfigChangeKind {
    Windows = 0,
    MaxStake = 1,
    MaxUserRoundExposure = 2,
    MaxPendingWinnings = 3,
    OracleStaleThreshold = 4,
    OracleMaxDeviationBps = 5,
    ProtocolFeeBps = 6,
    MinParticipants = 7,
    MaxPrecisionParticipants = 8,
    MintLimit = 9,
    ArchiveRetention = 10,
    CloseBufferLedgers = 11,
    EpochMintBudget = 12,
    PendingWinningsExpiry = 13,
    PrecisionPayoutPolicy = 14,
    MinBet = 15,
    DisputeLedgers = 16,
    FeeModel = 17,
}

/// Payload for a scheduled critical config change.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigChangePayload {
    Windows(u32, u32),
    MaxStake(Option<i128>),
    MaxUserRoundExposure(Option<i128>),
    MaxPendingWinnings(Option<i128>),
    OracleStaleThreshold(u64),
    OracleMaxDeviationBps(Option<u32>),
    ProtocolFeeBps(Option<u32>),
    MinParticipants(Option<u32>),
    MaxPrecisionParticipants(u32),
    MintLimit(u32),
    ArchiveRetention(u32),
    CloseBufferLedgers(u32),
    EpochMintBudget(i128),
    PendingWinningsExpiry(u32),
    PrecisionPayoutPolicy(u32),
    MinBet(Option<i128>),
    DisputeLedgers(u32),
    FeeModel(FeeModel),
}

/// Pending timelocked config change with activation ledger for on-chain observability.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingConfigChange {
    pub payload: ConfigChangePayload,
    pub activation_ledger: u32,
    pub scheduled_at_ledger: u32,
}

/// Represents which side a user bet on
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum BetSide {
    Up,
    Down,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserPosition {
    pub amount: i128,
    pub side: BetSide,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserStats {
    pub total_wins: u32,
    pub total_losses: u32,
    pub current_streak: u32,
    pub best_streak: u32,
}

/// Precision prediction entry (user address + predicted price)
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PrecisionPrediction {
    pub user: Address,
    pub predicted_price: u128, // Price scaled to 4 decimals (e.g., 0.2297 → 2297)
    pub amount: i128,          // Bet amount
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PrecisionCommitment {
    pub hash: BytesN<32>,
    pub amount: i128,
    pub revealed: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OraclePayload {
    pub price: u128,
    pub timestamp: u64,
    /// Round identifier that should match `Round.start_ledger`
    pub round_id: u32,
    /// Per-round replay-protection nonce.
    pub nonce: u64,
    /// SHA-256 hash of the network passphrase this payload targets.
    pub network_id: BytesN<32>,
    /// Contract address this payload is intended for.
    pub contract_addr: Address,
    /// Optional confidence score from the price feed (0–10000 bps, where 10000 = 100%).
    pub confidence: Option<u32>,
    /// Optional ed25519 signature over the attestation domain-separated message.
    pub attestation: Option<BytesN<64>>,
}

/// Multi-feed oracle resolution payload (N observations, quorum + median).
///
/// Unlike the legacy single-oracle `OraclePayload`, this carries N independent
/// feed observations as parallel arrays. The contract computes the median,
/// rejects outliers, and requires a configurable quorum of feeds to agree
/// within the outlier threshold before settlement proceeds.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MultiFeedPayload {
    /// Prices from each feed, scaled to 4 decimal places (e.g. 2297 = $0.2297).
    pub prices: Vec<u128>,
    /// Feed source identifiers (0-based index, max N-1). Must be unique.
    pub sources: Vec<u32>,
    /// Round identifier that must match `Round.start_ledger`
    pub round_id: u32,
    /// Per-round replay-protection nonce.
    pub nonce: u64,
    /// SHA-256 hash of the network passphrase this payload targets.
    pub network_id: BytesN<32>,
    /// Contract address this payload is intended for.
    pub contract_addr: Address,
    /// Unix epoch seconds when the observations were collected.
    pub timestamp: u64,
}

/// Admin-configurable quorum and outlier rejection parameters for multi-feed
/// oracle settlement. Stored under `DataKey::OracleQuorum`.
///
/// When set, `resolve_round_multi` becomes the preferred settlement path.
/// The legacy single-oracle `resolve_round` path remains available
/// independently of this configuration.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OracleQuorumConfig {
    /// Minimum number of unique feed observations required in a multi-feed payload.
    pub min_observations: u32,
    /// Minimum number of observations that must survive outlier rejection to
    /// form a valid quorum and proceed to settlement.
    pub quorum_threshold: u32,
    /// Maximum deviation from the median (in basis points, 1 bp = 0.01%)
    /// before an observation is rejected as an outlier.
    pub outlier_threshold_bps: u32,
}

/// Oracle liveness record, updated by the oracle service on each heartbeat call.
/// `status`: 0 = active, 1 = degraded, 2 = offline.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OracleHeartbeatRecord {
    pub timestamp: u64,
    pub status: u32,
}

/// Heartbeat health gate configuration (Issue #264).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HbGateConfig {
    pub strict_mode: bool,
    pub override_armed: bool,
    pub grace_seconds: u64,
}

/// Storage key for heartbeat gate config (separate from DataKey to stay within variant limits, Issue #264).
#[contracttype]
#[derive(Clone)]
pub enum HbGateKey {
    Config,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    pub round_id: u64,       // Unique monotonically increasing round identifier
    pub price_start: u128,   // Starting XLM price in stroops
    pub start_ledger: u32,   // Ledger when round was created
    pub bet_end_ledger: u32, // Ledger when betting closes
    pub end_ledger: u32,     // Ledger when round ends (~5s per ledger)
    pub pool_up: i128,       // Total vXLM bet on UP
    pub pool_down: i128,     // Total vXLM bet on DOWN
    pub mode: RoundMode,     // Round mode: UpDown (0) or Precision (1)
}

/// Aggregated active-round pool composition for frontend transparency.
///
/// Up/Down rounds populate the up/down pools, counts, and stake ratios.
/// Precision rounds populate the precision totals and participant counters while
/// leaving side-specific Up/Down fields at zero. Ratios are basis points of
/// the mode's total visible stake (10_000 = 100%).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RoundPoolStats {
    pub round_id: u64,
    pub mode: RoundMode,
    pub total_up_stake: i128,
    pub total_down_stake: i128,
    pub up_participant_count: u32,
    pub down_participant_count: u32,
    pub up_stake_ratio_bps: u32,
    pub down_stake_ratio_bps: u32,
    pub precision_total_stake: i128,
    pub precision_participant_count: u32,
    pub precision_prediction_count: u32,
    pub precision_commitment_count: u32,
    pub precision_revealed_count: u32,
}

/// Terminal outcome recorded when a round leaves the active state.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundArchiveStatus {
    /// Oracle settlement completed (normal resolution path).
    Resolved = 0,
    /// Admin cancelled the round and refunded participants.
    Cancelled = 1,
    /// Settlement aborted due to insufficient participants; stakes refunded.
    FallbackRefund = 2,
    /// Dispute window ended via void; all participants refunded their stake.
    Voided = 3,
}

/// Composite protocol health status returned by `get_protocol_health`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolHealthStatus {
    pub paused: bool,
    pub oracle_live: bool,
    pub oracle_status: u32,
    pub has_active_round: bool,
    pub active_round_phase: u32,
    pub schema_version: u32,
    pub ledger_sequence: u32,
    pub ledger_timestamp: u64,
    pub status_code: u32,
}

/// Compact historical round summary persisted after resolve or cancel.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ArchivedRoundSummary {
    pub round_id: u64,
    pub price_start: u128,
    pub price_final: u128,
    pub mode: RoundMode,
    pub status: RoundArchiveStatus,
    pub pool_up: i128,
    pub pool_down: i128,
    pub participant_count: u32,
    pub settled_at_ledger: u32,
}

/// Pending two-step oracle rotation proposal.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OracleRotationProposal {
    pub new_oracle: Address,
    pub proposed_at: u64,
    pub expires_at: u64,
}

/// Global status of the protocol, returned by `get_protocol_status`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ProtocolStatus {
    Active = 0,
    Paused = 1,
    ClaimsOnly = 2,
}

/// Status of a specific round, returned by `get_round_status(round_id)`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum RoundStatus {
    Unknown = 0,
    Betting = 1,
    Running = 2,
    AwaitingResolve = 3,
    Resolved = 4,
    Cancelled = 5,
    FallbackRefund = 6,
    Voided = 7,
}

/// Terminal outcome persisted per user per archived round.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum UserOutcomeType {
    Win = 0,
    Loss = 1,
    Refund = 2,
    Void = 3,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UserRoundOutcome {
    pub user: Address,
    pub round_mode: u32,
    pub prediction_side: u32,
    pub predicted_price: u128,
    pub stake: i128,
    pub payout: i128,
    pub outcome: UserOutcomeType,
}

/// Simulated payout result for a specific hypothetical final price.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SimulationResult {
    pub mode: RoundMode,
    pub pool_up: i128,
    pub pool_down: i128,
    pub precision_total_stake: i128,
    pub fee_amount: i128,
    pub outcomes: Vec<UserRoundOutcome>,
    pub fee_model: u32,
}

/// Admin-configured blueprint for `create_next_from_template`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RoundTemplate {
    pub start_price: u128,
    pub mode: Option<u32>,
}

/// A single entry in the lifetime (all-time) leaderboard.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LeaderboardEntry {
    pub user: Address,
    pub stats: UserStats,
}

/// A single entry in a season-scoped leaderboard, live or archived.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonLeaderboardEntry {
    pub user: Address,
    pub wins: u32,
    pub best_streak: u32,
}

/// Frozen snapshot of a season's final bounded rankings.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonArchive {
    pub season_id: u32,
    pub ended_at_ledger: u32,
    pub wins: Vec<SeasonLeaderboardEntry>,
    pub streak: Vec<SeasonLeaderboardEntry>,
    pub participant_count: u32,
}

/// Configurable pending-winnings expiry in ledgers.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PendingWinningsExpiryKey(pub ());

pub const PENDING_WINNINGS_EXPIRY_KEY: PendingWinningsExpiryKey = PendingWinningsExpiryKey(());

/// Ledger sequence when a user's pending winnings entry was last modified.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PendingWinningsUpdatedAtKey(pub Address);

/// Fee incidence model for protocol fees (Issue #268).
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum FeeModel {
    FeeOnPot = 0,      // Fee charged on total pot (default)
    FeeOnWinnings = 1, // Fee charged only on net winnings/profit
}

/// TWAP sample ring entry (Issue #266).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PriceSample {
    pub price: u128,
    pub timestamp: u64,
}

/// Storage key for TWAP samples ring (separate from DataKey to stay within variant limits, Issue #266).
#[contracttype]
#[derive(Clone)]
pub enum TwapSamplesKey {
    Samples,
}

/// Dev Reference Mode (Issue #266).
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum DeviationReferenceMode {
    StartPrice = 0, // Use round.price_start (default)
    Twap = 1,       // Use trailing-sample TWAP average
}

/// Deviation guardrail config (Issue #266).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeviationConfig {
    pub reference_mode: DeviationReferenceMode,
    pub window_samples: u32,
}

/// Storage key for deviation config (separate from DataKey to stay within variant limits, Issue #266).
#[contracttype]
#[derive(Clone)]
pub enum DeviationConfigKey {
    Config,
}

/// Oracle attestation config (Issue #263).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AttestationConfig {
    pub key: Option<BytesN<32>>, // ed25519 public key; None = attestation disabled
}

/// Storage key for attestation config (separate from DataKey to stay within variant limits, Issue #263).
#[contracttype]
#[derive(Clone)]
pub enum AttestationConfigKey {
    Config,
}

impl IntoVal<Env, Val> for DataKey {
    fn into_val(&self, env: &Env) -> Val {
        use Symbol as S;
        match self {
            DataKey::Balance(a) => (S::new(env, "Balance"), a.clone()).into_val(env),
            DataKey::Admin => S::new(env, "Admin").into_val(env),
            DataKey::Oracle => S::new(env, "Oracle").into_val(env),
            DataKey::SchemaVersion => S::new(env, "SchemaVersion").into_val(env),
            DataKey::ActiveRound => S::new(env, "ActiveRound").into_val(env),
            DataKey::Positions => S::new(env, "Positions").into_val(env),
            DataKey::UpDownPositions => S::new(env, "UpDownPositions").into_val(env),
            DataKey::PrecisionPositions => S::new(env, "PrecisionPositions").into_val(env),
            DataKey::PendingWinnings(a) => {
                (S::new(env, "PendingWinnings"), a.clone()).into_val(env)
            }
            DataKey::UserStats(a) => (S::new(env, "UserStats"), a.clone()).into_val(env),
            DataKey::Paused => S::new(env, "Paused").into_val(env),
            DataKey::BetWindowLedgers => S::new(env, "BetWindowLedgers").into_val(env),
            DataKey::RunWindowLedgers => S::new(env, "RunWindowLedgers").into_val(env),
            DataKey::CloseBufferLedgers => S::new(env, "CloseBufferLedgers").into_val(env),
            DataKey::LastRoundId => S::new(env, "LastRoundId").into_val(env),
            DataKey::Position(id, a) => {
                (S::new(env, "Position"), id, a.clone()).into_val(env)
            }
            DataKey::PrecisionPosition(id, a) => {
                (S::new(env, "PrecisionPosition"), id, a.clone()).into_val(env)
            }
            DataKey::PrecisionCommitment(id, a) => {
                (S::new(env, "PrecisionCommitment"), id, a.clone()).into_val(env)
            }
            DataKey::RoundParticipants(id) => {
                (S::new(env, "RoundParticipants"), id).into_val(env)
            }
            DataKey::MaxStake => S::new(env, "MaxStake").into_val(env),
            DataKey::MaxUserRoundExposure => S::new(env, "MaxUserRoundExposure").into_val(env),
            DataKey::MaxPendingWinnings => S::new(env, "MaxPendingWinnings").into_val(env),
            DataKey::CancelledRound(id) => (S::new(env, "CancelledRound"), id).into_val(env),
            DataKey::ConsumedOracleNonce(id, nonce) => {
                (S::new(env, "ConsumedOracleNonce"), id, nonce).into_val(env)
            }
            DataKey::MinParticipants => S::new(env, "MinParticipants").into_val(env),
            DataKey::OracleHeartbeat => S::new(env, "OracleHeartbeat").into_val(env),
            DataKey::OracleStaleThreshold => S::new(env, "OracleStaleThreshold").into_val(env),
            DataKey::MaxPrecisionParticipants => {
                S::new(env, "MaxPrecisionParticipants").into_val(env)
            }
            DataKey::OracleMaxDeviationBps => S::new(env, "OracleMaxDeviationBps").into_val(env),
            DataKey::OracleDeviationOverrideArmed => {
                S::new(env, "OracleDeviationOverrideArmed").into_val(env)
            }
            DataKey::OracleMinConfidenceBps => {
                S::new(env, "OracleMinConfidenceBps").into_val(env)
            }
            DataKey::OracleStrictMode => S::new(env, "OracleStrictMode").into_val(env),
            DataKey::ArchivedRound(id) => (S::new(env, "ArchivedRound"), id).into_val(env),
            DataKey::RecentArchivedRoundIds => {
                S::new(env, "RecentArchivedRoundIds").into_val(env)
            }
            DataKey::UserRoundOutcome(id, a) => {
                (S::new(env, "UserRoundOutcome"), id, a.clone()).into_val(env)
            }
            DataKey::MigratedToV3 => S::new(env, "MigratedToV3").into_val(env),
            DataKey::PendingConfigChange(k) => {
                (S::new(env, "PendingConfigChange"), k.clone()).into_val(env)
            }
            DataKey::ProtocolFeeBps => S::new(env, "ProtocolFeeBps").into_val(env),
            DataKey::ProtocolFeeTreasury => S::new(env, "ProtocolFeeTreasury").into_val(env),
            DataKey::LedgerMintCounter(id) => {
                (S::new(env, "LedgerMintCounter"), id).into_val(env)
            }
            DataKey::MintLimitConfig => S::new(env, "MintLimitConfig").into_val(env),
            DataKey::OracleRotationProposal => S::new(env, "OracleRotationProposal").into_val(env),
            DataKey::ArchiveRetention => S::new(env, "ArchiveRetention").into_val(env),
            DataKey::RoundTemplate => S::new(env, "RoundTemplate").into_val(env),
            DataKey::LeaderboardWins => S::new(env, "LeaderboardWins").into_val(env),
            DataKey::LeaderboardStreak => S::new(env, "LeaderboardStreak").into_val(env),
            DataKey::SeasonId => S::new(env, "SeasonId").into_val(env),
            DataKey::SeasonUserStats(sid, a) => {
                (S::new(env, "SeasonUserStats"), sid, a.clone()).into_val(env)
            }
            DataKey::SeasonLeaderboardWins => S::new(env, "SeasonLeaderboardWins").into_val(env),
            DataKey::SeasonLeaderboardStreak => {
                S::new(env, "SeasonLeaderboardStreak").into_val(env)
            }
            DataKey::SeasonArchive(id) => (S::new(env, "SeasonArchive"), id).into_val(env),
            DataKey::OracleQuorum => S::new(env, "OracleQuorum").into_val(env),
            DataKey::NextSchemaVersion => S::new(env, "NextSchemaVersion").into_val(env),
            DataKey::MinBet => S::new(env, "MinBet").into_val(env),
            DataKey::EpochMintBudget => S::new(env, "EpochMintBudget").into_val(env),
            DataKey::EarlyCashoutBps => S::new(env, "EarlyCashoutBps").into_val(env),
            DataKey::FeeModel => S::new(env, "FeeModel").into_val(env),
            DataKey::DisputeLedgers => S::new(env, "DisputeLedgers").into_val(env),
        }
    }
}

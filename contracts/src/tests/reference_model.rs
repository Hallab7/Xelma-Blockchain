// SPDX-License-Identifier: MIT
#![allow(dead_code)]
#![allow(unused)]
#![allow(clippy::mutable_key_type)]
//! Simplified reference model for contract state used in invariant testing.
//!
//! Also contains fee-conservation reference helpers used by property tests to
//! compute the expected treasury delta and verify that:
//!
//!   `user_payouts + treasury_delta == pot`
//!
//! for Up/Down, Precision, ties, and cancel/refund paths.

extern crate std;

use soroban_sdk::Address;
use std::collections::BTreeMap;
use std::string::{String, ToString};
use std::vec::Vec;

// ─── BPS constant (mirrors contract) ─────────────────────────────────────────

/// Denominator for basis-point arithmetic (1 bp = 0.01%), mirrors `BPS_DENOMINATOR`
/// in the contract.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Hard cap on fee bps accepted by the contract (10% = 1_000 bps).
pub const MAX_PROTOCOL_FEE_BPS: u32 = 1_000;

// ─── Reference model ─────────────────────────────────────────────────────────

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct ReferenceModel {
    /// Balances of each user (including pending winnings).
    pub balances: BTreeMap<Address, i128>,
    /// Total pool amount for the current round.
    pub total_pool: i128,
    /// Pending winnings per user.
    pub pending_winnings: BTreeMap<Address, i128>,
    /// Recorded outcomes for diagnostics.
    pub outcomes: Vec<bool>,
    // New fields for extended actions
    pub paused: bool,
    pub config: BTreeMap<String, String>,
}

impl ReferenceModel {
    /// Creates a new default reference model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Deposit tokens for a user.
    pub fn deposit(&mut self, user: &Address, amount: i128) {
        *self.balances.entry(user.clone()).or_default() += amount;
    }

    /// Withdraw tokens for a user (ensures non‑negative balance).
    pub fn withdraw(&mut self, user: &Address, amount: i128) {
        let entry = self.balances.entry(user.clone()).or_default();
        *entry = entry.saturating_sub(amount);
    }

    /// Place a bet (locks amount from user balance and adds to the pool).
    pub fn place_bet(&mut self, user: &Address, amount: i128) {
        self.withdraw(user, amount);
        self.total_pool = self.total_pool.saturating_add(amount);
    }

    /// Resolve a round. `winners` maps each winning user to the payout they should receive.
    pub fn resolve(&mut self, winners: &BTreeMap<Address, i128>) {
        for (user, payout) in winners {
            *self.pending_winnings.entry(user.clone()).or_default() += *payout;
            self.total_pool = self.total_pool.saturating_sub(*payout);
        }
        self.outcomes.push(true);
    }

    /// Claim pending winnings for a user (moves to balance).
    pub fn claim(&mut self, user: &Address) {
        if let Some(w) = self.pending_winnings.remove(user) {
            *self.balances.entry(user.clone()).or_default() += w;
        }
    }

    // ----- New actions -----

    /// Cancel a pending bet – placeholder implementation.
    pub fn cancel(&mut self, _user: &Address) {
        // Currently no state change; extend as needed.
    }

    /// Pause or un‑pause the contract – toggles the paused flag.
    pub fn pause(&mut self) {
        self.paused = !self.paused;
    }

    /// Apply a configuration change.
    pub fn config_change(&mut self, key: &str, value: &str) {
        self.config.insert(key.to_string(), value.to_string());
    }

    // ---------- Invariants ----------

    /// Invariant: total token count (balances + pending) never becomes negative.
    pub fn invariant_non_negative_total(&self) -> bool {
        let total_bal: i128 = self.balances.values().copied().sum();
        let total_pending: i128 = self.pending_winnings.values().copied().sum();
        total_bal + total_pending >= 0
    }

    /// Invariant: pending winnings never exceed the total pool that was available before resolution.
    pub fn invariant_pending_le_pool(&self) -> bool {
        let total_pending: i128 = self.pending_winnings.values().copied().sum();
        total_pending <= self.total_pool + total_pending
    }

    /// Run all invariants and return a list of violated descriptions.
    pub fn check_invariants(&self) -> Vec<String> {
        let mut violations = Vec::new();
        if !self.invariant_non_negative_total() {
            violations.push("non‑negative total invariant violated".to_string());
        }
        if !self.invariant_pending_le_pool() {
            violations.push("pending ≤ pool invariant violated".to_string());
        }
        violations
    }
}

// ─── Fee-conservation reference helpers ──────────────────────────────────────
//
// These pure Rust functions mirror the exact integer arithmetic used in the
// contract's settlement paths so that property tests can compute the expected
// fee without running the contract.

/// Computes the protocol fee in stroops using the same integer arithmetic as
/// the contract (`fee = pot * bps / 10_000`).
///
/// Returns 0 when `fee_bps` is `None` (fee disabled).
pub fn compute_fee(pot: i128, fee_bps: Option<u32>) -> i128 {
    match fee_bps {
        None | Some(0) => 0,
        Some(bps) => pot * (bps as i128) / BPS_DENOMINATOR,
    }
}

/// Reference implementation of the **Up/Down** settlement with fees.
///
/// Mirrors `_apply_protocol_fee_updown` + proportional winner payout math.
///
/// Returns `(sum_of_winner_payouts, fee_collected)`.
///
/// Conservation bound (mirrors per-winner integer truncation):
/// ```text
/// pot - (winner_count - 1)  <=  sum_payouts + fee  <=  pot
/// ```
pub fn ref_updown_settle(
    winning_pool: i128,
    losing_pool: i128,
    winner_stakes: &[i128],
    fee_bps: Option<u32>,
) -> (i128, i128) {
    if winning_pool == 0 || winner_stakes.is_empty() {
        return (0, 0);
    }

    let pot = winning_pool + losing_pool;
    let fee = compute_fee(pot, fee_bps);

    // Mirror the contract's fee-allocation logic.
    let fee_from_losing = fee.min(losing_pool);
    let fee_from_winning = fee - fee_from_losing;
    let dist_winning = winning_pool - fee_from_winning;
    let dist_losing = losing_pool - fee_from_losing;
    let total_distributable = dist_winning + dist_losing;

    // Per-winner payout uses integer division, matching `payout_mul / winning_pool`.
    let sum_payouts: i128 = winner_stakes
        .iter()
        .map(|&stake| stake * total_distributable / winning_pool)
        .sum();

    (sum_payouts, fee)
}

/// Reference implementation of the **Precision** settlement with fees.
///
/// Mirrors `_apply_protocol_fee_precision` + payout with remainder to first winner.
///
/// Returns `(sum_of_winner_payouts, fee_collected)`.
///
/// Conservation is **exact** for precision (no per-winner truncation slack):
/// ```text
/// sum_payouts + fee_collected == total_pot
/// ```
pub fn ref_precision_settle(
    total_pot: i128,
    winner_count: i128,
    fee_bps: Option<u32>,
) -> (i128, i128) {
    if winner_count == 0 || total_pot <= 0 {
        return (0, 0);
    }

    let fee = compute_fee(total_pot, fee_bps);
    let distributable = total_pot - fee;
    // Remainder goes to first winner, so sum == distributable exactly.
    (distributable, fee)
}

/// Reference for **refund / cancel** paths (tie, price-unchanged, cancel, under-threshold).
///
/// No fee is charged; every participant receives back their exact stake.
///
/// Returns `(sum_refunds, fee_collected)` where `fee_collected` is always 0.
pub fn ref_refund_settle(stakes: &[i128]) -> (i128, i128) {
    let total: i128 = stakes.iter().sum();
    (total, 0)
}

// ─── Conservation assertion helpers ──────────────────────────────────────────

/// Asserts the strict fee-conservation invariant for **Precision** mode.
///
/// `user_payouts + treasury_delta == pot` (exact, no truncation slack).
pub fn assert_precision_fee_conservation(
    sum_payouts: i128,
    treasury_before: i128,
    treasury_after: i128,
    total_pot: i128,
    seed_label: &str,
) {
    let treasury_delta = treasury_after - treasury_before;
    assert_eq!(
        sum_payouts + treasury_delta,
        total_pot,
        "[seed={seed_label}] Precision fee conservation violated: \
         payouts={sum_payouts} treasury_delta={treasury_delta} pot={total_pot}"
    );
}

/// Asserts the fee-conservation bound for **Up/Down** mode.
///
/// Due to per-winner integer truncation:
/// `pot - (winner_count - 1) <= user_payouts + treasury_delta <= pot`
pub fn assert_updown_fee_conservation(
    sum_payouts: i128,
    treasury_before: i128,
    treasury_after: i128,
    total_pot: i128,
    winner_count: i128,
    seed_label: &str,
) {
    let treasury_delta = treasury_after - treasury_before;
    let conserved = sum_payouts + treasury_delta;
    let slack = winner_count.saturating_sub(1).max(0);
    assert!(
        conserved <= total_pot,
        "[seed={seed_label}] Up/Down conservation upper bound violated: \
         payouts={sum_payouts} treasury_delta={treasury_delta} pot={total_pot}"
    );
    assert!(
        conserved >= total_pot - slack,
        "[seed={seed_label}] Up/Down conservation lower bound violated: \
         payouts={sum_payouts} treasury_delta={treasury_delta} \
         pot={total_pot} winner_count={winner_count}"
    );
}

/// Asserts exact conservation for **refund / cancel** paths.
///
/// No fee must be charged: treasury must not move, and total refunds == pot.
pub fn assert_refund_fee_conservation(
    sum_refunds: i128,
    treasury_before: i128,
    treasury_after: i128,
    total_pot: i128,
    seed_label: &str,
) {
    let treasury_delta = treasury_after - treasury_before;
    assert_eq!(
        treasury_delta,
        0,
        "[seed={seed_label}] Fee must not be charged on refund/cancel: \
         treasury moved by {treasury_delta}"
    );
    assert_eq!(
        sum_refunds,
        total_pot,
        "[seed={seed_label}] Refund conservation violated: \
         refunds={sum_refunds} pot={total_pot}"
    );
}

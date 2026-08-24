#!/usr/bin/env python3
"""Repair duplicated types.rs and add missing split-key aliases for compilation."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TYPES = ROOT / "contracts/src/types.rs"

INSERT_BEFORE_INTOVAL = '''
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

/// Participant access-control state (Issue #274).
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum AccessState {
    Open = 0,
    Allowlisted = 1,
    Denylisted = 2,
}

/// Policy gate action classes (Issue #261).
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum PolicyAction {
    RoundMutation = 0,
    Claim = 1,
    AdminConfig = 2,
    Settlement = 3,
}

/// Core singleton storage keys (alias over [`DataKey`] during schema split).
pub type DataKeyCore = DataKey;
/// Scoped / composite storage keys (alias over [`DataKey`] during schema split).
pub type DataKeyScoped = DataKey;

'''

EXTRA_VARIANTS = '''
    /// Optional participant access control master switch.
    AccessControlEnabled,
    /// Emergency denylist marker for a participant address.
    Denylisted(Address),
    /// Allowlist marker for a participant address.
    Allowlisted(Address),
    /// Precision payout policy configuration.
    PrecisionPayoutPolicy,
    /// Extension bucket for leaderboard/season keys (XDR variant limit workaround).
    Ext(DataKeyExt),
'''

EXTRA_MATCH_ARMS = '''
            DataKey::AccessControlEnabled => S::new(env, "AccessControlEnabled").into_val(env),
            DataKey::Denylisted(a) => (S::new(env, "Denylisted"), a.clone()).into_val(env),
            DataKey::Allowlisted(a) => (S::new(env, "Allowlisted"), a.clone()).into_val(env),
            DataKey::PrecisionPayoutPolicy => S::new(env, "PrecisionPayoutPolicy").into_val(env),
            DataKey::Ext(ext) => match ext {
                DataKeyExt::LeaderboardWins => S::new(env, "ExtLeaderboardWins").into_val(env),
                DataKeyExt::LeaderboardStreak => S::new(env, "ExtLeaderboardStreak").into_val(env),
                DataKeyExt::SeasonId => S::new(env, "ExtSeasonId").into_val(env),
                DataKeyExt::SeasonUserStats(sid, a) => {
                    (S::new(env, "ExtSeasonUserStats"), sid, a.clone()).into_val(env)
                }
                DataKeyExt::SeasonLeaderboardWins => {
                    S::new(env, "ExtSeasonLeaderboardWins").into_val(env)
                }
                DataKeyExt::SeasonLeaderboardStreak => {
                    S::new(env, "ExtSeasonLeaderboardStreak").into_val(env)
                }
                DataKeyExt::SeasonArchive(id) => {
                    (S::new(env, "ExtSeasonArchive"), id).into_val(env)
                }
            },
'''


def main() -> None:
    text = TYPES.read_text(encoding="utf-8")
    # Keep only the first complete copy (ends after first IntoVal impl).
    marker = "\n// SPDX-License-Identifier: MIT\n//! Type definitions for the XLM Price Prediction Market.\n\nuse soroban_sdk::{contracttype, Address, BytesN, Vec};"
    if marker in text:
        text = text.split(marker)[0].rstrip() + "\n"

    if "pub type DataKeyCore" not in text:
        text = text.replace(
            "impl IntoVal<Env, Val> for DataKey {",
            INSERT_BEFORE_INTOVAL + "impl IntoVal<Env, Val> for DataKey {",
            1,
        )

    if "AccessControlEnabled" not in text:
        text = text.replace(
            "    /// Dispute window length in ledgers. 0 = no dispute window.\n    DisputeLedgers,",
            EXTRA_VARIANTS + "    /// Dispute window length in ledgers. 0 = no dispute window.\n    DisputeLedgers,",
            1,
        )

    if "DataKey::AccessControlEnabled" not in text:
        text = text.replace(
            "            DataKey::DisputeLedgers => S::new(env, \"DisputeLedgers\").into_val(env),",
            EXTRA_MATCH_ARMS
            + "            DataKey::DisputeLedgers => S::new(env, \"DisputeLedgers\").into_val(env),",
            1,
        )

    TYPES.write_text(text, encoding="utf-8")
    print(f"Repaired {TYPES} ({len(text.splitlines())} lines)")


if __name__ == "__main__":
    main()

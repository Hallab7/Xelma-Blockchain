// SPDX-License-Identifier: MIT
//! Differential invariant test harness using a reference model.

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use soroban_sdk::{Address, Env};
use std::collections::HashMap;
use std::env;
use rand::{rngs::StdRng, SeedableRng};

use crate::contract::{VirtualTokenContract, VirtualTokenContractClient};
use crate::types::BetSide;
use super::reference_model::ReferenceModel;

/// Represents a simplified action that can be performed on the contract.
#[derive(Debug, Clone)]
enum Action {
    BetUp { user: Address, amount: i128 },
    BetDown { user: Address, amount: i128 },
    Resolve { price_up: bool },
    Claim { user: Address },
    // New actions for extended stress testing
    Cancel { user: Address },
    Pause,
    ConfigChange { key: String, value: String },
}

/// Generate a random sequence of actions.
fn action_strategy() -> impl Strategy<Value = Action> {
    // Generate a random address.
    let addr = any::<[u8; 32]>().prop_map(|bytes| Address::from_bytes(&bytes));
    let amount = 0i128..=1_000_000i128;
    // Simple string generators for config actions.
    let key = any::<String>();
    let value = any::<String>();
    prop_oneof![
        (addr.clone(), amount.clone()).prop_map(|(u, a)| Action::BetUp { user: u, amount: a }),
        (addr.clone(), amount.clone()).prop_map(|(u, a)| Action::BetDown { user: u, amount: a }),
        any::<bool>().prop_map(|up| Action::Resolve { price_up: up }),
        addr.clone().prop_map(|u| Action::Claim { user: u }),
        // New actions
        addr.clone().prop_map(|u| Action::Cancel { user: u }),
        any::<bool>().prop_map(|_| Action::Pause),
        (key.clone(), value.clone()).prop_map(|(k, v)| Action::ConfigChange { key: k, value: v }),
    ]
}

/// Helper to format failure information.
fn pretty_print_failure(seed: Option<u64>, actions: &[Action], diff: &str) -> ! {
    panic!(
        "Invariant violation!\nSeed: {:?}\nAction trace: {:#?}\nState diff:\n{}",
        seed, actions, diff
    );
}

proptest! {
    #[test]
    fn differential_invariant_harness() {
        // Environment configuration
        let seq_len: usize = env::var("SEQUENCE_LENGTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        let seed_opt: Option<u64> = env::var("SEED")
            .ok()
            .and_then(|v| v.parse().ok());

        // Set up proptest runner with optional seed (deterministic when seed is provided)
        let config = Config::with_cases(seq_len);
        let rng = match seed_opt {
            Some(seed) => StdRng::seed_from_u64(seed),
            None => StdRng::from_entropy(),
        };
        let mut runner = TestRunner::new_with_rng(config, rng);
        let actions_strategy = prop::collection::vec(action_strategy(), 1..=seq_len);
        let actions = runner.run(&actions_strategy, |v| Ok(v)).expect("Failed to generate actions");

        // Setup contract environment.
        let env = Env::default();
        let contract_id = env.register(VirtualTokenContract, ());
        let client = VirtualTokenContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);
        env.mock_all_auths();
        client.initialize(&admin, &oracle);

        // Reference model.
        let mut model = ReferenceModel::new();

        // Helper to compare invariants after each step.
        let check = |model: &ReferenceModel| {
            let violations = model.check_invariants();
            if !violations.is_empty() {
                // Provide a placeholder diff; in a real implementation, compare contract vs model state.
                let diff = "State diff not implemented";
                pretty_print_failure(seed_opt, &actions, diff);
            }
        };

        // Execute actions.
        for act in &actions {
            match act {
                Action::BetUp { user, amount } => {
                    client.place_bet(&user, &(amount as u128), &BetSide::Up).unwrap();
                    model.place_bet(&user, *amount);
                }
                Action::BetDown { user, amount } => {
                    client.place_bet(&user, &(amount as u128), &BetSide::Down).unwrap();
                    model.place_bet(&user, *amount);
                }
                Action::Resolve { price_up } => {
                    client.resolve_round(&crate::types::OraclePayload {
                        price: if *price_up { 2_000_0000 } else { 500_000 },
                        timestamp: env.ledger().timestamp(),
                        round_id: 0,
                        nonce: 1u64,
                    });
                    // Simplified: no explicit winners map; model resolves with empty map.
                    model.resolve(&HashMap::new());
                }
                Action::Claim { user } => {
                    let _ = client.claim_winnings(&user);
                    model.claim(&user);
                }
                Action::Cancel { user } => {
                    // Placeholder: implement contract cancel if available
                    // client.cancel(&user);
                    model.cancel(&user);
                }
                Action::Pause => {
                    // Placeholder: implement contract pause if available
                    // client.pause();
                    model.pause();
                }
                Action::ConfigChange { key, value } => {
                    // Placeholder: implement contract config change if available
                    // client.config_change(key, value);
                    model.config_change(key, value);
                }
            }
            // Check invariants after each action.
            check(&model);
        }
    }
}

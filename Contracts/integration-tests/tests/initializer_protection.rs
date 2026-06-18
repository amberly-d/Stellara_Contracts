#![cfg(test)]

//! Integration tests verifying that initializer protection prevents
//! re-initialization across all upgradeable Stellara contracts.
//!
//! These tests ensure that:
//! 1. Contracts can be initialized exactly once
//! 2. A second initialization attempt is rejected
//! 3. The upgradeability module's guard functions work correctly
//!    when integrated with real contract workflows

extern crate std;

use shared::circuit_breaker::CircuitBreakerConfig;
use soroban_sdk::{
    testutils::Address as _,
    Address, Env, Vec,
};
use trading::UpgradeableTradingContract;
use messaging::UpgradeableMessagingContract;

/// Helper: create a standard CircuitBreakerConfig for tests
fn test_cb_config() -> CircuitBreakerConfig {
    CircuitBreakerConfig {
        max_volume_per_period: 10_000_000,
        max_tx_count_per_period: 100,
        period_duration: 3600,
    }
}

// ──────────────────────────────────────────────────────────────────────
// Trading Contract — Double Initialization Blocked
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_trading_contract_initializes_successfully() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, UpgradeableTradingContract);
    let client = trading::UpgradeableTradingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let approver = Address::generate(&env);
    let executor = Address::generate(&env);

    let mut approvers = Vec::new(&env);
    approvers.push_back(approver);

    // First init should succeed without panicking
    client.init(&admin, &approvers, &executor, &test_cb_config());
}

#[test]
fn test_trading_double_initialization_blocked() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, UpgradeableTradingContract);
    let client = trading::UpgradeableTradingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let approver = Address::generate(&env);
    let executor = Address::generate(&env);

    let mut approvers = Vec::new(&env);
    approvers.push_back(approver);

    // First init — should succeed
    client.init(&admin, &approvers, &executor, &test_cb_config());

    // Second init — must fail (returns Unauthorized error)
    let result = client.try_init(&admin, &approvers, &executor, &test_cb_config());
    assert!(result.is_err(), "Trading contract must reject re-initialization");
}

// ──────────────────────────────────────────────────────────────────────
// Messaging Contract — Double Initialization Blocked
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_messaging_contract_initializes_successfully() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, UpgradeableMessagingContract);
    let client = messaging::UpgradeableMessagingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let approver = Address::generate(&env);
    let executor = Address::generate(&env);

    let mut approvers = Vec::new(&env);
    approvers.push_back(approver);

    // First init should succeed
    client.init(&admin, &approvers, &executor, &test_cb_config());
}

#[test]
fn test_messaging_double_initialization_blocked() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, UpgradeableMessagingContract);
    let client = messaging::UpgradeableMessagingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let approver = Address::generate(&env);
    let executor = Address::generate(&env);

    let mut approvers = Vec::new(&env);
    approvers.push_back(approver);

    // First init — should succeed
    client.init(&admin, &approvers, &executor, &test_cb_config());

    // Second init — must fail
    let result = client.try_init(&admin, &approvers, &executor, &test_cb_config());
    assert!(result.is_err(), "Messaging contract must reject re-initialization");
}

// Note: Direct tests for the upgradeability module's guard functions
// (initializer_guard, is_initialized, mark_initialized, etc.) live in
// the upgradeability crate itself (`cargo test -p upgradeability`).
// Soroban storage requires a contract context, so those functions cannot
// be tested directly from integration tests without a registered contract.


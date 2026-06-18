#![no_std]
//! # Upgradeability Module
//!
//! Provides reusable initializer protection for Stellara smart contracts
//! on Soroban/Stellar. This is the Soroban equivalent of OpenZeppelin's
//! `Initializable` pattern.
//!
//! ## Problem
//!
//! Upgradeable contracts use `initialize()` instead of constructors.
//! Without protection, `initialize()` can be called multiple times,
//! allowing attackers to reset admin roles, governance settings, or
//! contract state after deployment.
//!
//! ## Solution
//!
//! This module provides two guard functions that use persistent storage
//! to track initialization state:
//!
//! - `ensure_not_initialized(env)` — panics if the contract was already initialized
//! - `mark_initialized(env)` — records that initialization has occurred
//!
//! ## Usage
//!
//! ```rust,ignore
//! use upgradeability::{ensure_not_initialized, mark_initialized};
//!
//! pub fn initialize(env: Env, admin: Address) {
//!     // Guard: prevent re-initialization
//!     ensure_not_initialized(&env);
//!     mark_initialized(&env);
//!
//!     // ... rest of initialization logic
//! }
//! ```

use soroban_sdk::{symbol_short, Env, Symbol};

/// Storage key used to track whether a contract has been initialized.
/// Uses `symbol_short!` for gas-efficient persistent storage access.
const INIT_KEY: Symbol = symbol_short!("init");

/// Checks whether the contract has already been initialized.
///
/// # Returns
/// `true` if the contract has been initialized, `false` otherwise.
pub fn is_initialized(env: &Env) -> bool {
    env.storage().persistent().has(&INIT_KEY)
}

/// Panics if the contract has already been initialized.
///
/// This function should be called at the very beginning of any `initialize()`
/// or `init()` function to prevent re-initialization attacks.
///
/// # Panics
/// Panics with `"Already initialized"` if the contract was previously initialized.
pub fn ensure_not_initialized(env: &Env) {
    if env.storage().persistent().has(&INIT_KEY) {
        panic!("Already initialized");
    }
}

/// Marks the contract as initialized by writing to persistent storage.
///
/// This function should be called immediately after `ensure_not_initialized()`
/// to atomically protect against re-initialization.
///
/// # Storage
/// Sets the `"init"` key in persistent storage to `true`.
pub fn mark_initialized(env: &Env) {
    env.storage().persistent().set(&INIT_KEY, &true);
}

/// Combined guard: ensures the contract is not yet initialized, then marks it.
///
/// This is a convenience function that combines `ensure_not_initialized` and
/// `mark_initialized` into a single call for simpler usage.
///
/// # Panics
/// Panics with `"Already initialized"` if the contract was previously initialized.
///
/// # Example
/// ```rust,ignore
/// pub fn initialize(env: Env, admin: Address) {
///     upgradeability::initializer_guard(&env);
///     // ... setup logic
/// }
/// ```
pub fn initializer_guard(env: &Env) {
    ensure_not_initialized(env);
    mark_initialized(env);
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::{contract, contractimpl, Env};

    // A minimal test contract that delegates to the upgradeability module
    #[contract]
    pub struct TestInitContract;

    #[contractimpl]
    impl TestInitContract {
        pub fn do_init(env: Env) {
            initializer_guard(&env);
        }

        pub fn check_initialized(env: Env) -> bool {
            is_initialized(&env)
        }

        pub fn do_mark(env: Env) {
            mark_initialized(&env);
        }

        pub fn do_ensure(env: Env) {
            ensure_not_initialized(&env);
        }
    }

    #[test]
    fn test_fresh_contract_is_not_initialized() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TestInitContract);
        let client = TestInitContractClient::new(&env, &contract_id);
        assert!(!client.check_initialized());
    }

    #[test]
    fn test_mark_initialized_sets_flag() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TestInitContract);
        let client = TestInitContractClient::new(&env, &contract_id);
        assert!(!client.check_initialized());
        client.do_mark();
        assert!(client.check_initialized());
    }

    #[test]
    fn test_initializer_guard_succeeds_on_first_call() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TestInitContract);
        let client = TestInitContractClient::new(&env, &contract_id);
        client.do_init(); // should not panic
        assert!(client.check_initialized());
    }

    // Note: Tests for double-init rejection (panic path) are covered by
    // the integration tests in `integration-tests/tests/initializer_protection.rs`
    // because Soroban cdylib panics cause process abort rather than unwinding,
    // which cannot be caught by the unit test harness.

    #[test]
    fn test_is_initialized_returns_false_before_mark() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TestInitContract);
        let client = TestInitContractClient::new(&env, &contract_id);
        assert!(!client.check_initialized());
    }

    #[test]
    fn test_is_initialized_returns_true_after_mark() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TestInitContract);
        let client = TestInitContractClient::new(&env, &contract_id);
        client.do_mark();
        assert!(client.check_initialized());
    }
}


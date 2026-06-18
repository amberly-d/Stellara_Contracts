//! End-to-end, scenario-based governance tests for complex proposal flows.
//!
//! Unlike the unit tests in `shared/tests/governance.rs` (which drive
//! `GovernanceManager` directly), these tests exercise the *full contract path*
//! through real upgradeable contract clients — `require_auth`, role wiring done
//! in `init`, error mapping, and timelock progression on the ledger.
//!
//! Coverage focus (the fringe cases called out in the governance hardening issue):
//! * full multi-sig upgrade lifecycle (propose -> approve -> timelock -> execute),
//! * quorum edge cases (threshold not met, exact threshold, single/unanimous),
//! * timelock boundaries (one second early vs. exact expiry),
//! * reject / cancel paths blocking execution,
//! * unauthorized approvers and duplicate / post-quorum approvals,
//! * simulated upgrades across two independent contracts sharing the module,
//! * a bounded fuzz sweep over (approver count, threshold) combinations.

#![cfg(test)]

extern crate std;

use messaging::UpgradeableMessagingContract;
use shared::circuit_breaker::CircuitBreakerConfig;
use shared::governance::ProposalStatus;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    Address, Env, Vec,
};
use trading::{UpgradeableTradingContract, UpgradeableTradingContractClient};

const BASE_TS: u64 = 1_000;

fn cb_config() -> CircuitBreakerConfig {
    CircuitBreakerConfig {
        max_volume_per_period: 1_000_000_000,
        max_tx_count_per_period: 100,
        period_duration: 3600,
    }
}

/// Spin up a trading contract initialized with `n` approvers and an executor.
/// Returns the client plus the governance actors.
fn setup_trading(
    env: &Env,
    n: u32,
) -> (
    UpgradeableTradingContractClient<'_>,
    Address,
    Vec<Address>,
    Address,
) {
    let contract_id = env.register_contract(None, UpgradeableTradingContract);
    let client = UpgradeableTradingContractClient::new(env, &contract_id);

    let admin = Address::generate(env);
    let executor = Address::generate(env);
    let mut approvers = Vec::new(env);
    for _ in 0..n {
        approvers.push_back(Address::generate(env));
    }

    client.init(&admin, &approvers, &executor, &cb_config());
    (client, admin, approvers, executor)
}

// ----------------------------------------------------------------------------
// Full lifecycle
// ----------------------------------------------------------------------------

#[test]
fn full_2of3_lifecycle_executes_after_timelock() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, executor) = setup_trading(&env, 3);
    let timelock = 14_400u64;

    let pid = client.propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &approvers,
        &2u32,
        &timelock,
    );

    // First approval -> still pending.
    client.approve_upgrade(&pid, &approvers.get(0).unwrap());
    assert_eq!(
        client.get_upgrade_proposal(&pid).status,
        ProposalStatus::Pending
    );

    // Second approval -> quorum reached.
    client.approve_upgrade(&pid, &approvers.get(1).unwrap());
    assert_eq!(
        client.get_upgrade_proposal(&pid).status,
        ProposalStatus::Approved
    );

    // Timelock not yet elapsed -> execution blocked.
    assert!(client.try_execute_upgrade(&pid, &executor).is_err());

    // Advance past the timelock and execute.
    env.ledger().with_mut(|li| li.timestamp = BASE_TS + timelock);
    client.execute_upgrade(&pid, &executor);

    let prop = client.get_upgrade_proposal(&pid);
    assert_eq!(prop.status, ProposalStatus::Executed);
    assert!(prop.executed);
}

// ----------------------------------------------------------------------------
// Quorum edge cases
// ----------------------------------------------------------------------------

#[test]
fn quorum_not_met_blocks_execution() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, executor) = setup_trading(&env, 3);

    let pid = client.propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &approvers,
        &3u32, // unanimous required
        &0u64,
    );

    // Only two of three approve.
    client.approve_upgrade(&pid, &approvers.get(0).unwrap());
    client.approve_upgrade(&pid, &approvers.get(1).unwrap());

    assert_eq!(
        client.get_upgrade_proposal(&pid).status,
        ProposalStatus::Pending
    );
    assert!(client.try_execute_upgrade(&pid, &executor).is_err());
}

#[test]
fn invalid_threshold_zero_is_rejected_at_proposal_time() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, _executor) = setup_trading(&env, 2);

    let res = client.try_propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &approvers,
        &0u32,
        &0u64,
    );
    assert!(res.is_err());
}

#[test]
fn threshold_above_approver_count_is_rejected() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, _executor) = setup_trading(&env, 2);

    let res = client.try_propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &approvers,
        &3u32, // > 2 approvers
        &0u64,
    );
    assert!(res.is_err());
}

#[test]
fn approver_excluded_from_proposal_subset_cannot_approve() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    // The contract is initialized with two approvers (both hold the Approver
    // role), but the proposal restricts the approver set to just the first one.
    let (client, admin, approvers, _executor) = setup_trading(&env, 2);
    let included = approvers.get(0).unwrap();
    let excluded = approvers.get(1).unwrap();

    let mut subset = Vec::new(&env);
    subset.push_back(included.clone());

    let pid = client.propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &subset,
        &1u32,
        &0u64,
    );

    // `excluded` has the Approver role but is not on this proposal's list, so the
    // call is cleanly rejected (recoverable error) without changing the count.
    assert!(client.try_approve_upgrade(&pid, &excluded).is_err());
    assert_eq!(
        client.get_upgrade_proposal(&pid).approvals_count,
        0,
        "rejected approval must not change the count"
    );
}

#[test]
fn duplicate_and_post_quorum_approvals_are_rejected() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, _executor) = setup_trading(&env, 2);

    let pid = client.propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &approvers,
        &1u32,
        &0u64,
    );

    client.approve_upgrade(&pid, &approvers.get(0).unwrap());
    // Quorum already reached -> the first approver's repeat and the second
    // approver's late vote are both rejected.
    assert!(client.try_approve_upgrade(&pid, &approvers.get(0).unwrap()).is_err());
    assert!(client.try_approve_upgrade(&pid, &approvers.get(1).unwrap()).is_err());
    assert_eq!(client.get_upgrade_proposal(&pid).approvals_count, 1);
}

// ----------------------------------------------------------------------------
// Timelock boundary
// ----------------------------------------------------------------------------

#[test]
fn timelock_one_second_early_fails_exact_expiry_succeeds() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, executor) = setup_trading(&env, 1);
    let timelock = 3_600u64;

    let pid = client.propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &approvers,
        &1u32,
        &timelock,
    );
    client.approve_upgrade(&pid, &approvers.get(0).unwrap());

    // One second before expiry -> blocked.
    env.ledger().with_mut(|li| li.timestamp = BASE_TS + timelock - 1);
    assert!(client.try_execute_upgrade(&pid, &executor).is_err());

    // Exactly at expiry -> allowed.
    env.ledger().with_mut(|li| li.timestamp = BASE_TS + timelock);
    client.execute_upgrade(&pid, &executor);
    assert_eq!(
        client.get_upgrade_proposal(&pid).status,
        ProposalStatus::Executed
    );
}

// ----------------------------------------------------------------------------
// Reject / cancel paths
// ----------------------------------------------------------------------------

#[test]
fn rejected_proposal_cannot_be_executed() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, executor) = setup_trading(&env, 2);

    let pid = client.propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &approvers,
        &1u32,
        &0u64,
    );

    client.reject_upgrade(&pid, &approvers.get(0).unwrap());
    assert_eq!(
        client.get_upgrade_proposal(&pid).status,
        ProposalStatus::Rejected
    );
    // A rejected proposal can no longer be approved or executed.
    assert!(client.try_approve_upgrade(&pid, &approvers.get(1).unwrap()).is_err());
    assert!(client.try_execute_upgrade(&pid, &executor).is_err());
}

#[test]
fn cancelled_proposal_cannot_be_executed() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, executor) = setup_trading(&env, 1);

    let pid = client.propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &approvers,
        &1u32,
        &0u64,
    );
    client.approve_upgrade(&pid, &approvers.get(0).unwrap());

    // Admin cancels even though quorum was reached.
    client.cancel_upgrade(&pid, &admin);
    assert_eq!(
        client.get_upgrade_proposal(&pid).status,
        ProposalStatus::Cancelled
    );
    assert!(client.try_execute_upgrade(&pid, &executor).is_err());
}

#[test]
fn cannot_execute_twice() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, executor) = setup_trading(&env, 1);

    let pid = client.propose_upgrade(
        &admin,
        &symbol_short!("v2hash"),
        &symbol_short!("Upgrade"),
        &approvers,
        &1u32,
        &0u64,
    );
    client.approve_upgrade(&pid, &approvers.get(0).unwrap());
    client.execute_upgrade(&pid, &executor);
    assert!(client.try_execute_upgrade(&pid, &executor).is_err());
}

// ----------------------------------------------------------------------------
// Concurrency / multi-proposal & cross-contract simulation
// ----------------------------------------------------------------------------

#[test]
fn concurrent_proposals_have_independent_state() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    let (client, admin, approvers, _executor) = setup_trading(&env, 2);

    let p1 = client.propose_upgrade(
        &admin,
        &symbol_short!("h1"),
        &symbol_short!("d1"),
        &approvers,
        &1u32,
        &0u64,
    );
    let p2 = client.propose_upgrade(
        &admin,
        &symbol_short!("h2"),
        &symbol_short!("d2"),
        &approvers,
        &2u32,
        &0u64,
    );

    assert_eq!(p1, 1);
    assert_eq!(p2, 2);

    // Approving p1 to quorum must not affect p2.
    client.approve_upgrade(&p1, &approvers.get(0).unwrap());
    assert_eq!(
        client.get_upgrade_proposal(&p1).status,
        ProposalStatus::Approved
    );
    assert_eq!(
        client.get_upgrade_proposal(&p2).status,
        ProposalStatus::Pending
    );
}

/// Simulate a coordinated governance upgrade across two independent contracts
/// that share the governance module. Validates that proposal bookkeeping and the
/// timelock are tracked independently per contract.
///
/// Note: in the local test environment `execute_upgrade` records the proposal as
/// executed (governance bookkeeping); the on-chain WASM swap itself is a
/// ledger-level operation outside the scope of these unit-style tests.
#[test]
fn simulate_upgrade_across_two_contracts() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.mock_all_auths();

    // Trading contract.
    let (trading, t_admin, t_approvers, t_executor) = setup_trading(&env, 2);

    // Messaging contract sharing the same governance actors set.
    let messaging_id = env.register_contract(None, UpgradeableMessagingContract);
    let messaging = messaging::UpgradeableMessagingContractClient::new(&env, &messaging_id);
    let m_admin = Address::generate(&env);
    let m_executor = Address::generate(&env);
    let mut m_approvers = Vec::new(&env);
    m_approvers.push_back(Address::generate(&env));
    m_approvers.push_back(Address::generate(&env));
    messaging.init(&m_admin, &m_approvers, &m_executor, &cb_config());

    let timelock = 7_200u64;

    let t_pid = trading.propose_upgrade(
        &t_admin,
        &symbol_short!("tv2"),
        &symbol_short!("UpgrTrade"),
        &t_approvers,
        &2u32,
        &timelock,
    );
    let m_pid = messaging.propose_upgrade(
        &m_admin,
        &symbol_short!("mv2"),
        &symbol_short!("UpgrMsg"),
        &m_approvers,
        &2u32,
        &timelock,
    );

    trading.approve_upgrade(&t_pid, &t_approvers.get(0).unwrap());
    trading.approve_upgrade(&t_pid, &t_approvers.get(1).unwrap());
    messaging.approve_upgrade(&m_pid, &m_approvers.get(0).unwrap());
    messaging.approve_upgrade(&m_pid, &m_approvers.get(1).unwrap());

    assert_eq!(
        trading.get_upgrade_proposal(&t_pid).status,
        ProposalStatus::Approved
    );
    assert_eq!(
        messaging.get_upgrade_proposal(&m_pid).status,
        ProposalStatus::Approved
    );

    env.ledger().with_mut(|li| li.timestamp = BASE_TS + timelock);
    trading.execute_upgrade(&t_pid, &t_executor);
    messaging.execute_upgrade(&m_pid, &m_executor);

    assert_eq!(
        trading.get_upgrade_proposal(&t_pid).status,
        ProposalStatus::Executed
    );
    assert_eq!(
        messaging.get_upgrade_proposal(&m_pid).status,
        ProposalStatus::Executed
    );
}

// ----------------------------------------------------------------------------
// Bounded fuzz sweep through the real contract path
// ----------------------------------------------------------------------------

/// Bounded fuzz: for each (approver count, threshold) combination, a fresh
/// contract is initialized and the proposal is approved one vote at a time. The
/// proposal must reach `Approved` exactly when the threshold is hit and become
/// executable only after the timelock elapses.
#[test]
fn fuzz_quorum_and_execution_through_contract() {
    let env = Env::default();
    env.mock_all_auths();

    for n in 1u32..=4 {
        for threshold in 1u32..=n {
            env.ledger().with_mut(|li| li.timestamp = BASE_TS);
            let (client, admin, approvers, executor) = setup_trading(&env, n);
            let timelock = 100u64;

            let pid = client.propose_upgrade(
                &admin,
                &symbol_short!("vh"),
                &symbol_short!("desc"),
                &approvers,
                &threshold,
                &timelock,
            );

            for i in 0..threshold {
                client.approve_upgrade(&pid, &approvers.get(i).unwrap());
            }

            assert_eq!(
                client.get_upgrade_proposal(&pid).status,
                ProposalStatus::Approved,
                "n={n} threshold={threshold} should be Approved at quorum"
            );

            // Still timelocked.
            assert!(
                client.try_execute_upgrade(&pid, &executor).is_err(),
                "n={n} threshold={threshold} should be timelocked"
            );

            env.ledger().with_mut(|li| li.timestamp = BASE_TS + timelock);
            client.execute_upgrade(&pid, &executor);
            assert_eq!(
                client.get_upgrade_proposal(&pid).status,
                ProposalStatus::Executed,
                "n={n} threshold={threshold} should execute after timelock"
            );
        }
    }
}

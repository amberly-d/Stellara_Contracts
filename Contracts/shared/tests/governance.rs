//! Edge-case, scenario, and bounded-fuzz tests for the shared governance module.
//!
//! These tests exercise [`shared::governance::GovernanceManager`] *directly* (i.e.
//! the reusable building block every upgradeable contract depends on) rather than
//! going through a single contract client. They focus on the fringe behaviours the
//! happy-path contract tests do not cover:
//!
//! * quorum / threshold boundaries (single, unanimous, impossible),
//! * timelock boundaries (zero delay, one-second-before, exact expiry),
//! * proposal state-machine transitions (reject/cancel/execute ordering),
//! * role-based access control panics,
//! * documented fringe behaviours that are intentionally captured here so any
//!   change in behaviour is caught by CI (see `GOVERNANCE_TESTING.md`).
//!
//! Where a behaviour is a *known limitation* the test name and comment say so.

extern crate std;

use shared::governance::{GovernanceError, GovernanceManager, GovernanceRole, ProposalStatus};
use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, Ledger},
    Address, Env, Map, Symbol, Vec,
};

/// Minimal host contract used only to provide a contract-storage context so the
/// governance helpers (which read/write persistent storage) can run under
/// `env.as_contract`.
#[contract]
pub struct GovHost;

#[contractimpl]
impl GovHost {}

const ROLES_KEY: Symbol = symbol_short!("roles");
const BASE_TS: u64 = 1_000_000;

/// Register the host contract and pin a deterministic ledger timestamp.
fn setup(env: &Env) -> Address {
    env.ledger().with_mut(|li| li.timestamp = BASE_TS);
    env.register_contract(None, GovHost)
}

/// Write the role map exactly the way the production contracts do during `init`.
/// Must be called inside an `env.as_contract` closure.
fn set_roles(env: &Env, admin: &Address, approvers: &Vec<Address>, executor: &Address) {
    let mut roles: Map<Address, GovernanceRole> = Map::new(env);
    roles.set(admin.clone(), GovernanceRole::Admin);
    for approver in approvers.iter() {
        roles.set(approver, GovernanceRole::Approver);
    }
    roles.set(executor.clone(), GovernanceRole::Executor);
    env.storage().persistent().set(&ROLES_KEY, &roles);
}

/// Build a `Vec<Address>` of freshly generated approver addresses.
fn make_approvers(env: &Env, n: u32) -> Vec<Address> {
    let mut approvers = Vec::new(env);
    for _ in 0..n {
        approvers.push_back(Address::generate(env));
    }
    approvers
}

// ----------------------------------------------------------------------------
// Quorum / threshold edge cases
// ----------------------------------------------------------------------------

/// Bounded fuzz: for every (n approvers, threshold in 1..=n) combination the
/// proposal must flip to `Approved` *exactly* when the running approval count
/// reaches the threshold — never before.
#[test]
fn fuzz_quorum_flips_exactly_at_threshold() {
    let env = Env::default();
    let id = setup(&env);

    for n in 1u32..=6 {
        for threshold in 1u32..=n {
            let admin = Address::generate(&env);
            let executor = Address::generate(&env);
            let approvers = make_approvers(&env, n);

            env.as_contract(&id, || {
                set_roles(&env, &admin, &approvers, &executor);

                let pid = GovernanceManager::propose_upgrade(
                    &env,
                    admin.clone(),
                    symbol_short!("hash"),
                    id.clone(),
                    symbol_short!("desc"),
                    threshold,
                    approvers.clone(),
                    100,
                )
                .unwrap();

                for i in 0..n {
                    let approver = approvers.get(i).unwrap();
                    GovernanceManager::approve_proposal(&env, pid, approver).unwrap();

                    let approved_so_far = i + 1;
                    let prop = GovernanceManager::get_proposal(&env, pid).unwrap();

                    if approved_so_far >= threshold {
                        assert_eq!(
                            prop.status,
                            ProposalStatus::Approved,
                            "n={n} threshold={threshold} count={approved_so_far} should be Approved"
                        );
                        // Once approved the proposal is no longer Pending, so any
                        // further approval is rejected; stop here.
                        break;
                    } else {
                        assert_eq!(
                            prop.status,
                            ProposalStatus::Pending,
                            "n={n} threshold={threshold} count={approved_so_far} should still be Pending"
                        );
                    }
                }
            });
        }
    }
}

#[test]
fn single_approver_single_threshold_approves_immediately() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            100,
        )
        .unwrap();
        GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        assert_eq!(
            GovernanceManager::get_proposal(&env, pid).unwrap().status,
            ProposalStatus::Approved
        );
    });
}

#[test]
fn unanimous_threshold_requires_every_approver() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 3);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            3, // unanimous
            approvers.clone(),
            100,
        )
        .unwrap();

        GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        GovernanceManager::approve_proposal(&env, pid, approvers.get(1).unwrap()).unwrap();
        assert_eq!(
            GovernanceManager::get_proposal(&env, pid).unwrap().status,
            ProposalStatus::Pending
        );

        GovernanceManager::approve_proposal(&env, pid, approvers.get(2).unwrap()).unwrap();
        assert_eq!(
            GovernanceManager::get_proposal(&env, pid).unwrap().status,
            ProposalStatus::Approved
        );
    });
}

#[test]
fn threshold_zero_is_rejected() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let res = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            0,
            approvers.clone(),
            100,
        );
        assert_eq!(res, Err(GovernanceError::InvalidThreshold));
    });
}

#[test]
fn threshold_greater_than_approvers_is_rejected() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let res = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            3, // > 2 approvers
            approvers.clone(),
            100,
        );
        assert_eq!(res, Err(GovernanceError::InvalidThreshold));
    });
}

#[test]
fn empty_approver_set_cannot_form_a_quorum() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 0);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        // threshold 1 against 0 approvers is impossible -> InvalidThreshold.
        let res = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            100,
        );
        assert_eq!(res, Err(GovernanceError::InvalidThreshold));
    });
}

#[test]
fn approver_not_in_proposal_list_is_unauthorized() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);
    // An address that holds the Approver role globally but is NOT on this
    // proposal's approver list.
    let outsider = Address::generate(&env);

    env.as_contract(&id, || {
        let mut roles: Map<Address, GovernanceRole> = Map::new(&env);
        roles.set(admin.clone(), GovernanceRole::Admin);
        for a in approvers.iter() {
            roles.set(a, GovernanceRole::Approver);
        }
        roles.set(outsider.clone(), GovernanceRole::Approver);
        roles.set(executor.clone(), GovernanceRole::Executor);
        env.storage().persistent().set(&ROLES_KEY, &roles);

        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            100,
        )
        .unwrap();

        let res = GovernanceManager::approve_proposal(&env, pid, outsider.clone());
        assert_eq!(res, Err(GovernanceError::Unauthorized));
    });
}

#[test]
fn duplicate_approval_is_rejected() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 3);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            2,
            approvers.clone(),
            100,
        )
        .unwrap();

        GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        let res = GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap());
        assert_eq!(res, Err(GovernanceError::DuplicateApproval));
        // The rejected duplicate must not have inflated the count.
        assert_eq!(
            GovernanceManager::get_proposal(&env, pid).unwrap().approvals_count,
            1
        );
    });
}

#[test]
fn approval_after_quorum_reached_is_rejected() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            100,
        )
        .unwrap();

        GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        // Already Approved -> further approvals are invalid.
        let res = GovernanceManager::approve_proposal(&env, pid, approvers.get(1).unwrap());
        assert_eq!(res, Err(GovernanceError::InvalidProposal));
    });
}

// ----------------------------------------------------------------------------
// Timelock edge cases
// ----------------------------------------------------------------------------

/// Bounded fuzz over a spread of timelock delays. For every delay the proposal
/// must be executable at `created_at + delay` and (for non-zero delays) NOT one
/// second earlier.
#[test]
fn fuzz_timelock_boundary_enforced() {
    let env = Env::default();
    let id = setup(&env);

    for delay in [0u64, 1, 100, 3_600, 86_400, 14_400] {
        let admin = Address::generate(&env);
        let executor = Address::generate(&env);
        let approvers = make_approvers(&env, 1);

        env.ledger().with_mut(|li| li.timestamp = BASE_TS);

        env.as_contract(&id, || {
            set_roles(&env, &admin, &approvers, &executor);
            let pid = GovernanceManager::propose_upgrade(
                &env,
                admin.clone(),
                symbol_short!("hash"),
                id.clone(),
                symbol_short!("desc"),
                1,
                approvers.clone(),
                delay,
            )
            .unwrap();
            GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();

            let prop = GovernanceManager::get_proposal(&env, pid).unwrap();
            assert_eq!(prop.execution_time, BASE_TS + delay);

            if delay > 0 {
                // One second before expiry must fail.
                env.ledger().with_mut(|li| li.timestamp = BASE_TS + delay - 1);
                let early = GovernanceManager::execute_proposal(&env, pid, executor.clone());
                assert_eq!(early, Err(GovernanceError::TimelockNotExpired));
            }

            // Exactly at expiry must succeed.
            env.ledger().with_mut(|li| li.timestamp = BASE_TS + delay);
            GovernanceManager::execute_proposal(&env, pid, executor.clone()).unwrap();
            assert_eq!(
                GovernanceManager::get_proposal(&env, pid).unwrap().status,
                ProposalStatus::Executed
            );
        });
    }
}

/// KNOWN BEHAVIOUR: a zero timelock means an approved proposal can be executed in
/// the same ledger it was approved. Captured so the safety implication is visible
/// (see `GOVERNANCE_TESTING.md` — callers should enforce a minimum delay).
#[test]
fn zero_timelock_allows_immediate_execution() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            0,
        )
        .unwrap();
        GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        // No time advance at all.
        GovernanceManager::execute_proposal(&env, pid, executor.clone()).unwrap();
        assert_eq!(
            GovernanceManager::get_proposal(&env, pid).unwrap().status,
            ProposalStatus::Executed
        );
    });
}

/// KNOWN LIMITATION: `created_at + timelock_delay` is unchecked arithmetic, so an
/// absurd delay overflows `u64` and panics (in debug) / wraps (in release).
/// Callers must bound `timelock_delay`. This test pins the debug-mode panic.
#[test]
#[should_panic]
fn overflowing_timelock_delay_panics() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let _ = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            u64::MAX,
        );
    });
}

// ----------------------------------------------------------------------------
// Proposal state-machine transitions
// ----------------------------------------------------------------------------

#[test]
fn cannot_execute_before_approval() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            2,
            approvers.clone(),
            0,
        )
        .unwrap();
        // Only one of two approvals -> still Pending.
        GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        let res = GovernanceManager::execute_proposal(&env, pid, executor.clone());
        assert_eq!(res, Err(GovernanceError::ProposalNotApproved));
    });
}

#[test]
fn cannot_execute_twice() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            0,
        )
        .unwrap();
        GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        GovernanceManager::execute_proposal(&env, pid, executor.clone()).unwrap();
        let res = GovernanceManager::execute_proposal(&env, pid, executor.clone());
        assert_eq!(res, Err(GovernanceError::ProposalNotApproved));
    });
}

#[test]
fn reject_blocks_subsequent_approval_and_execution() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            0,
        )
        .unwrap();

        GovernanceManager::reject_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        assert_eq!(
            GovernanceManager::get_proposal(&env, pid).unwrap().status,
            ProposalStatus::Rejected
        );

        // A rejected proposal can no longer be approved...
        let approve = GovernanceManager::approve_proposal(&env, pid, approvers.get(1).unwrap());
        assert_eq!(approve, Err(GovernanceError::InvalidProposal));
        // ...and double rejection is also invalid.
        let reject_again = GovernanceManager::reject_proposal(&env, pid, approvers.get(1).unwrap());
        assert_eq!(reject_again, Err(GovernanceError::InvalidProposal));
    });
}

#[test]
fn cancel_blocks_execution() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            0,
        )
        .unwrap();
        GovernanceManager::cancel_proposal(&env, pid, admin.clone()).unwrap();
        assert_eq!(
            GovernanceManager::get_proposal(&env, pid).unwrap().status,
            ProposalStatus::Cancelled
        );
        // A cancelled proposal is not Approved, so execution fails.
        let res = GovernanceManager::execute_proposal(&env, pid, executor.clone());
        assert_eq!(res, Err(GovernanceError::ProposalNotApproved));
    });
}

#[test]
fn cannot_cancel_after_execution() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            0,
        )
        .unwrap();
        GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        GovernanceManager::execute_proposal(&env, pid, executor.clone()).unwrap();

        let res = GovernanceManager::cancel_proposal(&env, pid, admin.clone());
        assert_eq!(res, Err(GovernanceError::InvalidProposal));
    });
}

/// KNOWN BEHAVIOUR: `cancel_proposal` only guards against the `executed` flag, so
/// an already-rejected proposal can still be transitioned to `Cancelled`. Pinned
/// here so the loose terminal-state handling is visible (see docs).
#[test]
fn cancel_overwrites_a_rejected_proposal() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            0,
        )
        .unwrap();
        GovernanceManager::reject_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        // Currently allowed: Rejected -> Cancelled.
        GovernanceManager::cancel_proposal(&env, pid, admin.clone()).unwrap();
        assert_eq!(
            GovernanceManager::get_proposal(&env, pid).unwrap().status,
            ProposalStatus::Cancelled
        );
    });
}

#[test]
fn get_unknown_proposal_returns_not_found() {
    let env = Env::default();
    let id = setup(&env);

    env.as_contract(&id, || {
        // No proposals map exists yet.
        assert_eq!(
            GovernanceManager::get_proposal(&env, 1),
            Err(GovernanceError::ProposalNotFound)
        );
    });
}

#[test]
fn proposal_ids_increment_independently() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let first = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("h1"),
            id.clone(),
            symbol_short!("d1"),
            1,
            approvers.clone(),
            100,
        )
        .unwrap();
        let second = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("h2"),
            id.clone(),
            symbol_short!("d2"),
            1,
            approvers.clone(),
            100,
        )
        .unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, 2);
    });
}

// ----------------------------------------------------------------------------
// Role-based access control (panics)
// ----------------------------------------------------------------------------

#[test]
#[should_panic(expected = "UNAUTH")]
fn non_admin_cannot_propose() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);
    let approver = approvers.get(0).unwrap();

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        // An Approver (role 1) is not allowed to propose (needs Admin role 0).
        let _ = GovernanceManager::propose_upgrade(
            &env,
            approver.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            100,
        );
    });
}

#[test]
#[should_panic(expected = "UNAUTH")]
fn unknown_address_cannot_approve() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);
    let stranger = Address::generate(&env);

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            100,
        )
        .unwrap();
        // Stranger has no role -> defaults to Executor (lowest) -> fails the
        // Approver requirement and panics.
        let _ = GovernanceManager::approve_proposal(&env, pid, stranger.clone());
    });
}

/// KNOWN LIMITATION: because an address with no assigned role defaults to the
/// lowest-privilege `Executor` role, the `execute` step is effectively
/// permissionless — any address can execute an already-approved, timelock-expired
/// proposal. Captured here and documented in `GOVERNANCE_TESTING.md`.
#[test]
fn execution_is_permissionless_for_any_address() {
    let env = Env::default();
    let id = setup(&env);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);
    let random_caller = Address::generate(&env); // not the designated executor

    env.as_contract(&id, || {
        set_roles(&env, &admin, &approvers, &executor);
        let pid = GovernanceManager::propose_upgrade(
            &env,
            admin.clone(),
            symbol_short!("hash"),
            id.clone(),
            symbol_short!("desc"),
            1,
            approvers.clone(),
            0,
        )
        .unwrap();
        GovernanceManager::approve_proposal(&env, pid, approvers.get(0).unwrap()).unwrap();
        // Anyone can execute under current behaviour.
        GovernanceManager::execute_proposal(&env, pid, random_caller.clone()).unwrap();
        assert_eq!(
            GovernanceManager::get_proposal(&env, pid).unwrap().status,
            ProposalStatus::Executed
        );
    });
}

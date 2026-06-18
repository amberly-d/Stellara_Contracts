# Governance Testing & Known Limitations

This document describes the test coverage for the shared governance module
(`shared/src/governance.rs`) that powers upgradeability across the Stellara
contracts, with a focus on the **complex proposal flows** — timelocks, edge
votes, and quorum edge cases — and documents **known limitations** with their
mitigations.

## Why this exists

The governance module is the trust layer for all contract upgrades (multi-sig
approvals + timelock). The happy-path tests inside each contract
(`contracts/trading/src/test.rs`) covered the nominal lifecycle, but the fringe
cases (impossible quorums, zero/boundary timelocks, out-of-order state
transitions, role edge cases) were untested — leaving room for unexpected
behaviour on edge cases. The suites below close that gap.

## Test inventory

### 1. Module-level unit & bounded-fuzz tests — `shared/tests/governance.rs`

Drives `GovernanceManager` directly (under `env.as_contract`) so the reusable
building block is verified in isolation.

| Area | Tests |
| --- | --- |
| Quorum / threshold | `fuzz_quorum_flips_exactly_at_threshold` (sweeps every `(n, threshold)` for `n ≤ 6`), `single_approver_single_threshold_approves_immediately`, `unanimous_threshold_requires_every_approver`, `threshold_zero_is_rejected`, `threshold_greater_than_approvers_is_rejected`, `empty_approver_set_cannot_form_a_quorum` |
| Edge votes | `approver_not_in_proposal_list_is_unauthorized`, `duplicate_approval_is_rejected`, `approval_after_quorum_reached_is_rejected` |
| Timelock | `fuzz_timelock_boundary_enforced` (sweeps `{0, 1, 100, 3600, 86400, 14400}`, checks one-second-early failure + exact-expiry success), `zero_timelock_allows_immediate_execution`, `overflowing_timelock_delay_panics` |
| State machine | `cannot_execute_before_approval`, `cannot_execute_twice`, `reject_blocks_subsequent_approval_and_execution`, `cancel_blocks_execution`, `cannot_cancel_after_execution`, `cancel_overwrites_a_rejected_proposal`, `get_unknown_proposal_returns_not_found`, `proposal_ids_increment_independently` |
| Role-based access | `non_admin_cannot_propose`, `unknown_address_cannot_approve`, `execution_is_permissionless_for_any_address` |

### 2. End-to-end scenario & fuzz tests — `integration-tests/tests/governance_flows.rs`

Drives the full contract path through real contract clients (`require_auth`,
role wiring via `init`, error mapping, ledger timelock progression).

| Area | Tests |
| --- | --- |
| Full lifecycle | `full_2of3_lifecycle_executes_after_timelock` |
| Quorum edge cases | `quorum_not_met_blocks_execution`, `invalid_threshold_zero_is_rejected_at_proposal_time`, `threshold_above_approver_count_is_rejected`, `approver_excluded_from_proposal_subset_cannot_approve`, `duplicate_and_post_quorum_approvals_are_rejected` |
| Timelock | `timelock_one_second_early_fails_exact_expiry_succeeds` |
| Reject / cancel | `rejected_proposal_cannot_be_executed`, `cancelled_proposal_cannot_be_executed`, `cannot_execute_twice` |
| Concurrency / upgrades | `concurrent_proposals_have_independent_state`, `simulate_upgrade_across_two_contracts`, `fuzz_quorum_and_execution_through_contract` (sweeps `(n, threshold)` for `n ≤ 4`) |

## How to run

```bash
cd Contracts

# Module-level governance unit + fuzz tests
cargo test -p shared --test governance

# End-to-end governance scenario + fuzz tests
cargo test -p integration-tests --test governance_flows

# Everything
cargo test --all
```

## Known limitations & mitigations

These behaviours are **intentionally captured by tests** so any future change is
caught by CI. They are documented rather than silently changed because altering
them affects on-chain semantics and is out of scope for a test-hardening change.

1. **Execution is permissionless.**
   An address with no assigned role defaults to the lowest-privilege `Executor`
   role, so `execute_proposal` accepts any caller for an already-approved,
   timelock-expired proposal (`execution_is_permissionless_for_any_address`).
   *Impact:* low — execution only finalizes a proposal that already cleared
   multi-sig approval and the timelock; it cannot bypass either gate.
   *Mitigation:* if a dedicated executor is required, assign an explicit
   `Executor`-or-higher role and tighten `require_role` to reject the implicit
   default, or gate `execute_upgrade` with `require_auth` on a known executor.

2. **Zero timelock allows same-ledger execution.**
   `propose_upgrade` accepts `timelock_delay == 0`, permitting execution in the
   same ledger as approval (`zero_timelock_allows_immediate_execution`).
   *Mitigation:* callers should pass a non-zero `timelock_delay`; consider
   enforcing a protocol-level minimum (e.g. 1 hour) at the contract entry point.

3. **`timelock_delay` is unchecked arithmetic.**
   `created_at + timelock_delay` can overflow `u64` for absurd values
   (`overflowing_timelock_delay_panics`).
   *Mitigation:* bound `timelock_delay` (e.g. ≤ 30 days) before calling
   `propose_upgrade`; a `checked_add` would harden this further.

4. **Loose terminal-state handling for `cancel`.**
   `cancel_proposal` only guards the `executed` flag, so a `Rejected` proposal
   can still be transitioned to `Cancelled` (`cancel_overwrites_a_rejected_proposal`).
   *Impact:* cosmetic — both are non-actionable terminal states; a cancelled or
   rejected proposal can never be approved or executed.
   *Mitigation:* if strict state hygiene is desired, reject `cancel` on any
   already-terminal (`Rejected`/`Cancelled`/`Executed`) proposal.

5. **WASM swap is environment-level.**
   In the local test environment, `execute_upgrade` records governance
   bookkeeping (status → `Executed`); the actual on-chain WASM replacement is a
   ledger operation outside the scope of these unit-style tests
   (`simulate_upgrade_across_two_contracts` validates the bookkeeping path).

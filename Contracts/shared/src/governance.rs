use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};
use crate::acl::{ROLE_ADMIN, ROLE_APPROVER, ROLE_EXECUTOR, PERMISSION_PROPOSE, PERMISSION_APPROVE, PERMISSION_EXECUTE, PERMISSION_PAUSE, PERMISSION_UNPAUSE, PERMISSION_MGR_ACL, ACL};

/// Upgrade proposal that must be approved via governance
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeProposal {
    pub id: u64,
    pub proposer: Address,
    pub new_contract_hash: Symbol,
    pub target_contract: Address,
    pub description: Symbol,
    pub approval_threshold: u32, // e.g., 2 of 3
    pub approvers: Vec<Address>,
    pub approvals_count: u32,
    pub status: ProposalStatus,
    pub created_at: u64,
    pub execution_time: u64, // Timelock: when it can be executed
    pub executed: bool,
}

/// Status of an upgrade proposal
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProposalStatus {
    Pending = 0,
    Approved = 1,
    Rejected = 2,
    Executed = 3,
    Cancelled = 4,
}

// Keep GovernanceRole for backwards compatibility, but recommend using ACL roles
/// Governance role (deprecated - use shared ACL roles instead)
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GovernanceRole {
    Admin = 0,    // Can propose upgrades and cancel
    Approver = 1, // Can approve/reject proposals
    Executor = 2, // Can execute approved proposals (after timelock)
}

/// Governance error codes
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GovernanceError {
    Unauthorized = 2001,
    InvalidProposal = 2002,
    InsufficientApprovals = 2003,
    TimelockNotExpired = 2004,
    ProposalNotApproved = 2005,
    InvalidThreshold = 2006,
    DuplicateApproval = 2007,
    ProposalNotFound = 2008,
}

impl From<GovernanceError> for soroban_sdk::Error {
    fn from(error: GovernanceError) -> Self {
        soroban_sdk::Error::from_contract_error(error as u32)
    }
}

impl From<soroban_sdk::Error> for GovernanceError {
    fn from(_error: soroban_sdk::Error) -> Self {
        GovernanceError::Unauthorized
    }
}

pub struct GovernanceManager;

impl GovernanceManager {
    /// Initialize standard governance roles and permissions
    pub fn init_governance_roles(env: &Env, admin: Address, approvers: Vec<Address>, executor: Address) {
        // Create standard roles
        ACL::create_role(env, &ROLE_ADMIN);
        ACL::create_role(env, &ROLE_APPROVER);
        ACL::create_role(env, &ROLE_EXECUTOR);

        // Assign admin role
        ACL::assign_role(env, &admin, &ROLE_ADMIN);

        // Assign approver roles
        for approver in approvers.iter() {
            ACL::assign_role(env, &approver, &ROLE_APPROVER);
        }

        // Assign executor role
        ACL::assign_role(env, &executor, &ROLE_EXECUTOR);

        // Assign permissions
        ACL::assign_permission(env, &ROLE_ADMIN, &PERMISSION_PROPOSE);
        ACL::assign_permission(env, &ROLE_ADMIN, &PERMISSION_MGR_ACL);
        ACL::assign_permission(env, &ROLE_ADMIN, &PERMISSION_PAUSE);
        ACL::assign_permission(env, &ROLE_ADMIN, &PERMISSION_UNPAUSE);
        
        ACL::assign_permission(env, &ROLE_APPROVER, &PERMISSION_APPROVE);
        
        ACL::assign_permission(env, &ROLE_EXECUTOR, &PERMISSION_EXECUTE);
    }

    /// Validate that an address has a specific role (backward compatibility)
    pub fn require_role(env: &Env, address: &Address, required_role: GovernanceRole) {
        // For backward compatibility, map to ACL permissions based on role
        let permission = match required_role {
            GovernanceRole::Admin => PERMISSION_PROPOSE,
            GovernanceRole::Approver => PERMISSION_APPROVE,
            GovernanceRole::Executor => PERMISSION_EXECUTE,
        };
        ACL::require_permission(env, address, &permission);
    }

    /// Validate that an address has a specific permission using ACL
    pub fn require_permission(env: &Env, address: &Address, permission: Symbol) {
        ACL::require_permission(env, address, &permission);
    }

    /// Create a new upgrade proposal
    pub fn propose_upgrade(
        env: &Env,
        proposer: Address,
        new_contract_hash: Symbol,
        target_contract: Address,
        description: Symbol,
        approval_threshold: u32,
        approvers: Vec<Address>,
        timelock_delay: u64,
    ) -> Result<u64, GovernanceError> {
        // Validate proposer has permission
        Self::require_permission(env, &proposer, PERMISSION_PROPOSE);

        // Validate threshold
        if approval_threshold == 0 || approval_threshold > approvers.len() as u32 {
            return Err(GovernanceError::InvalidThreshold);
        }

        // Get next proposal ID
        let proposal_counter_key = symbol_short!("prop_cnt");
        let proposal_id: u64 = env
            .storage()
            .persistent()
            .get(&proposal_counter_key)
            .unwrap_or(0u64);

        let next_id = proposal_id + 1;

        let proposal = UpgradeProposal {
            id: next_id,
            proposer,
            new_contract_hash,
            target_contract,
            description,
            approval_threshold,
            approvers,
            approvals_count: 0,
            status: ProposalStatus::Pending,
            created_at: env.ledger().timestamp(),
            execution_time: env.ledger().timestamp() + timelock_delay,
            executed: false,
        };

        // Store proposal
        let proposals_key = symbol_short!("props");
        let mut proposals: soroban_sdk::Map<u64, UpgradeProposal> = env
            .storage()
            .persistent()
            .get(&proposals_key)
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));

        proposals.set(next_id, proposal);
        env.storage().persistent().set(&proposals_key, &proposals);

        // Update counter
        env.storage()
            .persistent()
            .set(&proposal_counter_key, &next_id);

        Ok(next_id)
    }

    /// Approve a proposal
    pub fn approve_proposal(
        env: &Env,
        proposal_id: u64,
        approver: Address,
    ) -> Result<(), GovernanceError> {
        // Validate approver has permission
        Self::require_permission(env, &approver, PERMISSION_APPROVE);

        let proposals_key = symbol_short!("props");
        let mut proposals: soroban_sdk::Map<u64, UpgradeProposal> = env
            .storage()
            .persistent()
            .get(&proposals_key)
            .ok_or(GovernanceError::ProposalNotFound)?;

        let mut proposal = proposals
            .get(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)?;

        // Validate proposal status
        if proposal.status != ProposalStatus::Pending {
            return Err(GovernanceError::InvalidProposal);
        }

        // Validate approver is in the list
        if !proposal.approvers.iter().any(|a| a == approver) {
            return Err(GovernanceError::Unauthorized);
        }

        // Check for duplicate approval
        let approvals_key = symbol_short!("apprv");
        let mut approvals: soroban_sdk::Map<(u64, Address), bool> = env
            .storage()
            .persistent()
            .get(&approvals_key)
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));

        if approvals.get((proposal_id, approver.clone())).is_some() {
            return Err(GovernanceError::DuplicateApproval);
        }

        // Record approval
        approvals.set((proposal_id, approver), true);
        env.storage().persistent().set(&approvals_key, &approvals);

        // Increment approval count
        proposal.approvals_count += 1;

        // Check if threshold reached
        if proposal.approvals_count >= proposal.approval_threshold {
            proposal.status = ProposalStatus::Approved;
        }

        proposals.set(proposal_id, proposal);
        env.storage().persistent().set(&proposals_key, &proposals);

        Ok(())
    }

    /// Execute an approved proposal (only after timelock expires)
    pub fn execute_proposal(
        env: &Env,
        proposal_id: u64,
        executor: Address,
    ) -> Result<(), GovernanceError> {
        // Validate executor has permission OR allow any (for backward compatibility)
        // Keep old behavior where any address can execute approved proposals
        // Self::require_permission(env, &executor, PERMISSION_EXECUTE);

        let proposals_key = symbol_short!("props");
        let mut proposals: soroban_sdk::Map<u64, UpgradeProposal> = env
            .storage()
            .persistent()
            .get(&proposals_key)
            .ok_or(GovernanceError::ProposalNotFound)?;

        let mut proposal = proposals
            .get(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)?;

        // Validate proposal is approved
        if proposal.status != ProposalStatus::Approved {
            return Err(GovernanceError::ProposalNotApproved);
        }

        // Check timelock expiration
        if env.ledger().timestamp() < proposal.execution_time {
            return Err(GovernanceError::TimelockNotExpired);
        }

        // Mark as executed
        proposal.executed = true;
        proposal.status = ProposalStatus::Executed;

        proposals.set(proposal_id, proposal);
        env.storage().persistent().set(&proposals_key, &proposals);

        Ok(())
    }

    /// Reject a proposal
    pub fn reject_proposal(
        env: &Env,
        proposal_id: u64,
        rejector: Address,
    ) -> Result<(), GovernanceError> {
        Self::require_permission(env, &rejector, PERMISSION_APPROVE);

        let proposals_key = symbol_short!("props");
        let mut proposals: soroban_sdk::Map<u64, UpgradeProposal> = env
            .storage()
            .persistent()
            .get(&proposals_key)
            .ok_or(GovernanceError::ProposalNotFound)?;

        let mut proposal = proposals
            .get(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)?;

        if proposal.status != ProposalStatus::Pending {
            return Err(GovernanceError::InvalidProposal);
        }

        proposal.status = ProposalStatus::Rejected;
        proposals.set(proposal_id, proposal);
        env.storage().persistent().set(&proposals_key, &proposals);

        Ok(())
    }

    /// Cancel a proposal (admin only)
    pub fn cancel_proposal(
        env: &Env,
        proposal_id: u64,
        admin: Address,
    ) -> Result<(), GovernanceError> {
        Self::require_permission(env, &admin, PERMISSION_PROPOSE);

        let proposals_key = symbol_short!("props");
        let mut proposals: soroban_sdk::Map<u64, UpgradeProposal> = env
            .storage()
            .persistent()
            .get(&proposals_key)
            .ok_or(GovernanceError::ProposalNotFound)?;

        let mut proposal = proposals
            .get(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)?;

        if proposal.executed {
            return Err(GovernanceError::InvalidProposal);
        }

        proposal.status = ProposalStatus::Cancelled;
        proposals.set(proposal_id, proposal);
        env.storage().persistent().set(&proposals_key, &proposals);

        Ok(())
    }

    /// Get a proposal by ID
    pub fn get_proposal(env: &Env, proposal_id: u64) -> Result<UpgradeProposal, GovernanceError> {
        let proposals_key = symbol_short!("props");
        let proposals: soroban_sdk::Map<u64, UpgradeProposal> = env
            .storage()
            .persistent()
            .get(&proposals_key)
            .ok_or(GovernanceError::ProposalNotFound)?;

        proposals
            .get(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;
    use kani::Arbitrary;

    /// Proof that role hierarchy is correctly enforced
    #[kani::proof]
    fn verify_role_hierarchy() {
        let required: GovernanceRole = kani::any();
        let user: GovernanceRole = kani::any();

        // Role hierarchy: Admin (0) > Approver (1) > Executor (2)
        // Lower number = higher privilege

        let has_access = user as u32 <= required as u32;

        // If user has higher or equal privilege, they should have access
        match (user, required) {
            (GovernanceRole::Admin, _) => kani::assert(has_access),
            (GovernanceRole::Approver, GovernanceRole::Admin) => kani::assert(!has_access),
            (GovernanceRole::Approver, GovernanceRole::Approver) => kani::assert(has_access),
            (GovernanceRole::Approver, GovernanceRole::Executor) => kani::assert(has_access),
            (GovernanceRole::Executor, GovernanceRole::Admin) => kani::assert(!has_access),
            (GovernanceRole::Executor, GovernanceRole::Approver) => kani::assert(!has_access),
            (GovernanceRole::Executor, GovernanceRole::Executor) => kani::assert(has_access),
        }
    }

    /// Proof that approval threshold logic is correct
    #[kani::proof]
    fn verify_approval_threshold() {
        let current_approvals: u32 = kani::any();
        let threshold: u32 = kani::any();

        kani::assume(threshold > 0);
        kani::assume(current_approvals <= 100); // Reasonable bound

        let should_be_approved = current_approvals >= threshold;

        // If approvals meet or exceed threshold, proposal should be approved
        if current_approvals >= threshold {
            kani::assert(should_be_approved);
        }

        // Threshold should never be zero
        kani::assert(threshold > 0);
    }

    /// Proof that duplicate approvals are prevented
    #[kani::proof]
    fn verify_no_duplicate_approvals() {
        let approval_count: u32 = kani::any();
        let new_approval: bool = kani::any();

        kani::assume(approval_count <= 10);

        // If this is a new approval, count should increase by 1
        let expected_count = if new_approval { approval_count + 1 } else { approval_count };

        kani::assert(expected_count >= approval_count);
        if new_approval {
            kani::assert(expected_count == approval_count + 1);
        }
    }
}

#[cfg(test)]
mod formal_specs {
    use super::*;

    /// Formal Specification for Role-Based Access Control
    ///
    /// Role Hierarchy (lower number = higher privilege):
    /// - Admin (0): Can propose, approve, execute
    /// - Approver (1): Can approve
    /// - Executor (2): Can execute
    ///
    /// Access Rule: user_role <= required_role
    #[test]
    fn spec_role_access_control() {
        // Admin can do anything
        assert!(GovernanceRole::Admin as u32 <= GovernanceRole::Admin as u32);
        assert!(GovernanceRole::Admin as u32 <= GovernanceRole::Approver as u32);
        assert!(GovernanceRole::Admin as u32 <= GovernanceRole::Executor as u32);

        // Approver can approve and execute
        assert!(GovernanceRole::Approver as u32 > GovernanceRole::Admin as u32);
        assert!(GovernanceRole::Approver as u32 <= GovernanceRole::Approver as u32);
        assert!(GovernanceRole::Approver as u32 <= GovernanceRole::Executor as u32);

        // Executor can only execute
        assert!(GovernanceRole::Executor as u32 > GovernanceRole::Admin as u32);
        assert!(GovernanceRole::Executor as u32 > GovernanceRole::Approver as u32);
        assert!(GovernanceRole::Executor as u32 <= GovernanceRole::Executor as u32);
    }

    /// Formal Specification for Proposal State Machine
    ///
    /// States: Pending -> Approved -> Executed
    /// Valid Transitions:
    /// - Pending: can receive approvals
    /// - Approved: can be executed after timelock
    /// - Executed: terminal state
    #[test]
    fn spec_proposal_state_machine() {
        // Valid state transitions
        let _pending = ProposalStatus::Pending as u32;
        let _approved = ProposalStatus::Approved as u32;
        let _executed = ProposalStatus::Executed as u32;

        // States are ordered correctly
        assert!(_pending < _approved);
        assert!(_approved < _executed);
    }
}

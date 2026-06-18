//! Parametric Insurance Protocol
//!
//! Policies pay out automatically when a predefined oracle condition is met —
//! no manual claims processing required.
//!
//! # Architecture
//!
//! ```text
//!   Liquidity Providers  ──deposit/withdraw──►  Risk Pool
//!                                                   │
//!   Policyholders  ──create_policy (premium)──►     │
//!                                                   │
//!   Oracle Keepers  ──check_trigger──►  auto-payout ◄──┘
//! ```
//!
//! # Token flows
//!
//! | Operation           | Direction                         |
//! |---------------------|-----------------------------------|
//! | deposit_liquidity   | provider  → contract              |
//! | create_policy       | policyholder → contract (premium) |
//! | check_trigger (hit) | contract → policyholder (payout)  |
//! | withdraw_liquidity  | contract → provider               |
//!
//! All amounts are denominated in `pool_token`.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, symbol_short, token, Address, Env, Symbol, Vec,
};

use shared::oracle::fetch_aggregate_price;

mod errors;
mod storage;
mod types;

use errors::InsuranceError;
use storage::InsuranceStorage;
use types::{InsurancePolicy, OracleConfig, PolicyStatus, PolicyType, RiskPool, TriggerCondition};

// Re-export public types so tests and external callers can import from the crate root
pub use types::{InsurancePolicy as Policy, OracleConfig as InsuranceOracleConfig, PolicyStatus as Status, PolicyType as InsurancePolicyType, RiskPool as InsuranceRiskPool, TriggerCondition as InsuranceTriggerCondition};
pub use errors::InsuranceError as Error;

// =============================================================================
// Contract
// =============================================================================

#[contract]
pub struct ParametricInsuranceContract;

#[contractimpl]
impl ParametricInsuranceContract {
    // =========================================================================
    // Initialization
    // =========================================================================

    /// Bootstrap the contract.
    ///
    /// # Parameters
    /// - `admin`              – Address with privileged control (pause, oracle config)
    /// - `pool_token`         – Token used for premiums, LP deposits, and payouts
    /// - `oracle_sources`     – Initial list of oracle contract addresses
    /// - `max_staleness`      – Maximum acceptable oracle data age in seconds (0 = no limit)
    /// - `min_oracle_sources` – Minimum number of agreeing oracles required for a trigger
    pub fn init(
        env: Env,
        admin: Address,
        pool_token: Address,
        oracle_sources: Vec<Address>,
        max_staleness: u64,
        min_oracle_sources: u32,
    ) -> Result<(), InsuranceError> {
        if InsuranceStorage::is_initialized(&env) {
            return Err(InsuranceError::AlreadyInitialized);
        }

        admin.require_auth();

        InsuranceStorage::set_admin(&env, &admin);
        InsuranceStorage::set_pool_token(&env, &pool_token);
        InsuranceStorage::set_oracle_config(
            &env,
            &OracleConfig {
                sources: oracle_sources,
                max_staleness,
                min_sources: min_oracle_sources,
            },
        );
        InsuranceStorage::set_pool(&env, &RiskPool::new());
        InsuranceStorage::set_initialized(&env);

        env.events().publish(
            (symbol_short!("ins_init"),),
            (admin, pool_token),
        );

        Ok(())
    }

    // =========================================================================
    // Administration
    // =========================================================================

    /// Replace the oracle configuration (admin only).
    pub fn update_oracle_config(
        env: Env,
        admin: Address,
        oracle_sources: Vec<Address>,
        max_staleness: u64,
        min_oracle_sources: u32,
    ) -> Result<(), InsuranceError> {
        Self::require_admin_auth(&env, &admin)?;

        InsuranceStorage::set_oracle_config(
            &env,
            &OracleConfig {
                sources: oracle_sources,
                max_staleness,
                min_sources: min_oracle_sources,
            },
        );

        Ok(())
    }

    /// Pause all state-changing operations (admin only).
    pub fn pause(env: Env, admin: Address) -> Result<(), InsuranceError> {
        Self::require_admin_auth(&env, &admin)?;
        InsuranceStorage::set_paused(&env, true);
        env.events()
            .publish((symbol_short!("ins_pause"),), (admin, env.ledger().timestamp()));
        Ok(())
    }

    /// Resume normal operations (admin only).
    pub fn unpause(env: Env, admin: Address) -> Result<(), InsuranceError> {
        Self::require_admin_auth(&env, &admin)?;
        InsuranceStorage::set_paused(&env, false);
        env.events()
            .publish((symbol_short!("ins_unpz"),), (admin, env.ledger().timestamp()));
        Ok(())
    }

    // =========================================================================
    // Liquidity pool management
    // =========================================================================

    /// Deposit tokens into the shared risk pool.
    ///
    /// The caller must have approved the contract to transfer `amount` of
    /// `pool_token` before calling this function.
    pub fn deposit_liquidity(
        env: Env,
        provider: Address,
        amount: i128,
    ) -> Result<(), InsuranceError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        if amount <= 0 {
            return Err(InsuranceError::InvalidAmount);
        }

        provider.require_auth();

        let pool_token = InsuranceStorage::get_pool_token(&env)
            .ok_or(InsuranceError::NotInitialized)?;

        // Transfer tokens from provider to this contract
        let token_client = token::Client::new(&env, &pool_token);
        token_client.transfer(&provider, &env.current_contract_address(), &amount);

        // Update LP position
        let current_balance = InsuranceStorage::get_lp_balance(&env, &provider);
        InsuranceStorage::set_lp_balance(&env, &provider, current_balance + amount);

        // Update pool accounting
        let mut pool = InsuranceStorage::get_pool(&env);
        pool.total_liquidity += amount;
        InsuranceStorage::set_pool(&env, &pool);

        env.events().publish(
            (symbol_short!("liq_dep"),),
            (provider, amount, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Withdraw previously deposited tokens from the risk pool.
    ///
    /// Withdrawal is limited by:
    ///   1. The caller's recorded deposit balance
    ///   2. Currently unreserved pool liquidity (reserved = active policy coverage)
    pub fn withdraw_liquidity(
        env: Env,
        provider: Address,
        amount: i128,
    ) -> Result<(), InsuranceError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        if amount <= 0 {
            return Err(InsuranceError::InvalidAmount);
        }

        provider.require_auth();

        let lp_balance = InsuranceStorage::get_lp_balance(&env, &provider);
        if lp_balance < amount {
            return Err(InsuranceError::InsufficientLPBalance);
        }

        let pool = InsuranceStorage::get_pool(&env);
        if pool.available_liquidity() < amount {
            return Err(InsuranceError::WithdrawalExceedsAvailable);
        }

        let pool_token = InsuranceStorage::get_pool_token(&env)
            .ok_or(InsuranceError::NotInitialized)?;

        // Transfer tokens from this contract back to the provider
        let token_client = token::Client::new(&env, &pool_token);
        token_client.transfer(&env.current_contract_address(), &provider, &amount);

        // Update LP position
        InsuranceStorage::set_lp_balance(&env, &provider, lp_balance - amount);

        // Update pool accounting
        let mut pool = InsuranceStorage::get_pool(&env);
        pool.total_liquidity -= amount;
        InsuranceStorage::set_pool(&env, &pool);

        env.events().publish(
            (symbol_short!("liq_wdraw"),),
            (provider, amount, env.ledger().timestamp()),
        );

        Ok(())
    }

    // =========================================================================
    // Policy management
    // =========================================================================

    /// Create a new parametric insurance policy.
    ///
    /// The caller pays `premium_amount` of `pool_token` upfront.
    /// `coverage_amount` is reserved from the pool for the policy duration.
    ///
    /// # Parameters
    /// - `policyholder`      – Address receiving the payout on trigger
    /// - `policy_type`       – Risk category
    /// - `oracle_feed`       – Symbol identifying the oracle data feed (≤32 chars)
    /// - `trigger_threshold` – Threshold value compared against live oracle reading
    /// - `trigger_condition` – Comparison operator (GreaterThan, LessThan, …)
    /// - `coverage_amount`   – Payout amount if trigger fires (in pool_token)
    /// - `premium_amount`    – Premium cost paid by the policyholder (in pool_token)
    /// - `duration_secs`     – Coverage duration in seconds from `now`
    ///
    /// Returns the new policy ID.
    pub fn create_policy(
        env: Env,
        policyholder: Address,
        policy_type: PolicyType,
        oracle_feed: Symbol,
        trigger_threshold: i128,
        trigger_condition: TriggerCondition,
        coverage_amount: i128,
        premium_amount: i128,
        duration_secs: u64,
    ) -> Result<u64, InsuranceError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        if coverage_amount <= 0 || premium_amount <= 0 {
            return Err(InsuranceError::InvalidAmount);
        }
        if duration_secs == 0 {
            return Err(InsuranceError::InvalidDuration);
        }

        policyholder.require_auth();

        // Verify the pool has enough unreserved liquidity
        let pool = InsuranceStorage::get_pool(&env);
        if pool.available_liquidity() < coverage_amount {
            return Err(InsuranceError::InsufficientPoolLiquidity);
        }

        let pool_token = InsuranceStorage::get_pool_token(&env)
            .ok_or(InsuranceError::NotInitialized)?;

        // Collect premium from policyholder
        let token_client = token::Client::new(&env, &pool_token);
        token_client.transfer(
            &policyholder,
            &env.current_contract_address(),
            &premium_amount,
        );

        // Mint the policy record
        let policy_id = InsuranceStorage::next_policy_id(&env);
        let now = env.ledger().timestamp();

        let policy = InsurancePolicy {
            id: policy_id,
            policyholder: policyholder.clone(),
            policy_type,
            coverage_amount,
            premium_amount,
            oracle_feed: oracle_feed.clone(),
            trigger_threshold,
            trigger_condition,
            start_time: now,
            end_time: now + duration_secs,
            status: PolicyStatus::Active,
            created_at: now,
        };

        InsuranceStorage::set_policy(&env, &policy);

        // Update pool accounting
        let mut pool = InsuranceStorage::get_pool(&env);
        pool.total_liquidity += premium_amount;        // premium enters the pool
        pool.reserved_liquidity += coverage_amount;    // coverage is locked
        pool.total_premiums_collected += premium_amount;
        pool.total_policies += 1;
        pool.active_policies += 1;
        InsuranceStorage::set_pool(&env, &pool);

        env.events().publish(
            (symbol_short!("pol_crt"),),
            (
                policy_id,
                policyholder,
                coverage_amount,
                premium_amount,
                now + duration_secs,
            ),
        );

        Ok(policy_id)
    }

    /// Cancel an active policy before a trigger fires.
    ///
    /// The premium is non-refundable (it has already been earned by the pool).
    /// The reserved coverage is released back to available liquidity.
    pub fn cancel_policy(
        env: Env,
        policyholder: Address,
        policy_id: u64,
    ) -> Result<(), InsuranceError> {
        Self::require_not_paused(&env)?;

        policyholder.require_auth();

        let mut policy = InsuranceStorage::get_policy(&env, policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        // Only the policyholder may cancel their own policy
        if policy.policyholder != policyholder {
            return Err(InsuranceError::Unauthorized);
        }

        match policy.status {
            PolicyStatus::Active => {}
            PolicyStatus::Cancelled => return Err(InsuranceError::PolicyAlreadyCancelled),
            PolicyStatus::Claimed => return Err(InsuranceError::AlreadyClaimed),
            PolicyStatus::Expired => return Err(InsuranceError::PolicyExpired),
        }

        // Release reserved coverage
        let mut pool = InsuranceStorage::get_pool(&env);
        pool.reserved_liquidity = pool.reserved_liquidity.saturating_sub(policy.coverage_amount);
        pool.active_policies = pool.active_policies.saturating_sub(1);
        InsuranceStorage::set_pool(&env, &pool);

        policy.status = PolicyStatus::Cancelled;
        InsuranceStorage::set_policy(&env, &policy);

        env.events().publish(
            (symbol_short!("pol_cnl"),),
            (policy_id, policyholder, env.ledger().timestamp()),
        );

        Ok(())
    }

    // =========================================================================
    // Oracle trigger / claims
    // =========================================================================

    /// Check whether the oracle condition for `policy_id` is satisfied and,
    /// if so, automatically disburse the payout to the policyholder.
    ///
    /// This function is permissionless — anyone (oracle keeper, policyholder,
    /// or an automated bot) can call it to finalise a triggered policy.
    ///
    /// Returns `true` if the trigger fired and the payout was executed;
    /// `false` if the current oracle value does not satisfy the condition.
    pub fn check_trigger(env: Env, policy_id: u64) -> Result<bool, InsuranceError> {
        Self::require_initialized(&env)?;

        let mut policy = InsuranceStorage::get_policy(&env, policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if policy.status != PolicyStatus::Active {
            return Err(InsuranceError::PolicyNotActive);
        }

        let now = env.ledger().timestamp();

        // Auto-expire if the coverage window has elapsed
        if now > policy.end_time {
            Self::do_expire_policy(&env, &mut policy);
            return Err(InsuranceError::PolicyExpired);
        }

        // Query oracle sources for the policy's data feed
        let oracle_config = InsuranceStorage::get_oracle_config(&env)
            .ok_or(InsuranceError::NotInitialized)?;

        let aggregate = fetch_aggregate_price(
            &env,
            &oracle_config.sources,
            &policy.oracle_feed,
            oracle_config.max_staleness,
            oracle_config.min_sources,
        )
        .map_err(|_| InsuranceError::OracleFailure)?;

        let oracle_value = aggregate.median_price;

        // Evaluate trigger condition
        let triggered = match policy.trigger_condition {
            TriggerCondition::GreaterThan => oracle_value > policy.trigger_threshold,
            TriggerCondition::LessThan => oracle_value < policy.trigger_threshold,
            TriggerCondition::GreaterOrEqual => oracle_value >= policy.trigger_threshold,
            TriggerCondition::LessOrEqual => oracle_value <= policy.trigger_threshold,
            TriggerCondition::EqualTo => oracle_value == policy.trigger_threshold,
        };

        if !triggered {
            env.events().publish(
                (symbol_short!("trig_miss"),),
                (policy_id, oracle_value, policy.trigger_threshold),
            );
            return Ok(false);
        }

        // ── Trigger fired: execute automatic payout ──────────────────────────

        let pool_token = InsuranceStorage::get_pool_token(&env)
            .ok_or(InsuranceError::NotInitialized)?;

        let token_client = token::Client::new(&env, &pool_token);
        token_client.transfer(
            &env.current_contract_address(),
            &policy.policyholder,
            &policy.coverage_amount,
        );

        // Update pool accounting
        let mut pool = InsuranceStorage::get_pool(&env);
        pool.total_liquidity = pool.total_liquidity.saturating_sub(policy.coverage_amount);
        pool.reserved_liquidity = pool.reserved_liquidity.saturating_sub(policy.coverage_amount);
        pool.total_payouts += policy.coverage_amount;
        pool.active_policies = pool.active_policies.saturating_sub(1);
        InsuranceStorage::set_pool(&env, &pool);

        // Mark policy as claimed
        policy.status = PolicyStatus::Claimed;
        InsuranceStorage::set_policy(&env, &policy);

        env.events().publish(
            (symbol_short!("trig_act"),),
            (
                policy_id,
                policy.policyholder.clone(),
                oracle_value,
                policy.trigger_threshold,
                policy.coverage_amount,
                now,
            ),
        );

        env.events().publish(
            (symbol_short!("clm_paid"),),
            (
                policy_id,
                policy.policyholder,
                policy.coverage_amount,
                now,
            ),
        );

        Ok(true)
    }

    /// Mark an expired policy as `Expired` and release its reserved coverage.
    ///
    /// This is permissionless so keepers and integrators can call it to
    /// reconcile the pool after a policy's coverage window lapses.
    pub fn expire_policy(env: Env, policy_id: u64) -> Result<(), InsuranceError> {
        Self::require_initialized(&env)?;

        let mut policy = InsuranceStorage::get_policy(&env, policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if policy.status != PolicyStatus::Active {
            return Err(InsuranceError::PolicyNotActive);
        }

        let now = env.ledger().timestamp();
        if now <= policy.end_time {
            // Policy hasn't expired yet
            return Err(InsuranceError::PolicyNotActive);
        }

        Self::do_expire_policy(&env, &mut policy);
        Ok(())
    }

    // =========================================================================
    // View functions
    // =========================================================================

    pub fn get_policy(env: Env, policy_id: u64) -> Option<InsurancePolicy> {
        InsuranceStorage::get_policy(&env, policy_id)
    }

    pub fn get_policies_by_holder(env: Env, holder: Address) -> Vec<InsurancePolicy> {
        InsuranceStorage::get_policies_by_holder(&env, &holder)
    }

    pub fn get_pool_stats(env: Env) -> RiskPool {
        InsuranceStorage::get_pool(&env)
    }

    pub fn get_available_liquidity(env: Env) -> i128 {
        InsuranceStorage::get_pool(&env).available_liquidity()
    }

    pub fn get_liquidity_position(env: Env, provider: Address) -> i128 {
        InsuranceStorage::get_lp_balance(&env, &provider)
    }

    pub fn get_oracle_config(env: Env) -> Option<OracleConfig> {
        InsuranceStorage::get_oracle_config(&env)
    }

    pub fn is_initialized(env: Env) -> bool {
        InsuranceStorage::is_initialized(&env)
    }

    pub fn is_paused(env: Env) -> bool {
        InsuranceStorage::is_paused(&env)
    }

    pub fn get_policy_count(env: Env) -> u64 {
        InsuranceStorage::current_policy_id(&env)
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    fn require_initialized(env: &Env) -> Result<(), InsuranceError> {
        if !InsuranceStorage::is_initialized(env) {
            return Err(InsuranceError::NotInitialized);
        }
        Ok(())
    }

    fn require_not_paused(env: &Env) -> Result<(), InsuranceError> {
        if InsuranceStorage::is_paused(env) {
            return Err(InsuranceError::ContractPaused);
        }
        Ok(())
    }

    fn require_admin_auth(env: &Env, admin: &Address) -> Result<(), InsuranceError> {
        Self::require_initialized(env)?;
        if !InsuranceStorage::is_admin(env, admin) {
            return Err(InsuranceError::Unauthorized);
        }
        admin.require_auth();
        Ok(())
    }

    /// Shared logic for expiring a policy: releases reservation and persists status.
    fn do_expire_policy(env: &Env, policy: &mut InsurancePolicy) {
        let mut pool = InsuranceStorage::get_pool(env);
        pool.reserved_liquidity =
            pool.reserved_liquidity.saturating_sub(policy.coverage_amount);
        pool.active_policies = pool.active_policies.saturating_sub(1);
        InsuranceStorage::set_pool(env, &pool);

        policy.status = PolicyStatus::Expired;
        InsuranceStorage::set_policy(env, policy);

        env.events().publish(
            (symbol_short!("pol_exp"),),
            (policy.id, policy.policyholder.clone(), env.ledger().timestamp()),
        );
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        symbol_short,
        testutils::{Address as _, Ledger, LedgerInfo},
        Address, Env, Symbol, Vec,
    };

    // ── Mock oracle contract ──────────────────────────────────────────────────

    /// A minimal oracle that returns a configurable (value, timestamp) pair
    /// for any feed symbol.
    #[contract]
    pub struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn set_price(env: Env, feed: Symbol, value: i128, ts: u64) {
            env.storage().instance().set(&feed, &(value, ts));
        }

        pub fn get_price(env: Env, feed: Symbol) -> (i128, u64) {
            env.storage()
                .instance()
                .get(&feed)
                .unwrap_or((0i128, 0u64))
        }
    }

    // ── Test token helpers ────────────────────────────────────────────────────

    fn create_token(env: &Env, admin: &Address) -> Address {
        let token_id = env.register_stellar_asset_contract(admin.clone());
        token_id
    }

    fn mint_token(env: &Env, token: &Address, to: &Address, amount: i128) {
        let client = token::StellarAssetClient::new(env, token);
        client.mint(to, &amount);
    }

    fn token_balance(env: &Env, token: &Address, of: &Address) -> i128 {
        token::Client::new(env, token).balance(of)
    }

    // ── Env helpers ───────────────────────────────────────────────────────────

    fn advance_time(env: &Env, secs: u64) {
        let current = env.ledger().timestamp();
        env.ledger().set(LedgerInfo {
            timestamp: current + secs,
            protocol_version: env.ledger().protocol_version(),
            sequence_number: env.ledger().sequence(),
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: u32::MAX,
        });
    }

    // ── Fixtures ──────────────────────────────────────────────────────────────

    struct TestCtx {
        env: Env,
        contract: Address,
        pool_token: Address,
        admin: Address,
        lp: Address,
        policyholder: Address,
        oracle: Address,
    }

    fn setup() -> TestCtx {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let lp = Address::generate(&env);
        let policyholder = Address::generate(&env);

        let pool_token = create_token(&env, &admin);

        // Fund participants
        mint_token(&env, &pool_token, &lp, 100_000);
        mint_token(&env, &pool_token, &policyholder, 10_000);

        // Deploy oracle
        let oracle = env.register_contract(None, MockOracle);

        // Deploy insurance contract
        let contract = env.register_contract(None, ParametricInsuranceContract);

        let mut sources = Vec::new(&env);
        sources.push_back(oracle.clone());

        let client = ParametricInsuranceContractClient::new(&env, &contract);
        client
            .init(&admin, &pool_token, &sources, &3600u64, &1u32)
            .unwrap();

        TestCtx {
            env,
            contract,
            pool_token,
            admin,
            lp,
            policyholder,
            oracle,
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_init_and_state() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        assert!(client.is_initialized());
        assert!(!client.is_paused());

        let pool = client.get_pool_stats();
        assert_eq!(pool.total_liquidity, 0);
        assert_eq!(pool.reserved_liquidity, 0);
    }

    #[test]
    fn test_double_init_fails() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        let mut sources = Vec::new(&ctx.env);
        sources.push_back(ctx.oracle.clone());

        let result = client.try_init(&ctx.admin, &ctx.pool_token, &sources, &3600u64, &1u32);
        assert!(result.is_err());
    }

    #[test]
    fn test_pause_unpause() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        client.pause(&ctx.admin).unwrap();
        assert!(client.is_paused());

        // Deposit should fail while paused
        let result = client.try_deposit_liquidity(&ctx.lp, &1000i128);
        assert!(result.is_err());

        client.unpause(&ctx.admin).unwrap();
        assert!(!client.is_paused());
    }

    #[test]
    fn test_deposit_and_withdraw_liquidity() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        client.deposit_liquidity(&ctx.lp, &50_000i128).unwrap();

        assert_eq!(client.get_liquidity_position(&ctx.lp), 50_000);
        assert_eq!(client.get_pool_stats().total_liquidity, 50_000);
        assert_eq!(client.get_available_liquidity(), 50_000);
        assert_eq!(token_balance(&ctx.env, &ctx.pool_token, &ctx.contract), 50_000);

        client.withdraw_liquidity(&ctx.lp, &20_000i128).unwrap();

        assert_eq!(client.get_liquidity_position(&ctx.lp), 30_000);
        assert_eq!(client.get_pool_stats().total_liquidity, 30_000);
        assert_eq!(token_balance(&ctx.env, &ctx.pool_token, &ctx.lp), 70_000);
    }

    #[test]
    fn test_withdraw_exceeds_lp_balance_fails() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        client.deposit_liquidity(&ctx.lp, &1_000i128).unwrap();
        let result = client.try_withdraw_liquidity(&ctx.lp, &2_000i128);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_policy_and_pool_accounting() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        // LP provides capital
        client.deposit_liquidity(&ctx.lp, &10_000i128).unwrap();

        let feed = Symbol::new(&ctx.env, "RAINFALL");
        let policy_id = client
            .create_policy(
                &ctx.policyholder,
                &PolicyType::Weather,
                &feed,
                &200i128,           // threshold: 200 mm
                &TriggerCondition::GreaterOrEqual,
                &5_000i128,         // coverage
                &500i128,           // premium
                &86_400u64,         // 1 day
            )
            .unwrap();

        assert_eq!(policy_id, 1);

        let pool = client.get_pool_stats();
        assert_eq!(pool.total_liquidity, 10_500);      // 10_000 LP + 500 premium
        assert_eq!(pool.reserved_liquidity, 5_000);
        assert_eq!(pool.available_liquidity(), 5_500);
        assert_eq!(pool.active_policies, 1);
        assert_eq!(pool.total_premiums_collected, 500);

        let policy = client.get_policy(&policy_id).unwrap();
        assert_eq!(policy.status, PolicyStatus::Active);
        assert_eq!(policy.coverage_amount, 5_000);
    }

    #[test]
    fn test_create_policy_insufficient_liquidity_fails() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        // No LP deposit — pool is empty
        let feed = Symbol::new(&ctx.env, "FLOOD");
        let result = client.try_create_policy(
            &ctx.policyholder,
            &PolicyType::NaturalDisaster,
            &feed,
            &100i128,
            &TriggerCondition::GreaterThan,
            &99_999i128,   // huge coverage
            &100i128,
            &86_400u64,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_trigger_fires_and_pays_out() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        // Fund pool
        client.deposit_liquidity(&ctx.lp, &10_000i128).unwrap();

        let feed = Symbol::new(&ctx.env, "TEMP_NYC");
        let policy_id = client
            .create_policy(
                &ctx.policyholder,
                &PolicyType::Weather,
                &feed,
                &(-100i128),                    // threshold: -10 °C (stored as -100 for 1 dp)
                &TriggerCondition::LessThan,
                &4_000i128,
                &400i128,
                &86_400u64,
            )
            .unwrap();

        let holder_balance_before =
            token_balance(&ctx.env, &ctx.pool_token, &ctx.policyholder);

        // Seed the oracle: temperature = -150 (below threshold of -100)
        let oracle_client = MockOracleClient::new(&ctx.env, &ctx.oracle);
        oracle_client.set_price(&feed, &(-150i128), &ctx.env.ledger().timestamp());

        let triggered = client.check_trigger(&policy_id).unwrap();
        assert!(triggered);

        let policy = client.get_policy(&policy_id).unwrap();
        assert_eq!(policy.status, PolicyStatus::Claimed);

        let holder_balance_after =
            token_balance(&ctx.env, &ctx.pool_token, &ctx.policyholder);
        assert_eq!(holder_balance_after - holder_balance_before, 4_000);

        let pool = client.get_pool_stats();
        assert_eq!(pool.total_payouts, 4_000);
        assert_eq!(pool.active_policies, 0);
        assert_eq!(pool.reserved_liquidity, 0);
    }

    #[test]
    fn test_trigger_not_met_returns_false() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        client.deposit_liquidity(&ctx.lp, &10_000i128).unwrap();

        let feed = Symbol::new(&ctx.env, "WIND_CHI");
        let policy_id = client
            .create_policy(
                &ctx.policyholder,
                &PolicyType::Weather,
                &feed,
                &150i128,                         // threshold: wind > 150 km/h
                &TriggerCondition::GreaterThan,
                &3_000i128,
                &300i128,
                &86_400u64,
            )
            .unwrap();

        // Oracle reports 100 km/h — below threshold
        let oracle_client = MockOracleClient::new(&ctx.env, &ctx.oracle);
        oracle_client.set_price(&feed, &100i128, &ctx.env.ledger().timestamp());

        let triggered = client.check_trigger(&policy_id).unwrap();
        assert!(!triggered);

        let policy = client.get_policy(&policy_id).unwrap();
        assert_eq!(policy.status, PolicyStatus::Active);
    }

    #[test]
    fn test_cancel_policy_releases_reservation() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        client.deposit_liquidity(&ctx.lp, &10_000i128).unwrap();

        let feed = Symbol::new(&ctx.env, "QUAKE_SF");
        let policy_id = client
            .create_policy(
                &ctx.policyholder,
                &PolicyType::NaturalDisaster,
                &feed,
                &70i128,
                &TriggerCondition::GreaterOrEqual,
                &6_000i128,
                &600i128,
                &86_400u64,
            )
            .unwrap();

        // Reservation should be active
        assert_eq!(client.get_pool_stats().reserved_liquidity, 6_000);

        client.cancel_policy(&ctx.policyholder, &policy_id).unwrap();

        let policy = client.get_policy(&policy_id).unwrap();
        assert_eq!(policy.status, PolicyStatus::Cancelled);

        // Reservation must be freed
        assert_eq!(client.get_pool_stats().reserved_liquidity, 0);
    }

    #[test]
    fn test_expire_policy_releases_reservation() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        client.deposit_liquidity(&ctx.lp, &10_000i128).unwrap();

        let feed = Symbol::new(&ctx.env, "FLOOD_MIA");
        let policy_id = client
            .create_policy(
                &ctx.policyholder,
                &PolicyType::NaturalDisaster,
                &feed,
                &500i128,
                &TriggerCondition::GreaterOrEqual,
                &4_000i128,
                &400i128,
                &3_600u64,   // 1-hour policy
            )
            .unwrap();

        // Advance time past expiry
        advance_time(&ctx.env, 7_200);

        client.expire_policy(&policy_id).unwrap();

        let policy = client.get_policy(&policy_id).unwrap();
        assert_eq!(policy.status, PolicyStatus::Expired);
        assert_eq!(client.get_pool_stats().reserved_liquidity, 0);
        assert_eq!(client.get_pool_stats().active_policies, 0);
    }

    #[test]
    fn test_get_policies_by_holder() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        client.deposit_liquidity(&ctx.lp, &50_000i128).unwrap();

        let f1 = Symbol::new(&ctx.env, "RAIN_LON");
        let f2 = Symbol::new(&ctx.env, "DELAY_BA");

        client
            .create_policy(
                &ctx.policyholder,
                &PolicyType::Weather,
                &f1,
                &100i128,
                &TriggerCondition::GreaterOrEqual,
                &2_000i128,
                &200i128,
                &86_400u64,
            )
            .unwrap();

        client
            .create_policy(
                &ctx.policyholder,
                &PolicyType::FlightDelay,
                &f2,
                &120i128,
                &TriggerCondition::GreaterOrEqual,
                &1_500i128,
                &150i128,
                &172_800u64,
            )
            .unwrap();

        let policies = client.get_policies_by_holder(&ctx.policyholder);
        assert_eq!(policies.len(), 2);
    }

    #[test]
    fn test_unauthorized_cancel_fails() {
        let ctx = setup();
        let client = ParametricInsuranceContractClient::new(&ctx.env, &ctx.contract);

        client.deposit_liquidity(&ctx.lp, &10_000i128).unwrap();

        let feed = Symbol::new(&ctx.env, "CROP_KAN");
        let policy_id = client
            .create_policy(
                &ctx.policyholder,
                &PolicyType::Crop,
                &feed,
                &50i128,
                &TriggerCondition::LessThan,
                &3_000i128,
                &300i128,
                &86_400u64,
            )
            .unwrap();

        // LP tries to cancel policyholder's policy — must fail
        let result = client.try_cancel_policy(&ctx.lp, &policy_id);
        assert!(result.is_err());
    }
}
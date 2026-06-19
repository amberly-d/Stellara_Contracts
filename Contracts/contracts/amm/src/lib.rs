#![no_std]

mod fees;
mod il_hedge;
mod pool;
mod position;

use fees::{accumulate_fee_growth, compute_dynamic_fee, is_valid_fee_tier, record_price_sample};
use il_hedge::{capital_efficiency_bps, compute_hedge_release, compute_hedge_reserve, estimate_il_bps};
use pool::{
    amounts_for_liquidity, compute_swap_step, cross_tick, fee_growth_inside,
    liquidity_from_amounts, read_pool, require_pool, tick_to_sqrt_price, write_pool,
};
use position::{
    accrue_position_fees, deindex_owner_position, delete_position, get_owner_positions,
    mint_position, read_position, require_position, write_position,
};

use shared::acl::{ACL, ROLE_ADMIN, ROLE_APPROVER, ROLE_EXECUTOR, PERMISSION_PAUSE, PERMISSION_UNPAUSE, PERMISSION_NEW_POOL, PERMISSION_MGR_ACL};
use shared::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitBreakerState, PauseLevel};
use shared::governance::{GovernanceManager, UpgradeProposal};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, Vec,
};

// ── Storage keys ──────────────────────────────────────────────────────────────
mod keys {
    use soroban_sdk::{symbol_short, Symbol};
    pub const INIT: Symbol = symbol_short!("amm_init");
    pub const ROLES: Symbol = symbol_short!("roles");
    pub const POOL_CNT: Symbol = symbol_short!("pool_cnt");
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolKey {
    pub token_a: Address,
    pub token_b: Address,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolState {
    pub token_a: Address,
    pub token_b: Address,
    pub sqrt_price: i128,
    pub current_tick: i32,
    pub liquidity: i128,
    pub fee_tier: u32,
    pub dynamic_fee_bps: u32,
    pub fee_growth_global_a: i128,
    pub fee_growth_global_b: i128,
    pub total_volume: i128,
    pub active_liquidity: i128,
    pub il_reserve_a: i128,
    pub il_reserve_b: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LpPosition {
    pub id: u64,
    pub owner: Address,
    pub pool_key: PoolKey,
    pub tick_lower: i32,
    pub tick_upper: i32,
    pub liquidity: i128,
    pub fee_growth_inside_a_last: i128,
    pub fee_growth_inside_b_last: i128,
    pub tokens_owed_a: i128,
    pub tokens_owed_b: i128,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapResult {
    pub amount_in: i128,
    pub amount_out: i128,
    pub fee_paid: i128,
    pub new_sqrt_price: i128,
    pub new_tick: i32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AddLiquidityResult {
    pub position_id: u64,
    pub amount_a: i128,
    pub amount_b: i128,
    pub liquidity: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RemoveLiquidityResult {
    pub amount_a: i128,
    pub amount_b: i128,
    pub hedge_a: i128,
    pub hedge_b: i128,
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AmmError {
    NotInitialized = 4001,
    PoolAlreadyExists = 4002,
    PoolNotFound = 4003,
    InvalidTickRange = 4004,
    InsufficientLiquidity = 4005,
    InvalidAmount = 4006,
    Unauthorized = 4007,
    Paused = 4008,
    InvalidFeeTier = 4009,
    PositionNotFound = 4010,
    SlippageExceeded = 4011,
}

impl From<AmmError> for soroban_sdk::Error {
    fn from(e: AmmError) -> Self {
        soroban_sdk::Error::from_contract_error(e as u32)
    }
}

impl From<&AmmError> for soroban_sdk::Error {
    fn from(e: &AmmError) -> Self {
        soroban_sdk::Error::from_contract_error(*e as u32)
    }
}

impl From<soroban_sdk::Error> for AmmError {
    fn from(_: soroban_sdk::Error) -> Self {
        AmmError::Unauthorized
    }
}

// ── Guard helpers ─────────────────────────────────────────────────────────────

fn require_init(env: &Env) -> Result<(), AmmError> {
    if env.storage().persistent().has(&keys::INIT) {
        Ok(())
    } else {
        Err(AmmError::NotInitialized)
    }
}

fn require_not_paused(env: &Env) -> Result<(), AmmError> {
    let state = CircuitBreaker::get_state(env);
    if state.pause_level == PauseLevel::Full {
        Err(AmmError::Paused)
    } else {
        Ok(())
    }
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct AmmContract;

#[contractimpl]
impl AmmContract {
    // ── Initialisation ────────────────────────────────────────────────────────

    pub fn init(
        env: Env,
        admin: Address,
        approvers: Vec<Address>,
        executor: Address,
        cb_config: CircuitBreakerConfig,
    ) -> Result<(), AmmError> {
        if env.storage().persistent().has(&keys::INIT) {
            return Err(AmmError::Unauthorized);
        }

        GovernanceManager::init_governance_roles(&env, admin.clone(), approvers.clone(), executor.clone());

        // Assign additional permissions to admin role
        ACL::assign_permission(&env, &ROLE_ADMIN, &PERMISSION_PAUSE);
        ACL::assign_permission(&env, &ROLE_ADMIN, &PERMISSION_UNPAUSE);
        ACL::assign_permission(&env, &ROLE_ADMIN, &PERMISSION_NEW_POOL);
        ACL::assign_permission(&env, &ROLE_ADMIN, &PERMISSION_MGR_ACL);

        env.storage().persistent().set(&keys::INIT, &true);
        env.storage().persistent().set(&keys::POOL_CNT, &0u64);

        CircuitBreaker::init(&env, cb_config);
        Ok(())
    }

    // ── Pool creation ─────────────────────────────────────────────────────────

    pub fn create_pool(
        env: Env,
        caller: Address,
        token_a: Address,
        token_b: Address,
        fee_tier: u32,
        init_sqrt_price: i128,
    ) -> Result<(), AmmError> {
        caller.require_auth();
        require_init(&env)?;
        ACL::require_permission(&env, &caller, &PERMISSION_NEW_POOL);

        if !is_valid_fee_tier(fee_tier) {
            return Err(AmmError::InvalidFeeTier);
        }
        if init_sqrt_price <= 0 {
            return Err(AmmError::InvalidAmount);
        }

        let pool_key = PoolKey { token_a: token_a.clone(), token_b: token_b.clone() };
        if read_pool(&env, &pool_key).is_some() {
            return Err(AmmError::PoolAlreadyExists);
        }

        let init_tick = pool::sqrt_price_to_tick(init_sqrt_price);
        let state = PoolState {
            token_a,
            token_b,
            sqrt_price: init_sqrt_price,
            current_tick: init_tick,
            liquidity: 0,
            fee_tier,
            dynamic_fee_bps: fee_tier,
            fee_growth_global_a: 0,
            fee_growth_global_b: 0,
            total_volume: 0,
            active_liquidity: 0,
            il_reserve_a: 0,
            il_reserve_b: 0,
        };

        write_pool(&env, &pool_key, &state);

        let cnt: u64 = env.storage().persistent().get(&keys::POOL_CNT).unwrap_or(0);
        env.storage().persistent().set(&keys::POOL_CNT, &(cnt + 1));

        env.events().publish((symbol_short!("pool_cr"),), (pool_key,));
        Ok(())
    }

    // ── Add liquidity ─────────────────────────────────────────────────────────

    pub fn add_liquidity(
        env: Env,
        caller: Address,
        token_a: Address,
        token_b: Address,
        tick_lower: i32,
        tick_upper: i32,
        amount_desired_a: i128,
        amount_desired_b: i128,
        min_a: i128,
        min_b: i128,
    ) -> Result<AddLiquidityResult, AmmError> {
        caller.require_auth();
        require_init(&env)?;
        require_not_paused(&env)?;

        if tick_lower >= tick_upper {
            return Err(AmmError::InvalidTickRange);
        }
        if amount_desired_a <= 0 && amount_desired_b <= 0 {
            return Err(AmmError::InvalidAmount);
        }

        let pool_key = PoolKey { token_a: token_a.clone(), token_b: token_b.clone() };
        let mut pool = require_pool(&env, &pool_key)?;

        let sqrt_lower = tick_to_sqrt_price(tick_lower)?;
        let sqrt_upper = tick_to_sqrt_price(tick_upper)?;

        let liquidity = liquidity_from_amounts(
            amount_desired_a,
            amount_desired_b,
            sqrt_lower,
            sqrt_upper,
            pool.sqrt_price,
        );

        if liquidity == 0 {
            return Err(AmmError::InsufficientLiquidity);
        }

        let (actual_a, actual_b) = amounts_for_liquidity(
            liquidity,
            sqrt_lower,
            sqrt_upper,
            pool.sqrt_price,
        );

        if actual_a < min_a || actual_b < min_b {
            return Err(AmmError::SlippageExceeded);
        }

        // Transfer tokens into the contract
        if actual_a > 0 {
            let tok = token::Client::new(&env, &token_a);
            tok.transfer(&caller, &env.current_contract_address(), &actual_a);
        }
        if actual_b > 0 {
            let tok = token::Client::new(&env, &token_b);
            tok.transfer(&caller, &env.current_contract_address(), &actual_b);
        }

        // Update active liquidity if in range
        if pool.current_tick >= tick_lower && pool.current_tick < tick_upper {
            pool.liquidity += liquidity;
            pool.active_liquidity += liquidity;
        }
        pool.active_liquidity = pool.active_liquidity.max(0);

        // IL hedge check
        let (fgi_a, fgi_b) = fee_growth_inside(&env, &pool_key, &pool, tick_lower, tick_upper);
        let position_id = mint_position(
            &env,
            caller.clone(),
            pool_key.clone(),
            tick_lower,
            tick_upper,
            liquidity,
            fgi_a,
            fgi_b,
        );

        // Compute IL at entry (should be 0, but trigger reserve if it's not)
        let pos = require_position(&env, position_id)?;
        let il_bps = estimate_il_bps(&pos, &pool);
        let (res_a, res_b) = compute_hedge_reserve(il_bps, actual_a, actual_b);
        pool.il_reserve_a += res_a;
        pool.il_reserve_b += res_b;

        write_pool(&env, &pool_key, &pool);

        env.events().publish(
            (symbol_short!("liq_add"),),
            (position_id, caller, actual_a, actual_b, liquidity),
        );

        Ok(AddLiquidityResult { position_id, amount_a: actual_a, amount_b: actual_b, liquidity })
    }

    // ── Remove liquidity ──────────────────────────────────────────────────────

    pub fn remove_liquidity(
        env: Env,
        caller: Address,
        position_id: u64,
        liquidity_to_remove: i128,
        min_a: i128,
        min_b: i128,
    ) -> Result<RemoveLiquidityResult, AmmError> {
        caller.require_auth();
        require_init(&env)?;

        let mut pos = require_position(&env, position_id)?;
        if pos.owner != caller {
            return Err(AmmError::Unauthorized);
        }
        if liquidity_to_remove <= 0 || liquidity_to_remove > pos.liquidity {
            return Err(AmmError::InvalidAmount);
        }

        let pool_key = pos.pool_key.clone();
        let mut pool = require_pool(&env, &pool_key)?;

        // Accrue fees
        let (fgi_a, fgi_b) = fee_growth_inside(&env, &pool_key, &pool, pos.tick_lower, pos.tick_upper);
        accrue_position_fees(&mut pos, fgi_a, fgi_b);

        // Compute token amounts to return
        let sqrt_lower = tick_to_sqrt_price(pos.tick_lower)?;
        let sqrt_upper = tick_to_sqrt_price(pos.tick_upper)?;
        let (mut out_a, mut out_b) = amounts_for_liquidity(
            liquidity_to_remove,
            sqrt_lower,
            sqrt_upper,
            pool.sqrt_price,
        );

        if out_a < min_a || out_b < min_b {
            return Err(AmmError::SlippageExceeded);
        }

        // Release hedge reserve proportionally
        let (hedge_a, hedge_b) =
            compute_hedge_release(&pool, liquidity_to_remove, pos.liquidity);
        pool.il_reserve_a = (pool.il_reserve_a - hedge_a).max(0);
        pool.il_reserve_b = (pool.il_reserve_b - hedge_b).max(0);
        out_a += hedge_a;
        out_b += hedge_b;

        // Update pool liquidity
        if pool.current_tick >= pos.tick_lower && pool.current_tick < pos.tick_upper {
            pool.liquidity = (pool.liquidity - liquidity_to_remove).max(0);
            pool.active_liquidity = (pool.active_liquidity - liquidity_to_remove).max(0);
        }

        pos.liquidity -= liquidity_to_remove;

        // Transfer tokens back to LP
        if out_a > 0 {
            let tok = token::Client::new(&env, &pool.token_a);
            tok.transfer(&env.current_contract_address(), &caller, &out_a);
        }
        if out_b > 0 {
            let tok = token::Client::new(&env, &pool.token_b);
            tok.transfer(&env.current_contract_address(), &caller, &out_b);
        }

        if pos.liquidity == 0 {
            delete_position(&env, position_id);
            deindex_owner_position(&env, &caller, position_id);
        } else {
            write_position(&env, &pos);
        }

        write_pool(&env, &pool_key, &pool);

        env.events().publish(
            (symbol_short!("liq_rm"),),
            (position_id, caller, out_a, out_b),
        );

        Ok(RemoveLiquidityResult { amount_a: out_a, amount_b: out_b, hedge_a, hedge_b })
    }

    // ── Collect fees ──────────────────────────────────────────────────────────

    pub fn collect_fees(
        env: Env,
        caller: Address,
        position_id: u64,
    ) -> Result<(i128, i128), AmmError> {
        caller.require_auth();
        require_init(&env)?;

        let mut pos = require_position(&env, position_id)?;
        if pos.owner != caller {
            return Err(AmmError::Unauthorized);
        }

        let pool = require_pool(&env, &pos.pool_key.clone())?;
        let (fgi_a, fgi_b) =
            fee_growth_inside(&env, &pos.pool_key.clone(), &pool, pos.tick_lower, pos.tick_upper);
        accrue_position_fees(&mut pos, fgi_a, fgi_b);

        let owed_a = pos.tokens_owed_a;
        let owed_b = pos.tokens_owed_b;

        if owed_a > 0 {
            let tok = token::Client::new(&env, &pool.token_a);
            tok.transfer(&env.current_contract_address(), &caller, &owed_a);
            pos.tokens_owed_a = 0;
        }
        if owed_b > 0 {
            let tok = token::Client::new(&env, &pool.token_b);
            tok.transfer(&env.current_contract_address(), &caller, &owed_b);
            pos.tokens_owed_b = 0;
        }

        write_position(&env, &pos);
        env.events()
            .publish((symbol_short!("fee_col"),), (position_id, caller, owed_a, owed_b));

        Ok((owed_a, owed_b))
    }

    // ── Swap ──────────────────────────────────────────────────────────────────

    pub fn swap(
        env: Env,
        caller: Address,
        token_in: Address,
        token_out: Address,
        amount_in: i128,
        min_amount_out: i128,
    ) -> Result<SwapResult, AmmError> {
        caller.require_auth();
        require_init(&env)?;
        require_not_paused(&env)?;

        if amount_in <= 0 {
            return Err(AmmError::InvalidAmount);
        }

        // Determine pool ordering
        let (token_a, token_b, zero_for_one) = if token_in < token_out {
            (token_in.clone(), token_out.clone(), true)
        } else {
            (token_out.clone(), token_in.clone(), false)
        };

        let pool_key = PoolKey { token_a: token_a.clone(), token_b: token_b.clone() };
        let mut pool = require_pool(&env, &pool_key)?;

        if pool.liquidity == 0 {
            return Err(AmmError::InsufficientLiquidity);
        }

        // Record price sample for volatility oracle
        record_price_sample(&env, &pool_key, pool.sqrt_price);

        let fee_bps = pool.dynamic_fee_bps;

        // Determine price target (tick boundary)
        let target_tick = if zero_for_one {
            pool.current_tick - 1
        } else {
            pool.current_tick + 1
        };
        let target_tick = target_tick
            .max(pool::MIN_TICK)
            .min(pool::MAX_TICK);
        let target_sqrt = tick_to_sqrt_price(target_tick)?;

        let step = compute_swap_step(
            pool.sqrt_price,
            target_sqrt,
            pool.liquidity,
            amount_in,
            fee_bps,
        );

        if step.amount_out < min_amount_out {
            return Err(AmmError::SlippageExceeded);
        }

        // Update fee growth globals
        let new_fg_a;
        let new_fg_b;
        if zero_for_one {
            new_fg_a = accumulate_fee_growth(step.fee_paid, pool.liquidity, pool.fee_growth_global_a);
            new_fg_b = pool.fee_growth_global_b;
        } else {
            new_fg_a = pool.fee_growth_global_a;
            new_fg_b = accumulate_fee_growth(step.fee_paid, pool.liquidity, pool.fee_growth_global_b);
        }

        // Cross tick if price boundary was reached
        let new_tick = pool::sqrt_price_to_tick(step.new_sqrt_price);
        if new_tick != pool.current_tick {
            cross_tick(&env, &pool_key, &pool, if zero_for_one { pool.current_tick } else { target_tick });
        }

        pool.sqrt_price = step.new_sqrt_price;
        pool.current_tick = new_tick;
        pool.fee_growth_global_a = new_fg_a;
        pool.fee_growth_global_b = new_fg_b;
        pool.total_volume += step.amount_in;

        write_pool(&env, &pool_key, &pool);

        // Execute token transfers
        let tok_in = token::Client::new(&env, &token_in);
        tok_in.transfer(&caller, &env.current_contract_address(), &step.amount_in);

        let tok_out = token::Client::new(&env, &token_out);
        tok_out.transfer(&env.current_contract_address(), &caller, &step.amount_out);

        env.events().publish(
            (symbol_short!("swap"),),
            (caller, step.amount_in, step.amount_out, step.fee_paid),
        );

        Ok(SwapResult {
            amount_in: step.amount_in,
            amount_out: step.amount_out,
            fee_paid: step.fee_paid,
            new_sqrt_price: step.new_sqrt_price,
            new_tick: new_tick,
        })
    }

    // ── Dynamic fee update ────────────────────────────────────────────────────

    pub fn update_dynamic_fee(
        env: Env,
        token_a: Address,
        token_b: Address,
    ) -> Result<u32, AmmError> {
        require_init(&env)?;
        let pool_key = PoolKey { token_a, token_b };
        let mut pool = require_pool(&env, &pool_key)?;

        let (new_fee, _) = compute_dynamic_fee(&env, &pool_key, pool.fee_tier);
        pool.dynamic_fee_bps = new_fee;
        write_pool(&env, &pool_key, &pool);

        env.events()
            .publish((symbol_short!("fee_upd"),), (pool_key, new_fee));
        Ok(new_fee)
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    pub fn get_pool(env: Env, token_a: Address, token_b: Address) -> Option<PoolState> {
        let pool_key = PoolKey { token_a, token_b };
        read_pool(&env, &pool_key)
    }

    pub fn get_position(env: Env, position_id: u64) -> Option<LpPosition> {
        read_position(&env, position_id)
    }

    pub fn get_lp_positions(env: Env, owner: Address) -> Vec<LpPosition> {
        get_owner_positions(&env, &owner)
    }

    pub fn estimate_il(env: Env, position_id: u64) -> Result<i128, AmmError> {
        let pos = require_position(&env, position_id)?;
        let pool = require_pool(&env, &pos.pool_key.clone())?;
        Ok(estimate_il_bps(&pos, &pool))
    }

    pub fn get_capital_efficiency(env: Env, token_a: Address, token_b: Address) -> Result<i128, AmmError> {
        let pool_key = PoolKey { token_a, token_b };
        let pool = require_pool(&env, &pool_key)?;
        Ok(capital_efficiency_bps(pool.active_liquidity, pool.liquidity))
    }

    // ── Pause / unpause ───────────────────────────────────────────────────────

    pub fn pause(env: Env, admin: Address) -> Result<(), AmmError> {
        admin.require_auth();
        require_init(&env)?;
        ACL::require_permission(&env, &admin, &PERMISSION_PAUSE);
        let mut state = CircuitBreaker::get_state(&env);
        state.pause_level = PauseLevel::Full;
        env.storage().persistent().set(&symbol_short!("cb_state"), &state);
        env.events().publish((symbol_short!("paused"),), ());
        Ok(())
    }

    pub fn unpause(env: Env, admin: Address) -> Result<(), AmmError> {
        admin.require_auth();
        require_init(&env)?;
        ACL::require_permission(&env, &admin, &PERMISSION_UNPAUSE);
        let mut state = CircuitBreaker::get_state(&env);
        state.pause_level = PauseLevel::None;
        env.storage().persistent().set(&symbol_short!("cb_state"), &state);
        env.events().publish((symbol_short!("unpaused"),), ());
        Ok(())
    }

    pub fn get_cb_state(env: Env) -> CircuitBreakerState {
        CircuitBreaker::get_state(&env)
    }

    // ── Governance upgrade ────────────────────────────────────────────────────

    pub fn propose_upgrade(
        env: Env,
        admin: Address,
        new_contract_hash: Symbol,
        description: Symbol,
        approvers: Vec<Address>,
        approval_threshold: u32,
        timelock_delay: u64,
    ) -> Result<u64, AmmError> {
        admin.require_auth();
        require_init(&env)?;
        GovernanceManager::propose_upgrade(
            &env,
            admin,
            new_contract_hash,
            env.current_contract_address(),
            description,
            approval_threshold,
            approvers,
            timelock_delay,
        )
        .map_err(|_| AmmError::Unauthorized)
    }

    pub fn approve_upgrade(env: Env, proposal_id: u64, approver: Address) -> Result<(), AmmError> {
        approver.require_auth();
        require_init(&env)?;
        GovernanceManager::approve_proposal(&env, proposal_id, approver)
            .map_err(|_| AmmError::Unauthorized)
    }

    pub fn execute_upgrade(env: Env, proposal_id: u64, executor: Address) -> Result<(), AmmError> {
        executor.require_auth();
        require_init(&env)?;
        GovernanceManager::execute_proposal(&env, proposal_id, executor)
            .map_err(|_| AmmError::Unauthorized)
    }

    pub fn get_upgrade_proposal(env: Env, proposal_id: u64) -> Result<UpgradeProposal, AmmError> {
        require_init(&env)?;
        GovernanceManager::get_proposal(&env, proposal_id).map_err(|_| AmmError::Unauthorized)
    }

    pub fn reject_upgrade(env: Env, proposal_id: u64, rejector: Address) -> Result<(), AmmError> {
        rejector.require_auth();
        require_init(&env)?;
        GovernanceManager::reject_proposal(&env, proposal_id, rejector)
            .map_err(|_| AmmError::Unauthorized)
    }

    pub fn cancel_upgrade(env: Env, proposal_id: u64, admin: Address) -> Result<(), AmmError> {
        admin.require_auth();
        require_init(&env)?;
        GovernanceManager::cancel_proposal(&env, proposal_id, admin)
            .map_err(|_| AmmError::Unauthorized)
    }
}

#[cfg(test)]
mod test;

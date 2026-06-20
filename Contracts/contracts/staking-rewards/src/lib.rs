#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

mod storage_keys {
    use soroban_sdk::{symbol_short, Symbol};

    pub const ADMIN: Symbol = symbol_short!("admin");
    pub const STAKE_TOKEN: Symbol = symbol_short!("s_token");
    pub const REWARD_TOKEN: Symbol = symbol_short!("r_token");
    pub const POOL_CONFIG: Symbol = symbol_short!("p_cfg");
    pub const USER_STAKE: Symbol = symbol_short!("u_stake");
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    InvalidPool = 5,
    InsufficientBalance = 6,
    StillLocked = 7,
    NothingToClaim = 8,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolConfig {
    pub lockup_seconds: u64,
    pub apy_bps: u32, // APY in basis points (100 = 1%)
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserStake {
    pub amount: i128,
    pub pool_id: u32,
    pub start_timestamp: u64,
    pub last_claim_timestamp: u64,
}

#[contract]
pub struct StakingRewardsContract;

#[contractimpl]
impl StakingRewardsContract {
    /// Initialize the contract with admin and token details
    pub fn initialize(
        env: Env,
        admin: Address,
        staking_token: Address,
        reward_token: Address,
    ) -> Result<(), ContractError> {
        if env.storage().instance().has(&storage_keys::ADMIN) {
            return Err(ContractError::AlreadyInitialized);
        }

        env.storage().instance().set(&storage_keys::ADMIN, &admin);
        env.storage()
            .instance()
            .set(&storage_keys::STAKE_TOKEN, &staking_token);
        env.storage()
            .instance()
            .set(&storage_keys::REWARD_TOKEN, &reward_token);

        // Define default pools: 30, 60, 90 days
        let pools = soroban_sdk::vec![
            &env,
            PoolConfig {
                lockup_seconds: 30 * 24 * 60 * 60,
                apy_bps: 500, // 5%
            },
            PoolConfig {
                lockup_seconds: 60 * 24 * 60 * 60,
                apy_bps: 1000, // 10%
            },
            PoolConfig {
                lockup_seconds: 90 * 24 * 60 * 60,
                apy_bps: 1500, // 15%
            },
        ];
        env.storage()
            .instance()
            .set(&storage_keys::POOL_CONFIG, &pools);

        Ok(())
    }

    /// Stake tokens in a specific pool
    pub fn stake(env: Env, user: Address, amount: i128, pool_id: u32) -> Result<(), ContractError> {
        user.require_auth();

        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        let pools: soroban_sdk::Vec<PoolConfig> = env
            .storage()
            .instance()
            .get(&storage_keys::POOL_CONFIG)
            .ok_or(ContractError::NotInitialized)?;

        if pool_id >= pools.len() {
            return Err(ContractError::InvalidPool);
        }

        let staking_token: Address = env
            .storage()
            .instance()
            .get(&storage_keys::STAKE_TOKEN)
            .unwrap();

        // Transfer tokens to contract
        let client = soroban_sdk::token::Client::new(&env, &staking_token);
        client.transfer(&user, &env.current_contract_address(), &amount);

        let key = (storage_keys::USER_STAKE, user.clone());
        let mut user_stake = env
            .storage()
            .persistent()
            .get::<_, UserStake>(&key)
            .unwrap_or(UserStake {
                amount: 0,
                pool_id,
                start_timestamp: env.ledger().timestamp(),
                last_claim_timestamp: env.ledger().timestamp(),
            });

        // For simplicity, if they already have a stake, they must unstake first or we just update
        // In this implementation, we allow adding to stake but reset the timer for the whole amount
        user_stake.amount += amount;
        user_stake.pool_id = pool_id;
        user_stake.start_timestamp = env.ledger().timestamp();
        user_stake.last_claim_timestamp = env.ledger().timestamp();

        env.storage().persistent().set(&key, &user_stake);

        env.events().publish(
            (symbol_short!("stake"), user),
            (amount, pool_id, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Claim rewards for the user
    pub fn claim(env: Env, user: Address) -> Result<i128, ContractError> {
        user.require_auth();

        let key = (storage_keys::USER_STAKE, user.clone());
        let mut user_stake: UserStake = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::NothingToClaim)?;

        let reward_amount = calculate_rewards(&env, &user_stake)?;
        if reward_amount <= 0 {
            return Err(ContractError::NothingToClaim);
        }

        let reward_token: Address = env
            .storage()
            .instance()
            .get(&storage_keys::REWARD_TOKEN)
            .unwrap();

        let client = soroban_sdk::token::Client::new(&env, &reward_token);
        client.transfer(&env.current_contract_address(), &user, &reward_amount);

        user_stake.last_claim_timestamp = env.ledger().timestamp();
        env.storage().persistent().set(&key, &user_stake);

        env.events().publish(
            (symbol_short!("claim"), user),
            (reward_amount, env.ledger().timestamp()),
        );

        Ok(reward_amount)
    }

    /// Unstake principal and any pending rewards
    pub fn unstake(env: Env, user: Address) -> Result<i128, ContractError> {
        user.require_auth();

        let key = (storage_keys::USER_STAKE, user.clone());
        let user_stake: UserStake = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::NothingToClaim)?;

        let pools: soroban_sdk::Vec<PoolConfig> = env
            .storage()
            .instance()
            .get(&storage_keys::POOL_CONFIG)
            .unwrap();

        let pool = pools.get_unchecked(user_stake.pool_id);
        let now = env.ledger().timestamp();
        let elapsed = now - user_stake.start_timestamp;

        let mut principal_to_return = user_stake.amount;

        // Apply early withdrawal penalty if lockup hasn't expired
        if elapsed < pool.lockup_seconds {
            let penalty_bps = 1000; // 10% penalty
            let penalty_amount = (principal_to_return * penalty_bps as i128) / 10000;
            principal_to_return -= penalty_amount;

            // Penalty stays in the contract (could be sent to a treasury)
        }

        // Pay out any pending rewards before removing stake
        let reward_amount = calculate_rewards(&env, &user_stake).unwrap_or(0);
        if reward_amount > 0 {
            let reward_token: Address = env
                .storage()
                .instance()
                .get(&storage_keys::REWARD_TOKEN)
                .unwrap();
            soroban_sdk::token::Client::new(&env, &reward_token).transfer(
                &env.current_contract_address(),
                &user,
                &reward_amount,
            );
        }

        let staking_token: Address = env
            .storage()
            .instance()
            .get(&storage_keys::STAKE_TOKEN)
            .unwrap();

        soroban_sdk::token::Client::new(&env, &staking_token).transfer(
            &env.current_contract_address(),
            &user,
            &principal_to_return,
        );

        // Emit the claim event once using the already-transferred reward amount.
        if reward_amount > 0 {
            env.events().publish(
                (symbol_short!("claim"), user.clone()),
                (reward_amount, env.ledger().timestamp()),
            );
        }

        // Remove stake
        env.storage().persistent().remove(&key);

        env.events().publish(
            (symbol_short!("unstake"), user),
            (principal_to_return, reward_amount, env.ledger().timestamp()),
        );

        Ok(principal_to_return)
    }

    /// Re-stake pending rewards (Auto-compounding)
    pub fn compound(env: Env, user: Address) -> Result<(), ContractError> {
        user.require_auth();

        let key = (storage_keys::USER_STAKE, user.clone());
        let mut user_stake: UserStake = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::NothingToClaim)?;

        let reward_amount = calculate_rewards(&env, &user_stake)?;
        if reward_amount <= 0 {
            return Err(ContractError::NothingToClaim);
        }

        // Logic check: reward token must be the same as staking token for auto-compound
        let staking_token: Address = env
            .storage()
            .instance()
            .get(&storage_keys::STAKE_TOKEN)
            .unwrap();
        let reward_token: Address = env
            .storage()
            .instance()
            .get(&storage_keys::REWARD_TOKEN)
            .unwrap();

        if staking_token != reward_token {
            return Err(ContractError::Unauthorized); // Or a more specific error
        }

        user_stake.amount += reward_amount;
        user_stake.last_claim_timestamp = env.ledger().timestamp();
        env.storage().persistent().set(&key, &user_stake);

        env.events().publish(
            (symbol_short!("compound"), user),
            (reward_amount, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Get user's current stake info
    pub fn get_stake(env: Env, user: Address) -> Option<UserStake> {
        let key = (storage_keys::USER_STAKE, user);
        env.storage().persistent().get(&key)
    }

    /// Get pending rewards for a user
    pub fn get_pending_rewards(env: Env, user: Address) -> i128 {
        let key = (storage_keys::USER_STAKE, user);
        if let Some(user_stake) = env.storage().persistent().get::<_, UserStake>(&key) {
            return calculate_rewards(&env, &user_stake).unwrap_or(0);
        }
        0
    }
}

fn calculate_rewards(env: &Env, user_stake: &UserStake) -> Result<i128, ContractError> {
    let pools: soroban_sdk::Vec<PoolConfig> = env
        .storage()
        .instance()
        .get(&storage_keys::POOL_CONFIG)
        .ok_or(ContractError::NotInitialized)?;

    if user_stake.pool_id >= pools.len() {
        return Err(ContractError::InvalidPool);
    }

    let pool = pools.get(user_stake.pool_id).unwrap();
    let now = env.ledger().timestamp();
    let elapsed_seconds = now - user_stake.last_claim_timestamp;

    if elapsed_seconds == 0 {
        return Ok(0);
    }

    // Reward = Principal * APY * (elapsed / seconds_in_year)
    // APY is in basis points
    let seconds_in_year: u64 = 365 * 24 * 60 * 60;

    // Scale precision for calculation: (amount * apy * seconds) / (10000 * seconds_in_year)
    // Using i128 to prevent overflow in Intermediate calculation
    let reward = (user_stake.amount * pool.apy_bps as i128 * elapsed_seconds as i128)
        / (10000i128 * seconds_in_year as i128);

    Ok(reward)
}

#[cfg(test)]
mod test;

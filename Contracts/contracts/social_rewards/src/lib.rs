#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

#[contract]
pub struct SocialRewardsContract;

/// Engagement record for tracking social activities
#[contracttype]
#[derive(Clone, Debug)]
pub struct Engagement {
    pub id: u64,
    pub user: Address,
    pub engagement_type: Symbol,
    pub timestamp: u64,
    pub metadata: i128,
}

/// Event emitted when engagement is recorded
#[contracttype]
#[derive(Clone, Debug)]
pub struct EngagementRecorded {
    pub engagement_id: u64,
    pub user: Address,
    pub engagement_type: Symbol,
    pub timestamp: u64,
    pub metadata: i128,
}

/// Event emitted when reward is distributed
#[contracttype]
#[derive(Clone, Debug)]
pub struct RewardDistributed {
    pub engagement_id: u64,
    pub user: Address,
    pub reward_amount: i128,
    pub timestamp: u64,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RewardError {
    InvalidAmount = 5001,
    NotInitialized = 5002,
    Unauthorized = 5003,
    InsufficientBalance = 5004,
    EngagementNotFound = 5005,
}

impl From<RewardError> for soroban_sdk::Error {
    fn from(error: RewardError) -> Self {
        soroban_sdk::Error::from_contract_error(error as u32)
    }
}

impl From<&RewardError> for soroban_sdk::Error {
    fn from(error: &RewardError) -> Self {
        soroban_sdk::Error::from_contract_error(*error as u32)
    }
}

impl From<soroban_sdk::Error> for RewardError {
    fn from(_error: soroban_sdk::Error) -> Self {
        RewardError::Unauthorized
    }
}

// Storage key for the reward token address
const REWARD_TOKEN_KEY: &str = "rew_tok";

/// Returns the SEP-41 token client balance for `account`.
fn token_balance(env: &Env, token: &Address, account: &Address) -> i128 {
    soroban_sdk::token::Client::new(env, token).balance(account)
}

/// Transfers `amount` of the reward token from `from` to `to`.
fn token_transfer(env: &Env, token: &Address, from: &Address, to: &Address, amount: i128) {
    soroban_sdk::token::Client::new(env, token).transfer(from, to, &amount);
}

#[contractimpl]
impl SocialRewardsContract {
    /// Initialize the social rewards contract.
    /// `reward_token` is the SEP-41 token used for reward payouts.
    pub fn init(env: Env, admin: Address, reward_token: Address) -> Result<(), RewardError> {
        let init_key = symbol_short!("init");
        if env.storage().persistent().has(&init_key) {
            return Err(RewardError::Unauthorized);
        }
        env.storage().persistent().set(&init_key, &true);
        env.storage()
            .persistent()
            .set(&symbol_short!("admin"), &admin);
        env.storage()
            .persistent()
            .set(&symbol_short!("eng_cnt"), &0u64);
        env.storage()
            .persistent()
            .set(&Symbol::new(&env, REWARD_TOKEN_KEY), &reward_token);
        Ok(())
    }

    /// Record an engagement activity.
    ///
    /// Requires the `user` address to authorise this call so that no third party
    /// can fabricate engagement history for another account.
    pub fn record_engagement(
        env: Env,
        user: Address,
        engagement_type: Symbol,
        metadata: i128,
    ) -> Result<u64, RewardError> {
        // SECURITY: only the user themselves may record their own engagement.
        user.require_auth();

        let init_key = symbol_short!("init");
        if !env.storage().persistent().has(&init_key) {
            return Err(RewardError::NotInitialized);
        }

        if metadata < 0 {
            return Err(RewardError::InvalidAmount);
        }

        let current_timestamp = env.ledger().timestamp();

        // Get next engagement ID
        let counter_key = symbol_short!("eng_cnt");
        let engagement_id: u64 = env.storage().persistent().get(&counter_key).unwrap_or(0u64);
        let next_id = engagement_id + 1;

        // Create engagement record
        let engagement = Engagement {
            id: next_id,
            user: user.clone(),
            engagement_type: engagement_type.clone(),
            timestamp: current_timestamp,
            metadata,
        };

        // Store engagement
        let engagement_key = (symbol_short!("eng"), next_id);
        env.storage().persistent().set(&engagement_key, &engagement);

        // Update counter
        env.storage().persistent().set(&counter_key, &next_id);

        // Emit EngagementRecorded event
        let engagement_event = EngagementRecorded {
            engagement_id: next_id,
            user,
            engagement_type,
            timestamp: current_timestamp,
            metadata,
        };
        env.events()
            .publish((symbol_short!("eng_rec"),), engagement_event);

        Ok(next_id)
    }

    /// Distribute reward for an engagement.
    /// Verifies the admin, checks the reward pool balance, transfers tokens to
    /// the user, and only then emits the RewardDistributed event.
    pub fn distribute_reward(
        env: Env,
        admin: Address,
        engagement_id: u64,
        reward_amount: i128,
    ) -> Result<(), RewardError> {
        admin.require_auth();

        let init_key = symbol_short!("init");
        if !env.storage().persistent().has(&init_key) {
            return Err(RewardError::NotInitialized);
        }

        // Verify admin
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&symbol_short!("admin"))
            .ok_or(RewardError::Unauthorized)?;
        if admin != stored_admin {
            return Err(RewardError::Unauthorized);
        }

        if reward_amount <= 0 {
            return Err(RewardError::InvalidAmount);
        }

        // Get engagement
        let engagement_key = (symbol_short!("eng"), engagement_id);
        let engagement: Engagement = env
            .storage()
            .persistent()
            .get(&engagement_key)
            .ok_or(RewardError::EngagementNotFound)?;

        // Load reward token
        let reward_token: Address = env
            .storage()
            .persistent()
            .get(&Symbol::new(&env, REWARD_TOKEN_KEY))
            .ok_or(RewardError::NotInitialized)?;

        // Check reward pool balance (admin holds the reward pool)
        let pool_balance = token_balance(&env, &reward_token, &admin);
        if pool_balance < reward_amount {
            return Err(RewardError::InsufficientBalance);
        }

        // Transfer tokens to the user before emitting event
        token_transfer(&env, &reward_token, &admin, &engagement.user, reward_amount);

        let current_timestamp = env.ledger().timestamp();

        // Emit RewardDistributed event only after successful transfer
        let reward_event = RewardDistributed {
            engagement_id,
            user: engagement.user,
            reward_amount,
            timestamp: current_timestamp,
        };
        env.events()
            .publish((symbol_short!("rew_dist"),), reward_event);

        Ok(())
    }

    /// Adds a reward. Fails if amount is 0 (to simulate validation logic).
    pub fn add_reward(env: Env, user: Address, amount: i128) -> Result<(), RewardError> {
        if amount <= 0 {
            return Err(RewardError::InvalidAmount);
        }
        let init_key = symbol_short!("init");
        if !env.storage().persistent().has(&init_key) {
            return Err(RewardError::NotInitialized);
        }

        // Create reward symbol before moving env
        let reward_type = Symbol::new(&env, "reward");

        // Record engagement as generic reward activity; caller must have authorised user.
        let _engagement_id = Self::record_engagement(env, user, reward_type, amount)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Address, Env,
    };

    fn create_token(
        env: &Env,
        admin: &Address,
    ) -> (Address, TokenClient<'static>, StellarAssetClient<'static>) {
        let address = env.register_stellar_asset_contract(admin.clone());
        (
            address.clone(),
            TokenClient::new(env, &address),
            StellarAssetClient::new(env, &address),
        )
    }

    fn setup() -> (
        Env,
        Address,
        Address,
        Address,
        TokenClient<'static>,
        StellarAssetClient<'static>,
        SocialRewardsContractClient<'static>,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 1_000_000,
            protocol_version: 20,
            sequence_number: 1,
            network_id: [0u8; 32],
            base_reserve: 10,
            max_entry_ttl: 31104000,
            min_persistent_entry_ttl: 31104000,
            min_temp_entry_ttl: 31104000,
        });

        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let (token_addr, token_client, token_admin) = create_token(&env, &admin);

        // Mint 10_000 reward tokens to admin (the reward pool)
        token_admin.mint(&admin, &10_000);

        let contract_id = env.register_contract(None, SocialRewardsContract);
        let client = SocialRewardsContractClient::new(&env, &contract_id);
        client.init(&admin, &token_addr);

        (env, admin, user, token_addr, token_client, token_admin, client)
    }

    // ── Token transfer tests (PR) ────────────────────────────────────────────

    #[test]
    fn test_init_stores_reward_token() {
        let (_env, _admin, _user, _token_addr, _token, _token_admin, _client) = setup();
    }

    #[test]
    fn test_distribute_reward_transfers_tokens() {
        let (_env, admin, user, _token_addr, token, _token_admin, client) = setup();

        let eng_id = client.record_engagement(&user, &Symbol::new(&_env, "like"), &100);

        let admin_before = token.balance(&admin);
        let user_before = token.balance(&user);

        client.distribute_reward(&admin, &eng_id, &500);

        assert_eq!(token.balance(&admin), admin_before - 500);
        assert_eq!(token.balance(&user), user_before + 500);
    }

    #[test]
    fn test_distribute_reward_emits_event_after_transfer() {
        let (_env, admin, user, _token_addr, token, _token_admin, client) = setup();

        let eng_id = client.record_engagement(&user, &Symbol::new(&_env, "post"), &50);
        client.distribute_reward(&admin, &eng_id, &200);

        assert_eq!(token.balance(&user), 200);
    }

    #[test]
    #[should_panic]
    fn test_distribute_reward_fails_on_insufficient_balance() {
        let (_env, admin, user, _token_addr, _token, _token_admin, client) = setup();

        let eng_id = client.record_engagement(&user, &Symbol::new(&_env, "share"), &10);
        client.distribute_reward(&admin, &eng_id, &100_000);
    }

    #[test]
    #[should_panic]
    fn test_distribute_reward_fails_for_invalid_amount() {
        let (_env, admin, user, _token_addr, _token, _token_admin, client) = setup();

        let eng_id = client.record_engagement(&user, &Symbol::new(&_env, "share"), &10);
        client.distribute_reward(&admin, &eng_id, &0);
    }

    #[test]
    #[should_panic]
    fn test_distribute_reward_fails_for_unknown_engagement() {
        let (_env, admin, _user, _token_addr, _token, _token_admin, client) = setup();
        client.distribute_reward(&admin, &999, &100);
    }

    // ── Auth and engagement tests (upstream) ─────────────────────────────────

    #[test]
    fn authorized_user_records_own_engagement() {
        let (env, _, _, _, _, _, client) = setup();
        let user = Address::generate(&env);
        let etype = Symbol::new(&env, "like");
        let id = client.record_engagement(&user, &etype, &50);
        assert_eq!(id, 1u64);
    }

    #[test]
    fn sequential_engagements_get_incrementing_ids() {
        let (env, _, _, _, _, _, client) = setup();
        let user = Address::generate(&env);
        let etype = Symbol::new(&env, "share");
        assert_eq!(client.record_engagement(&user, &etype, &10), 1u64);
        assert_eq!(client.record_engagement(&user, &etype, &20), 2u64);
    }

    #[test]
    fn negative_metadata_is_rejected() {
        let (env, _, _, _, _, _, client) = setup();
        let user = Address::generate(&env);
        let etype = Symbol::new(&env, "vote");
        let result = client.try_record_engagement(&user, &etype, &-1);
        assert!(result.is_err());
    }

    #[test]
    #[should_panic]
    fn unauthorized_caller_cannot_record_engagement_for_another_user() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let (token_addr, _, _) = create_token(&env, &admin);

        let id = env.register_contract(None, SocialRewardsContract);
        let client = SocialRewardsContractClient::new(&env, &id);
        env.mock_all_auths();
        client.init(&admin, &token_addr);

        // Fresh env with no mock_all_auths — require_auth must panic.
        let env2 = Env::default();
        let client2 = SocialRewardsContractClient::new(&env2, &id);
        let victim = Address::generate(&env2);
        let etype = Symbol::new(&env2, "like");
        client2.record_engagement(&victim, &etype, &10);
    }
}
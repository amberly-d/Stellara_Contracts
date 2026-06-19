#![cfg(test)]

use super::*;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Vec,
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

#[test]
fn test_staking_workflow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    // Create tokens
    let (staking_token_address, staking_token, staking_token_admin) = create_token(&env, &admin);
    let (reward_token_address, reward_token, reward_token_admin) = create_token(&env, &admin);

    // Register contract
    let contract_id = env.register_contract(None, StakingRewardsContract);
    let client = StakingRewardsContractClient::new(&env, &contract_id);

    // Initialize
    client.initialize(&admin, &staking_token_address, &reward_token_address);

    // Mint tokens to user
    staking_token_admin.mint(&user, &10000);
    reward_token_admin.mint(&contract_id, &100000); // Fund the contract with rewards

    // User stakes 1000 in pool 0 (30 days, 5% APY)
    client.stake(&user, &1000, &0);

    assert_eq!(staking_token.balance(&user), 9000);
    assert_eq!(staking_token.balance(&contract_id), 1000);

    // Jump time: 15 days (1/2 of 30 days)
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 15 * 24 * 60 * 60,
        protocol_version: 20,
        sequence_number: 10,
        network_id: [0u8; 32],
        base_reserve: 10,
        max_entry_ttl: 31104000,
        min_persistent_entry_ttl: 31104000,
        min_temp_entry_ttl: 31104000,
    });

    // Check pending rewards
    // 1000 * 0.05 * (15 / 365) = approx 2.05... truncated to 2
    let pending = client.get_pending_rewards(&user);
    assert!(pending > 0);
    assert_eq!(pending, 2);

    // Jump to 31 days (Expired lockup)
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 31 * 24 * 60 * 60,
        protocol_version: 20,
        sequence_number: 20,
        network_id: [0u8; 32],
        base_reserve: 10,
        max_entry_ttl: 31104000,
        min_persistent_entry_ttl: 31104000,
        min_temp_entry_ttl: 31104000,
    });

    // Claim rewards
    let claimed = client.claim(&user);
    assert_eq!(claimed, 4); // 1000 * 0.05 * (31 / 365) = 4.24...
    assert_eq!(reward_token.balance(&user), 4);

    // Unstake
    let returned = client.unstake(&user);
    assert_eq!(returned, 1000); // No penalty after 30 days
    assert_eq!(staking_token.balance(&user), 10000);
}

#[test]
fn test_early_withdrawal_penalty() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let (staking_token_address, _staking_token, staking_token_admin) = create_token(&env, &admin);
    let (reward_token_address, _reward_token, _reward_token_admin) = create_token(&env, &admin);

    let contract_id = env.register_contract(None, StakingRewardsContract);
    let client = StakingRewardsContractClient::new(&env, &contract_id);

    client.initialize(&admin, &staking_token_address, &reward_token_address);

    staking_token_admin.mint(&user, &1000);
    client.stake(&user, &1000, &0); // 30 day pool

    // Jump 1 day (Early)
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 1 * 24 * 60 * 60,
        protocol_version: 20,
        sequence_number: 10,
        network_id: [0u8; 32],
        base_reserve: 10,
        max_entry_ttl: 31104000,
        min_persistent_entry_ttl: 31104000,
        min_temp_entry_ttl: 31104000,
    });

    // Unstake early (10% penalty)
    let returned = client.unstake(&user);
    assert_eq!(returned, 900); // 1000 - 100
}

#[test]
fn test_compounding() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    // In compounding test, staking token and reward token MUST be the same
    let (token_address, token, token_admin) = create_token(&env, &admin);

    let contract_id = env.register_contract(None, StakingRewardsContract);
    let client = StakingRewardsContractClient::new(&env, &contract_id);

    client.initialize(&admin, &token_address, &token_address);

    token_admin.mint(&user, &1000);
    client.stake(&user, &1000, &2); // 90 day pool (15% APY)

    // Jump 180 days (half year)
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 180 * 24 * 60 * 60,
        protocol_version: 20,
        sequence_number: 10,
        network_id: [0u8; 32],
        base_reserve: 10,
        max_entry_ttl: 31104000,
        min_persistent_entry_ttl: 31104000,
        min_temp_entry_ttl: 31104000,
    });

    // Pending: 1000 * 0.15 * (180 / 365) = 150 * 0.493... = 73.97 -> 73
    let pending = client.get_pending_rewards(&user);
    assert_eq!(pending, 73);

    // Compound
    client.compound(&user);

    let stake_info = client.get_stake(&user).unwrap();
    assert_eq!(stake_info.amount, 1073);
    assert_eq!(token.balance(&user), 0);
}

#[test]
fn test_unstake_pays_pending_rewards() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let (staking_token_address, staking_token, staking_token_admin) = create_token(&env, &admin);
    let (reward_token_address, reward_token, reward_token_admin) = create_token(&env, &admin);

    let contract_id = env.register_contract(None, StakingRewardsContract);
    let client = StakingRewardsContractClient::new(&env, &contract_id);

    client.initialize(&admin, &staking_token_address, &reward_token_address);

    staking_token_admin.mint(&user, &1000);
    reward_token_admin.mint(&contract_id, &100000);

    client.stake(&user, &1000, &0); // 30-day pool, 5% APY

    // Jump 31 days (past lockup) — expected reward: 1000 * 0.05 * (31/365) = 4
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 31 * 24 * 60 * 60,
        protocol_version: 20,
        sequence_number: 10,
        network_id: [0u8; 32],
        base_reserve: 10,
        max_entry_ttl: 31104000,
        min_persistent_entry_ttl: 31104000,
        min_temp_entry_ttl: 31104000,
    });

    let pending = client.get_pending_rewards(&user);
    assert_eq!(pending, 4);

    // Unstake without claiming first — should receive both principal and rewards
    let returned = client.unstake(&user);
    assert_eq!(returned, 1000); // full principal, no penalty
    assert_eq!(staking_token.balance(&user), 1000);
    assert_eq!(reward_token.balance(&user), 4); // rewards paid automatically
}

#[test]
fn test_unstake_early_pays_pending_rewards() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let (staking_token_address, staking_token, staking_token_admin) = create_token(&env, &admin);
    let (reward_token_address, reward_token, reward_token_admin) = create_token(&env, &admin);

    let contract_id = env.register_contract(None, StakingRewardsContract);
    let client = StakingRewardsContractClient::new(&env, &contract_id);

    client.initialize(&admin, &staking_token_address, &reward_token_address);

    staking_token_admin.mint(&user, &1000);
    reward_token_admin.mint(&contract_id, &100000);

    client.stake(&user, &1000, &0); // 30-day pool

    // Jump 15 days (early withdrawal) — expected reward: 1000 * 0.05 * (15/365) = 2
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 15 * 24 * 60 * 60,
        protocol_version: 20,
        sequence_number: 10,
        network_id: [0u8; 32],
        base_reserve: 10,
        max_entry_ttl: 31104000,
        min_persistent_entry_ttl: 31104000,
        min_temp_entry_ttl: 31104000,
    });

    // Unstake early: 10% penalty on principal, but rewards still paid
    let returned = client.unstake(&user);
    assert_eq!(returned, 900); // 1000 - 10% penalty
    assert_eq!(staking_token.balance(&user), 900);
    assert_eq!(reward_token.balance(&user), 2); // accrued rewards still paid
}

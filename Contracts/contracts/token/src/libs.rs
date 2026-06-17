use soroban_sdk::{contract, contractimpl, symbol_short, Env, Address, String};

mod admin;
mod storage;

/// SEP-41 compliant token contract.
///
/// Exposes the full standard interface expected by soroban_sdk::token::Client
/// across all contracts in the Stellara workspace.
#[contract]
pub struct TokenContract;

#[contractimpl]
impl TokenContract {

    // =========================================================================
    // Initialisation
    // =========================================================================

    pub fn initialize(env: Env, admin: Address, name: String, symbol: String, decimals: u32) {
        if storage::has_admin(&env) {
            panic!("Already initialized");
        }
        admin.require_auth();
        storage::set_admin(&env, &admin);

        env.storage().instance().set(&symbol_short!("NAME"), &name);
        env.storage().instance().set(&symbol_short!("SYMBOL"), &symbol);
        env.storage().instance().set(&symbol_short!("DECIMALS"), &decimals);
    }

    // =========================================================================
    // Metadata
    // =========================================================================

    pub fn name(env: Env) -> String {
        env.storage().instance().get(&symbol_short!("NAME")).expect("Not initialized")
    }

    pub fn symbol(env: Env) -> String {
        env.storage().instance().get(&symbol_short!("SYMBOL")).expect("Not initialized")
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&symbol_short!("DECIMALS")).expect("Not initialized")
    }

    // =========================================================================
    // Core SEP-41 / ERC-20 interface
    // =========================================================================

    pub fn total_supply(env: Env) -> i128 {
        storage::get_total_supply(&env)
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        storage::balance_of(&env, &id)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        assert!(amount > 0, "Amount must be positive");

        let from_balance = storage::balance_of(&env, &from);
        assert!(from_balance >= amount, "Insufficient balance");

        let to_balance = storage::balance_of(&env, &to);

        storage::set_balance(&env, &from, &(from_balance - amount));
        storage::set_balance(&env, &to, &(to_balance + amount));

        env.events().publish(
            (symbol_short!("transfer"), from.clone(), to.clone()),
            amount,
        );
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        assert!(amount > 0, "Amount must be positive");

        // Consume allowance
        let allowance = Self::allowance(env.clone(), from.clone(), spender.clone());
        assert!(allowance >= amount, "Allowance exceeded");
        Self::_set_allowance(&env, &from, &spender, allowance - amount);

        let from_balance = storage::balance_of(&env, &from);
        assert!(from_balance >= amount, "Insufficient balance");
        let to_balance = storage::balance_of(&env, &to);

        storage::set_balance(&env, &from, &(from_balance - amount));
        storage::set_balance(&env, &to, &(to_balance + amount));

        env.events().publish(
            (symbol_short!("transfer"), from.clone(), to.clone()),
            amount,
        );
    }

    pub fn approve(env: Env, from: Address, spender: Address, amount: i128, _expiration_ledger: u32) {
        from.require_auth();
        assert!(amount >= 0, "Amount must be non-negative");
        Self::_set_allowance(&env, &from, &spender, amount);
        env.events().publish(
            (symbol_short!("approve"), from.clone(), spender.clone()),
            amount,
        );
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        let key = (symbol_short!("ALLOW"), from, spender);
        env.storage().temporary().get(&key).unwrap_or(0)
    }

    // =========================================================================
    // Mint / Burn (admin-only)
    // =========================================================================

    pub fn mint(env: Env, to: Address, amount: i128) {
        admin::require_admin(&env);
        assert!(amount > 0, "Amount must be positive");

        let balance = storage::balance_of(&env, &to);
        let new_balance = balance.checked_add(amount).expect("Overflow");
        storage::set_balance(&env, &to, &new_balance);

        let supply = storage::get_total_supply(&env);
        storage::set_total_supply(&env, &(supply.checked_add(amount).expect("Supply overflow")));

        env.events().publish(
            (symbol_short!("mint"), to.clone()),
            amount,
        );
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        assert!(amount > 0, "Amount must be positive");

        let balance = storage::balance_of(&env, &from);
        assert!(balance >= amount, "Insufficient balance");
        storage::set_balance(&env, &from, &(balance - amount));

        let supply = storage::get_total_supply(&env);
        storage::set_total_supply(&env, &(supply - amount));

        env.events().publish(
            (symbol_short!("burn"), from.clone()),
            amount,
        );
    }

    pub fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        assert!(amount > 0, "Amount must be positive");

        let allowance = Self::allowance(env.clone(), from.clone(), spender.clone());
        assert!(allowance >= amount, "Allowance exceeded");
        Self::_set_allowance(&env, &from, &spender, allowance - amount);

        let balance = storage::balance_of(&env, &from);
        assert!(balance >= amount, "Insufficient balance");
        storage::set_balance(&env, &from, &(balance - amount));

        let supply = storage::get_total_supply(&env);
        storage::set_total_supply(&env, &(supply - amount));

        env.events().publish(
            (symbol_short!("burn"), from.clone()),
            amount,
        );
    }

    // =========================================================================
    // Admin helpers
    // =========================================================================

    pub fn set_admin(env: Env, new_admin: Address) {
        admin::require_admin(&env);
        new_admin.require_auth();
        storage::set_admin(&env, &new_admin);
    }

    // =========================================================================
    // Internal helpers
    // =========================================================================

    fn _set_allowance(env: &Env, from: &Address, spender: &Address, amount: i128) {
        let key = (symbol_short!("ALLOW"), from.clone(), spender.clone());
        env.storage().temporary().set(&key, &amount);
    }
}

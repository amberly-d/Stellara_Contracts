use soroban_sdk::{testutils::Address as _, Env, String};
use token::TokenContract;

fn setup(env: &Env) -> soroban_sdk::Address {
    let admin  = soroban_sdk::Address::generate(env);
    let name   = String::from_str(env, "Stellara Token");
    let symbol = String::from_str(env, "STA");
    TokenContract::initialize(env.clone(), admin.clone(), name, symbol, 7);
    admin
}

#[test]
#[should_panic(expected = "Overflow")]
fn mint_overflow_attack() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = setup(&env);
    // First mint to i128::MAX
    TokenContract::mint(env.clone(), admin.clone(), i128::MAX);
    // Second mint should overflow
    TokenContract::mint(env, admin, 1);
}

#[test]
#[should_panic(expected = "Insufficient balance")]
fn transfer_beyond_balance_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = setup(&env);
    let other = soroban_sdk::Address::generate(&env);
    TokenContract::mint(env.clone(), admin.clone(), 100);
    TokenContract::transfer(env, admin, other, 101);
}

#[test]
#[should_panic(expected = "Allowance exceeded")]
fn transfer_from_beyond_allowance_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let admin   = setup(&env);
    let spender = soroban_sdk::Address::generate(&env);
    let to      = soroban_sdk::Address::generate(&env);
    TokenContract::mint(env.clone(), admin.clone(), 1000);
    TokenContract::approve(env.clone(), admin.clone(), spender.clone(), 50, 0);
    TokenContract::transfer_from(env, spender, admin, to, 51);
}

#[test]
#[should_panic(expected = "Already initialized")]
fn double_initialize_blocked() {
    let env  = Env::default();
    env.mock_all_auths();
    let admin  = soroban_sdk::Address::generate(&env);
    let name   = String::from_str(&env, "Stellara Token");
    let symbol = String::from_str(&env, "STA");
    TokenContract::initialize(env.clone(), admin.clone(), name.clone(), symbol.clone(), 7);
    TokenContract::initialize(env, admin, name, symbol, 7);
}

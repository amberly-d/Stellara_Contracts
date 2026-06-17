use soroban_sdk::{testutils::Address as _, Env, String};
use token::TokenContract;

fn setup(env: &Env) -> (soroban_sdk::Address, soroban_sdk::Address) {
    let admin = soroban_sdk::Address::generate(env);
    let name   = String::from_str(env, "Stellara Token");
    let symbol = String::from_str(env, "STA");
    TokenContract::initialize(env.clone(), admin.clone(), name, symbol, 7);
    let other = soroban_sdk::Address::generate(env);
    (admin, other)
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn non_admin_cannot_mint() {
    let env = Env::default();
    let (_admin, attacker) = setup(&env);
    // attacker tries mint — require_admin will panic "Unauthorized"
    TokenContract::mint(env, attacker, 100);
}

#[test]
fn admin_can_mint_and_balance_updates() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, recipient) = setup(&env);
    TokenContract::mint(env.clone(), recipient.clone(), 500);
    assert_eq!(TokenContract::balance(env.clone(), recipient), 500);
    assert_eq!(TokenContract::total_supply(env), 500);
}

#[test]
fn transfer_moves_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, recipient) = setup(&env);
    let other = soroban_sdk::Address::generate(&env);
    TokenContract::mint(env.clone(), recipient.clone(), 1000);
    TokenContract::transfer(env.clone(), recipient.clone(), other.clone(), 300);
    assert_eq!(TokenContract::balance(env.clone(), recipient), 700);
    assert_eq!(TokenContract::balance(env.clone(), other), 300);
}

#[test]
fn burn_reduces_supply() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, holder) = setup(&env);
    TokenContract::mint(env.clone(), holder.clone(), 1000);
    TokenContract::burn(env.clone(), holder.clone(), 400);
    assert_eq!(TokenContract::balance(env.clone(), holder), 600);
    assert_eq!(TokenContract::total_supply(env), 600);
}

#![cfg(test)]

extern crate std;

use academy_rewards::AcademyRewardsContract;
use messaging::UpgradeableMessagingContract;
use shared::circuit_breaker::CircuitBreakerConfig;
use shared::governance::ProposalStatus;
use social_rewards::SocialRewardsContract;
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Events, Ledger},
    token, Address, Env, String, Symbol, Vec,
};
use trading::UpgradeableTradingContract;

// ─────────────────────────────────────────────────────────────────────────────
// Mock token
// ─────────────────────────────────────────────────────────────────────────────

#[contract]
pub struct MockTokenContract;

#[contracttype]
#[derive(Clone)]
pub enum TokenDataKey {
    Balance(Address),
}

#[contractimpl]
impl MockTokenContract {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let current = Self::balance(env.clone(), to.clone());
        let updated = current.checked_add(amount).expect("overflow");
        env.storage()
            .persistent()
            .set(&TokenDataKey::Balance(to), &updated);
    }
    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&TokenDataKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let from_balance = Self::balance(env.clone(), from.clone());
        if from_balance < amount {
            panic!("insufficient balance")
        }
        let to_balance = Self::balance(env.clone(), to.clone());
        env.storage()
            .persistent()
            .set(&TokenDataKey::Balance(from), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&TokenDataKey::Balance(to), &(to_balance + amount));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: assert at least one published event matches the given topic symbol.
//
// soroban_sdk::Env::events().all() returns:
//   Vec<(Address, soroban_sdk::Vec<Val>, Val)>
//             contract  topics           data
//
// Long symbols (>9 chars) are heap-allocated in the Soroban test VM — two
// Symbol::new calls for the same string produce different object handles.
// The only stable comparison is to convert both sides to their string
// representation via soroban_sdk::Symbol::to_string.
// ─────────────────────────────────────────────────────────────────────────────
fn assert_event_emitted(env: &Env, expected_topic: Symbol) {
    use soroban_sdk::TryFromVal;
    let expected_str = expected_topic.to_string();

    let found = env.events().all().iter().any(|(_, topics, _)| {
        topics.iter().any(|raw_val| {
            Symbol::try_from_val(env, &raw_val)
                .map(|s| s.to_string() == expected_str)
                .unwrap_or(false)
        })
    });

    assert!(
        found,
        "Expected event with topic \"{}\" was not emitted",
        expected_str
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_academy_rewards_trigger_social_rewards() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 1000);
    env.mock_all_auths();

    let academy_id = env.register_contract(None, AcademyRewardsContract);
    let academy = academy_rewards::AcademyRewardsContractClient::new(&env, &academy_id);

    let social_id = env.register_contract(None, SocialRewardsContract);
    let social = social_rewards::SocialRewardsContractClient::new(&env, &social_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let cb_config = CircuitBreakerConfig {
        max_volume_per_period: 10_000_000,
        max_tx_count_per_period: 100,
        period_duration: 3600,
    };

    academy.initialize(&admin, &cb_config);
    social.init(&admin);

    academy.create_badge_type(
        &admin,
        &1u32,
        &String::from_str(&env, "Gold"),
        &500u32,
        &5u32,
        &0u64,
    );
    academy.mint_badge(&admin, &user, &1u32);

    let discount = academy.redeem_badge(&user, &String::from_str(&env, "tx-1"));
    assert_eq!(discount, 500);

    social.add_reward(&user, &(discount as i128));
    social.record_engagement(
        &user,
        &Symbol::new(&env, "badge"),
        &(discount as i128),
    );

    let record = academy.get_redemption_history(&user, &0u32).unwrap();
    assert_eq!(record.discount_applied, 500);

    // ── Event assertions ─────────────────────────────────────────────────────
    // academy-rewards uses Symbol::new (not symbol_short!) for its topics
    assert_event_emitted(&env, Symbol::new(&env, "badge_minted"));
    assert_event_emitted(&env, Symbol::new(&env, "badge_redeemed"));
    // social_rewards emits "eng_rec" on record_engagement; add_reward is silent
    assert_event_emitted(&env, Symbol::new(&env, "eng_rec"));
}

#[test]
fn test_trading_interacts_with_fee_distribution() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 1000);
    env.mock_all_auths();

    let token_id = env.register_contract(None, MockTokenContract);
    let token_admin = MockTokenContractClient::new(&env, &token_id);

    let trading_id = env.register_contract(None, UpgradeableTradingContract);
    let trading = trading::UpgradeableTradingContractClient::new(&env, &trading_id);

    let admin        = Address::generate(&env);
    let approver     = Address::generate(&env);
    let executor     = Address::generate(&env);
    let trader       = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    let mut approvers = Vec::new(&env);
    approvers.push_back(approver);

    let cb_config = CircuitBreakerConfig {
        max_volume_per_period: 10_000_000,
        max_tx_count_per_period: 100,
        period_duration: 3600,
    };

    trading.init(&admin, &approvers, &executor, &cb_config);
    token_admin.mint(&trader, &1000i128);

    let fee_before_trader    = token::Client::new(&env, &token_id).balance(&trader);
    let fee_before_recipient = token::Client::new(&env, &token_id).balance(&fee_recipient);

    let trade_id = trading.trade(
        &trader,
        &Symbol::new(&env, "XLMUSD"),
        &250i128,
        &100i128,
        &true,
        &token_id,
        &25i128,
        &fee_recipient,
    );

    assert_eq!(trade_id, 1);

    let fee_after_trader    = token::Client::new(&env, &token_id).balance(&trader);
    let fee_after_recipient = token::Client::new(&env, &token_id).balance(&fee_recipient);

    assert_eq!(fee_before_trader - fee_after_trader, 25);
    assert_eq!(fee_after_recipient - fee_before_recipient, 25);

    let stats = trading.get_stats();
    assert_eq!(stats.total_trades, 1);
    assert_eq!(stats.total_volume, 250);

    // ── Event assertions ─────────────────────────────────────────────────────
    assert_event_emitted(&env, Symbol::new(&env, "trade"));
    assert_event_emitted(&env, Symbol::new(&env, "fee_col"));
}

#[test]
fn test_messaging_notifications_from_other_contract_flows() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 1000);
    env.mock_all_auths();

    let academy_id = env.register_contract(None, AcademyRewardsContract);
    let academy = academy_rewards::AcademyRewardsContractClient::new(&env, &academy_id);

    let messaging_id = env.register_contract(None, UpgradeableMessagingContract);
    let messaging = messaging::UpgradeableMessagingContractClient::new(&env, &messaging_id);

    let admin    = Address::generate(&env);
    let approver = Address::generate(&env);
    let executor = Address::generate(&env);
    let notifier = Address::generate(&env);
    let user     = Address::generate(&env);

    let mut approvers = Vec::new(&env);
    approvers.push_back(approver);

    let cb_config = CircuitBreakerConfig {
        max_volume_per_period: 10_000_000,
        max_tx_count_per_period: 100,
        period_duration: 3600,
    };

    messaging.init(&admin, &approvers, &executor, &cb_config);
    academy.initialize(&admin, &cb_config);
    academy.create_badge_type(
        &admin,
        &2u32,
        &String::from_str(&env, "Silver"),
        &250u32,
        &3u32,
        &0u64,
    );
    academy.mint_badge(&admin, &user, &2u32);

    let discount = academy.redeem_badge(&user, &String::from_str(&env, "tx-2"));
    let payload  = String::from_str(&env, "Your academy badge was redeemed successfully");

    let message_id = messaging.send_message(&notifier, &user, &payload);
    assert_eq!(message_id, 1);
    assert_eq!(discount, 250);

    let unread = messaging.get_unread_count(&user);
    assert_eq!(unread, 1);

    let notifications = messaging.get_messages(&user, &false, &true, &true);
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications.get(0).unwrap().payload, payload);

    // ── Event assertions ─────────────────────────────────────────────────────
    assert_event_emitted(&env, Symbol::new(&env, "msg_sent"));
    assert_event_emitted(&env, Symbol::new(&env, "badge_redeemed"));
}

#[test]
fn test_shared_governance_module_across_contracts() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 1000);
    env.mock_all_auths();

    let trading_id = env.register_contract(None, UpgradeableTradingContract);
    let trading    = trading::UpgradeableTradingContractClient::new(&env, &trading_id);

    let messaging_id = env.register_contract(None, UpgradeableMessagingContract);
    let messaging    = messaging::UpgradeableMessagingContractClient::new(&env, &messaging_id);

    let admin    = Address::generate(&env);
    let approver = Address::generate(&env);
    let executor = Address::generate(&env);

    let mut approvers = Vec::new(&env);
    approvers.push_back(approver.clone());

    let cb_config = CircuitBreakerConfig {
        max_volume_per_period: 10_000_000,
        max_tx_count_per_period: 100,
        period_duration: 3600,
    };

    trading.init(&admin, &approvers, &executor, &cb_config);
    messaging.init(&admin, &approvers, &executor, &cb_config);

    let trading_proposal = trading.propose_upgrade(
        &admin,
        &Symbol::new(&env, "tv2hash"),
        &Symbol::new(&env, "UpgrTrade"),
        &approvers,
        &1u32,
        &3600u64,
    );
    trading.approve_upgrade(&trading_proposal, &approver);

    let messaging_proposal = messaging.propose_upgrade(
        &admin,
        &Symbol::new(&env, "mv2hash"),
        &Symbol::new(&env, "UpgrMsg"),
        &approvers,
        &1u32,
        &3600u64,
    );
    messaging.approve_upgrade(&messaging_proposal, &approver);

    let trade_status = trading.get_upgrade_proposal(&trading_proposal).status;
    let msg_status   = messaging.get_upgrade_proposal(&messaging_proposal).status;

    assert_eq!(trade_status, ProposalStatus::Approved);
    assert_eq!(msg_status, ProposalStatus::Approved);
}

// #[test]
// fn test_replay_message_on_messaging_rejected() {
//     let env = Env::default();
//     env.ledger().with_mut(|li| li.timestamp = 1000);
//     env.mock_all_auths();

//     let messaging_id = env.register_contract(None, UpgradeableMessagingContract);
//     let messaging = messaging::UpgradeableMessagingContractClient::new(&env, &messaging_id);

//     let admin = Address::generate(&env);
//     let approver = Address::generate(&env);
//     let executor = Address::generate(&env);

//     let mut approvers = Vec::new(&env);
//     approvers.push_back(approver.clone());

//     let cb_config = CircuitBreakerConfig {
//         max_volume_per_period: 10_000_000,
//         max_tx_count_per_period: 100,
//         period_duration: 3600,
//     };

//     messaging.init(&admin, &approvers, &executor, &cb_config);

//     let alice = Address::generate(&env);
//     let bob = Address::generate(&env);
//     let payload = String::from_str(&env, "adversarial replay");

//     let first = messaging.send_message(&alice, &bob, &payload);
//     assert_eq!(first, 1);

//     let replay = messaging.try_send_message(&alice, &bob, &payload);
//     assert!(replay.is_err());
// }

// #[test]
// fn test_reentrancy_guard_blocks_internal_reentry() {
//     let env = Env::default();
//     env.ledger().with_mut(|li| li.timestamp = 1000);
//     env.mock_all_auths();

//     let messaging_id = env.register_contract(None, UpgradeableMessagingContract);
//     let messaging = messaging::UpgradeableMessagingContractClient::new(&env, &messaging_id);

//     let admin = Address::generate(&env);
//     let approver = Address::generate(&env);
//     let executor = Address::generate(&env);

//     let mut approvers = Vec::new(&env);
//     approvers.push_back(approver.clone());

//     let cb_config = CircuitBreakerConfig {
//         max_volume_per_period: 10_000_000,
//         max_tx_count_per_period: 100,
//         period_duration: 3600,
//     };

//     messaging.init(&admin, &approvers, &executor, &cb_config);

//     let alice = Address::generate(&env);
//     let bob = Address::generate(&env);
//     messaging.send_message(&alice, &bob, &String::from_str(&env, "first"));

//     ReentrancyGuard::enter(&env);
//     let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
//         messaging.mark_as_read(&bob, &1u64);
//     }));
//     assert!(result.is_err(), "Reentrancy should be blocked");
//     ReentrancyGuard::exit(&env);
// }
    assert_eq!(msg_status,   ProposalStatus::Approved);

    // ── Governance has no on-chain events — validate via state ────────────────
    // Both proposals reached Approved status, confirming the governance module
    // correctly processes propose → approve flows across contracts.
    // (Event emission for governance transitions is tracked in issue #774)
}

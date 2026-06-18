#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol,
};
use shared::events::{
    extended_topics,
    TcrApplicationEvent, TcrChallengedEvent, TcrVotedEvent, TcrResolvedEvent,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ListingStatus {
    Pending,
    Challenged,
    Approved,
    Rejected,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Listing {
    pub applicant: Address,
    pub deposit: i128,
    pub metadata: Symbol,
    pub status: ListingStatus,
    pub challenge_id: u32,
    pub expiry: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Challenge {
    pub challenger: Address,
    pub listing_id: u32,
    pub deposit: i128,
    pub votes_for: i128,
    pub votes_against: i128,
    pub end_time: u64,
    pub resolved: bool,
}

#[contract]
pub struct TCRContract;

#[contractimpl]
impl TCRContract {
    pub fn apply(env: Env, applicant: Address, deposit: i128, metadata: Symbol) -> u32 {
        applicant.require_auth();

        let mut listing_count: u32 = env
            .storage()
            .instance()
            .get(&symbol_short!("L_COUNT"))
            .unwrap_or(0);
        listing_count += 1;

        let listing = Listing {
            applicant: applicant.clone(),
            deposit,
            metadata: metadata.clone(),
            status: ListingStatus::Pending,
            challenge_id: 0,
            expiry: env.ledger().timestamp() + 604800,
        };

        env.storage().instance().set(&listing_count, &listing);
        env.storage()
            .instance()
            .set(&symbol_short!("L_COUNT"), &listing_count);

        env.events().publish(
            (extended_topics::TCR_APPLIED,),
            TcrApplicationEvent {
                listing_id: listing_count,
                applicant,
                deposit,
                metadata,
                timestamp: env.ledger().timestamp(),
            },
        );

        listing_count
    }

    pub fn challenge(env: Env, challenger: Address, listing_id: u32, deposit: i128) -> u32 {
        challenger.require_auth();

        let mut listing: Listing = env
            .storage()
            .instance()
            .get(&listing_id)
            .expect("Listing not found");
        assert!(
            listing.status == ListingStatus::Pending,
            "Cannot challenge non-pending listing"
        );
        assert!(deposit >= listing.deposit, "Insufficient challenge deposit");

        let mut challenge_count: u32 = env
            .storage()
            .instance()
            .get(&symbol_short!("C_COUNT"))
            .unwrap_or(0);
        challenge_count += 1;

        let challenge = Challenge {
            challenger: challenger.clone(),
            listing_id,
            deposit,
            votes_for: 0,
            votes_against: 0,
            end_time: env.ledger().timestamp() + 604800,
            resolved: false,
        };

        listing.status = ListingStatus::Challenged;
        listing.challenge_id = challenge_count;

        env.storage().instance().set(&listing_id, &listing);
        env.storage().instance().set(&challenge_count, &challenge);
        env.storage()
            .instance()
            .set(&symbol_short!("C_COUNT"), &challenge_count);

        env.events().publish(
            (extended_topics::TCR_CHALLENGED,),
            TcrChallengedEvent {
                challenge_id: challenge_count,
                listing_id,
                challenger,
                deposit,
                timestamp: env.ledger().timestamp(),
            },
        );

        challenge_count
    }

    pub fn vote(env: Env, voter: Address, challenge_id: u32, side: bool, amount: i128) {
        voter.require_auth();

        let mut challenge: Challenge = env
            .storage()
            .instance()
            .get(&challenge_id)
            .expect("Challenge not found");
        assert!(!challenge.resolved, "Challenge already resolved");
        assert!(
            env.ledger().timestamp() < challenge.end_time,
            "Voting period ended"
        );

        if side {
            challenge.votes_for += amount;
        } else {
            challenge.votes_against += amount;
        }

        env.storage().instance().set(&challenge_id, &challenge);

        let key = (voter.clone(), challenge_id);
        let current_stake: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&key, &(current_stake + amount));

        env.events().publish(
            (extended_topics::TCR_VOTED,),
            TcrVotedEvent {
                challenge_id,
                voter,
                side,
                weight: amount,
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    pub fn resolve(env: Env, listing_id: u32) {
        let mut listing: Listing = env
            .storage()
            .instance()
            .get(&listing_id)
            .expect("Listing not found");

        let accepted;

        if listing.status == ListingStatus::Challenged {
            let mut challenge: Challenge = env
                .storage()
                .instance()
                .get(&listing.challenge_id)
                .expect("Challenge not found");
            assert!(
                env.ledger().timestamp() >= challenge.end_time,
                "Voting period active"
            );
            assert!(!challenge.resolved, "Already resolved");

            if challenge.votes_for > challenge.votes_against {
                listing.status = ListingStatus::Approved;
                accepted = true;
            } else {
                listing.status = ListingStatus::Rejected;
                accepted = false;
            }

            challenge.resolved = true;
            env.storage()
                .instance()
                .set(&listing.challenge_id, &challenge);
        } else if listing.status == ListingStatus::Pending {
            assert!(
                env.ledger().timestamp() >= listing.expiry,
                "Challenge period active"
            );
            listing.status = ListingStatus::Approved;
            accepted = true;
        } else {
            return;
        }

        let challenge_id = listing.challenge_id;
        env.storage().instance().set(&listing_id, &listing);

        env.events().publish(
            (extended_topics::TCR_RESOLVED,),
            TcrResolvedEvent {
                listing_id,
                challenge_id,
                accepted,
                timestamp: env.ledger().timestamp(),
            },
        );
    }
}
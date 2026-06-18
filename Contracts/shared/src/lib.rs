#![no_std]
//! Shared utilities and types for Stellara contracts

use soroban_sdk::contracttype;

#[contracttype]
#[derive(Clone, Debug)]
pub struct ContractConfig {
    pub admin: soroban_sdk::String,
    pub version: u32,
    pub is_paused: bool,
}

pub mod acl;
pub mod circuit_breaker;
pub mod events;
pub mod fees;
pub mod governance;
pub mod nonce;
pub mod reentrancy_guard;

/// Standard contract error codes
pub mod errors {
    pub const UNAUTHORIZED: &str = "UNAUTHORIZED";
    pub const NOT_FOUND: &str = "NOT_FOUND";
    pub const INVALID_AMOUNT: &str = "INVALID_AMOUNT";
    pub const PAUSED: &str = "PAUSED";
    pub const ALREADY_EXISTS: &str = "ALREADY_EXISTS";
    pub const REENTRANCY_DETECTED: &str = "REENTRANCY_DETECTED";
    pub const REPLAY_DETECTED: &str = "REPLAY_DETECTED";
    pub const NONCE_OUT_OF_ORDER: &str = "NONCE_OUT_OF_ORDER";
}

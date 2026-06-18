#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env, Symbol,
};
use shared::events::{
    extended_topics,
    AssetRegisteredEvent, CdpOpenedEvent, CdpClosedEvent,
    CollateralAddedEvent, CdpLiquidatedEvent, PriceUpdatedEvent,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    InsufficientCollateral = 5,
    BelowMinCratio = 6,
    CDPNotFound = 7,
    NotLiquidatable = 8,
    AssetNotFound = 9,
}

#[contracttype]
#[derive(Clone)]
pub struct CDP {
    pub owner: Address,
    pub collateral_amount: i128, // in base units
    pub minted_amount: i128,     // synthetic tokens minted
    pub collateral_ratio: i128,  // scaled by 10000 (15000 = 150%)
    pub is_active: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct SyntheticConfig {
    pub oracle_price: i128,     // price scaled by 1_000_000
    pub min_cratio: i128,       // scaled by 10000 (15000 = 150%)
    pub liq_cratio: i128,       // scaled by 10000 (12000 = 120%)
    pub liq_penalty: i128,      // scaled by 10000 (1300 = 13%)
    pub stability_fee_bps: i32, // annual fee in bps (200 = 2%)
    pub total_minted: i128,
    pub is_active: bool,
}

mod keys {
    use soroban_sdk::{symbol_short, Symbol};
    pub const ADMIN: Symbol = symbol_short!("admin");
}

#[contract]
pub struct SyntheticAssetsContract;

#[contractimpl]
impl SyntheticAssetsContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&keys::ADMIN) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&keys::ADMIN, &admin);
        Ok(())
    }

    /// Register a synthetic asset with initial config
    pub fn register_asset(
        env: Env,
        caller: Address,
        asset_symbol: Symbol,
        min_cratio: i128,
        liq_cratio: i128,
        liq_penalty: i128,
        stability_fee_bps: i32,
    ) -> Result<(), Error> {
        caller.require_auth();
        Self::require_admin(&env, &caller)?;

        let config = SyntheticConfig {
            oracle_price: 0,
            min_cratio,
            liq_cratio,
            liq_penalty,
            stability_fee_bps,
            total_minted: 0,
            is_active: true,
        };
        env.storage().persistent().set(&asset_symbol, &config);

        env.events().publish(
            (extended_topics::ASSET_REGISTERED,),
            AssetRegisteredEvent {
                asset_symbol: asset_symbol.clone(),
                registered_by: caller,
                collateral_ratio: min_cratio as u32,
                timestamp: env.ledger().timestamp(),
            },
        );
        Ok(())
    }

    /// Update oracle price (called by authorized oracle)
    pub fn update_price(
        env: Env,
        caller: Address,
        asset_symbol: Symbol,
        new_price: i128,
    ) -> Result<(), Error> {
        caller.require_auth();
        Self::require_admin(&env, &caller)?;

        let mut config: SyntheticConfig = env
            .storage()
            .persistent()
            .get(&asset_symbol)
            .ok_or(Error::AssetNotFound)?;

        config.oracle_price = new_price;
        env.storage().persistent().set(&asset_symbol, &config);

        env.events().publish(
            (extended_topics::PRICE_UPDATED,),
            PriceUpdatedEvent {
                asset_symbol,
                old_price: config.oracle_price,
                new_price,
                updated_by: caller,
                timestamp: env.ledger().timestamp(),
            },
        );
        Ok(())
    }

    /// Open CDP: deposit collateral
    pub fn open_cdp(
        env: Env,
        owner: Address,
        asset_symbol: Symbol,
        collateral_amount: i128,
    ) -> Result<(), Error> {
        owner.require_auth();

        if collateral_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let cdp_key = Self::cdp_key(&owner, &asset_symbol);
        let cdp = CDP {
            owner: owner.clone(),
            collateral_amount,
            minted_amount: 0,
            collateral_ratio: 0,
            is_active: true,
        };
        env.storage().persistent().set(&cdp_key, &cdp);

        env.events().publish(
            (extended_topics::CDP_OPENED,),
            CdpOpenedEvent {
                owner: owner.clone(),
                asset_symbol,
                collateral_amount,
                timestamp: env.ledger().timestamp(),
            },
        );
        Ok(())
    }

    /// Mint synthetic tokens against collateral
    pub fn mint(
        env: Env,
        owner: Address,
        asset_symbol: Symbol,
        mint_amount: i128,
    ) -> Result<i128, Error> {
        owner.require_auth();

        if mint_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let config: SyntheticConfig = env
            .storage()
            .persistent()
            .get(&asset_symbol)
            .ok_or(Error::AssetNotFound)?;

        let cdp_key = Self::cdp_key(&owner, &asset_symbol);
        let mut cdp: CDP = env
            .storage()
            .persistent()
            .get(&cdp_key)
            .ok_or(Error::CDPNotFound)?;

        // collateral_ratio = (collateral_amount * 1e6 / oracle_price) / (minted+mint) * 10000
        let new_minted = cdp.minted_amount + mint_amount;
        let collateral_usd = cdp.collateral_amount * 1_000_000 / config.oracle_price;
        let cratio = collateral_usd * 10000 / new_minted;

        if cratio < config.min_cratio {
            return Err(Error::BelowMinCratio);
        }

        cdp.minted_amount = new_minted;
        cdp.collateral_ratio = cratio;

        let mut updated_config = config.clone();
        updated_config.total_minted += mint_amount;

        env.storage().persistent().set(&cdp_key, &cdp);
        env.storage()
            .persistent()
            .set(&asset_symbol, &updated_config);

        Ok(new_minted)
    }

    /// Burn synthetic tokens to reduce debt
    pub fn burn(
        env: Env,
        owner: Address,
        asset_symbol: Symbol,
        burn_amount: i128,
    ) -> Result<(), Error> {
        owner.require_auth();

        if burn_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let config: SyntheticConfig = env
            .storage()
            .persistent()
            .get(&asset_symbol)
            .ok_or(Error::AssetNotFound)?;

        let cdp_key = Self::cdp_key(&owner, &asset_symbol);
        let mut cdp: CDP = env
            .storage()
            .persistent()
            .get(&cdp_key)
            .ok_or(Error::CDPNotFound)?;

        if burn_amount > cdp.minted_amount {
            return Err(Error::InvalidAmount);
        }

        cdp.minted_amount -= burn_amount;

        // Recalculate cratio
        if cdp.minted_amount > 0 {
            let collateral_usd = cdp.collateral_amount * 1_000_000 / config.oracle_price;
            cdp.collateral_ratio = collateral_usd * 10000 / cdp.minted_amount;
        } else {
            cdp.collateral_ratio = i128::MAX;
        }

        let mut updated_config = config;
        updated_config.total_minted -= burn_amount;

        env.storage().persistent().set(&cdp_key, &cdp);
        env.storage()
            .persistent()
            .set(&asset_symbol, &updated_config);
        Ok(())
    }

    /// Add more collateral to improve health
    pub fn add_collateral(
        env: Env,
        owner: Address,
        asset_symbol: Symbol,
        amount: i128,
    ) -> Result<(), Error> {
        owner.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let config: SyntheticConfig = env
            .storage()
            .persistent()
            .get(&asset_symbol)
            .ok_or(Error::AssetNotFound)?;

        let cdp_key = Self::cdp_key(&owner, &asset_symbol);
        let mut cdp: CDP = env
            .storage()
            .persistent()
            .get(&cdp_key)
            .ok_or(Error::CDPNotFound)?;

        cdp.collateral_amount += amount;

        if cdp.minted_amount > 0 {
            let collateral_usd = cdp.collateral_amount * 1_000_000 / config.oracle_price;
            cdp.collateral_ratio = collateral_usd * 10000 / cdp.minted_amount;
        }

        env.storage().persistent().set(&cdp_key, &cdp);

        env.events().publish(
            (extended_topics::COLLATERAL_ADDED,),
            CollateralAddedEvent {
                owner,
                asset_symbol,
                amount,
                new_ratio: cdp.collateral_ratio as u32,
                timestamp: env.ledger().timestamp(),
            },
        );
        Ok(())
    }

    /// Liquidate an undercollateralized CDP
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        cdp_owner: Address,
        asset_symbol: Symbol,
    ) -> Result<i128, Error> {
        liquidator.require_auth();

        let config: SyntheticConfig = env
            .storage()
            .persistent()
            .get(&asset_symbol)
            .ok_or(Error::AssetNotFound)?;

        let cdp_key = Self::cdp_key(&cdp_owner, &asset_symbol);
        let mut cdp: CDP = env
            .storage()
            .persistent()
            .get(&cdp_key)
            .ok_or(Error::CDPNotFound)?;

        if cdp.collateral_ratio >= config.liq_cratio {
            return Err(Error::NotLiquidatable);
        }

        // Seize collateral with penalty: seized = debt_usd * (1 + penalty) / collateral_price
        let penalty_collateral = cdp.collateral_amount * config.liq_penalty / 10000;
        let seized = cdp
            .collateral_amount
            .min(cdp.collateral_amount - penalty_collateral);

        let mut updated_config = config;
        updated_config.total_minted -= cdp.minted_amount;

        cdp.is_active = false;
        cdp.minted_amount = 0;
        cdp.collateral_amount = 0;

        env.storage().persistent().set(&cdp_key, &cdp);
        env.storage()
            .persistent()
            .set(&asset_symbol, &updated_config);

        env.events().publish(
            (extended_topics::CDP_LIQUIDATED,),
            CdpLiquidatedEvent {
                owner: cdp_owner,
                liquidator,
                asset_symbol,
                collateral_seized: seized,
                debt_repaid: cdp.minted_amount,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(seized)
    }

    /// Close a CDP with zero debt
    pub fn close_cdp(env: Env, owner: Address, asset_symbol: Symbol) -> Result<i128, Error> {
        owner.require_auth();

        let cdp_key = Self::cdp_key(&owner, &asset_symbol);
        let mut cdp: CDP = env
            .storage()
            .persistent()
            .get(&cdp_key)
            .ok_or(Error::CDPNotFound)?;

        if cdp.minted_amount > 0 {
            return Err(Error::InsufficientCollateral);
        }

        let returned = cdp.collateral_amount;
        cdp.is_active = false;
        cdp.collateral_amount = 0;

        env.storage().persistent().set(&cdp_key, &cdp);

        env.events().publish(
            (extended_topics::CDP_CLOSED,),
            CdpClosedEvent {
                owner,
                asset_symbol,
                collateral_returned: returned,
                timestamp: env.ledger().timestamp(),
            },
        );
        Ok(returned)
    }

    /// View CDP info
    pub fn get_cdp(env: Env, owner: Address, asset_symbol: Symbol) -> Result<CDP, Error> {
        let cdp_key = Self::cdp_key(&owner, &asset_symbol);
        env.storage()
            .persistent()
            .get(&cdp_key)
            .ok_or(Error::CDPNotFound)
    }

    /// View asset config
    pub fn get_config(env: Env, asset_symbol: Symbol) -> Result<SyntheticConfig, Error> {
        env.storage()
            .persistent()
            .get(&asset_symbol)
            .ok_or(Error::AssetNotFound)
    }

    // ── Helpers ────────────────────────────────────────────────────────────────

    fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&keys::ADMIN)
            .ok_or(Error::NotInitialized)?;
        if &admin != caller {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn cdp_key(owner: &Address, asset: &Symbol) -> (Address, Symbol) {
        (owner.clone(), asset.clone())
    }
}
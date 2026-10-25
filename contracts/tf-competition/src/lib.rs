//! `tf-competition` — TradeFighters competition escrow on Soroban.
//!
//! Port of the duel escrow that runs in production on Solana (Anchor program
//! `TFrXd1Fm4FoWdhzqm7qxL7dD8yqzDw3kf8Cjt9gVgZx`). Stakes are held in USDC
//! through the Stellar Asset Contract; the contract address itself is the
//! escrow, so no admin key can move escrowed funds.
//!
//! Current state: config + admin entrypoints. The duel entrypoints
//! (`create_duel`, `join_duel`, `cancel_duel`, `settle_duel`) come next and
//! reuse the types, errors and storage helpers already defined here.
#![no_std]

mod errors;
mod events;
mod storage;
mod types;

pub use errors::Error;
pub use events::{AdminChanged, ConfigInitialized, ConfigUpdated};
pub use types::{Config, Duel, DuelOutcome, DuelParams, DuelStatus, BPS_DENOMINATOR};

use soroban_sdk::{contract, contractimpl, panic_with_error, Address, Env};

/// Highest platform fee the config accepts (20 %), so a bad config can never
/// take most of a pool.
pub const MAX_FEE_BPS: u32 = 2_000;

#[contract]
pub struct TfCompetition;

#[contractimpl]
impl TfCompetition {
    /// Runs once, atomically with the deploy, so nobody can front-run the
    /// initialization. Validates and stores the config.
    ///
    /// Deploy with:
    /// `stellar contract deploy --wasm ... -- --config '<Config as JSON>'`
    pub fn __constructor(env: Env, config: Config) {
        validate_config(&env, &config);
        storage::set_config(&env, &config);
        storage::bump_instance(&env);

        ConfigInitialized {
            admin: config.admin,
            resolver: config.resolver,
            treasury: config.treasury,
            usdc: config.usdc,
            fee_bps: config.fee_bps,
        }
        .publish(&env);
    }

    /// Returns the current config.
    pub fn config(env: Env) -> Config {
        storage::bump_instance(&env);
        storage::get_config(&env)
    }

    /// Replaces the config. Admin only. The admin itself is changed with
    /// `set_admin`, never here.
    pub fn set_config(env: Env, config: Config) {
        storage::bump_instance(&env);
        let current = storage::get_config(&env);
        current.admin.require_auth();

        if config.admin != current.admin {
            panic_with_error!(&env, Error::AdminChangeNotAllowed);
        }
        validate_config(&env, &config);
        storage::set_config(&env, &config);

        ConfigUpdated {
            admin: config.admin,
            resolver: config.resolver,
            treasury: config.treasury,
            fee_bps: config.fee_bps,
        }
        .publish(&env);
    }

    /// Hands the admin role to `new_admin`. Needs both the current and the new
    /// admin to sign, so the role can't be sent to an address nobody controls.
    pub fn set_admin(env: Env, new_admin: Address) {
        storage::bump_instance(&env);
        let mut config = storage::get_config(&env);
        config.admin.require_auth();
        new_admin.require_auth();

        let old_admin = config.admin.clone();
        config.admin = new_admin.clone();
        storage::set_config(&env, &config);

        AdminChanged {
            old_admin,
            new_admin,
        }
        .publish(&env);
    }

    /// Returns a duel by id, if it exists.
    pub fn duel(env: Env, duel_id: u64) -> Option<Duel> {
        storage::bump_instance(&env);
        storage::get_duel(&env, duel_id)
    }
}

/// Panics with `Error::InvalidConfig` unless every bound holds.
fn validate_config(env: &Env, config: &Config) {
    let valid = config.fee_bps <= MAX_FEE_BPS
        && config.creator_fee_bps <= BPS_DENOMINATOR
        && config.min_entry_fee > 0
        && config.max_entry_fee >= config.min_entry_fee;
    if !valid {
        panic_with_error!(env, Error::InvalidConfig);
    }
}

#[cfg(test)]
mod test;

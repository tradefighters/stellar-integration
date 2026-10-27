//! `tf-competition` — TradeFighters competition escrow on Soroban.
//!
//! Port of the duel escrow that runs in production on Solana (Anchor program
//! `TFrXd1Fm4FoWdhzqm7qxL7dD8yqzDw3kf8Cjt9gVgZx`). Stakes are held in USDC
//! through the Stellar Asset Contract; the contract address itself is the
//! escrow, so no admin key can move escrowed funds.
//!
//! Duel lifecycle:
//!
//! ```text
//! create_duel ─► Open ─► join_duel ─► Active ─► settle_duel ─► Settled
//!                  │                    │
//!                  └──── cancel_duel ───┴──► Cancelled (stakes refunded)
//! ```
//!
//! Battle royales, prize top-ups and `tf-rewards` come in tranche 1.
#![no_std]

mod errors;
mod events;
mod storage;
mod types;

pub use errors::Error;
pub use events::{
    AdminChanged, ConfigInitialized, ConfigUpdated, DuelCancelled, DuelCreated, DuelJoined,
    DuelPayout, DuelSettled,
};
pub use types::{Config, Duel, DuelOutcome, DuelParams, DuelStatus, PoolSplit, BPS_DENOMINATOR};

use soroban_sdk::{contract, contractimpl, panic_with_error, token, Address, Env};

/// Highest platform fee the config accepts (20 %), so a bad config can never
/// take most of a pool.
pub const MAX_FEE_BPS: u32 = 2_000;

/// Smallest virtual starting capital (USD, 6 decimals): $2, as on Solana.
pub const MIN_SCC: u64 = 2_000_000;
/// Largest virtual starting capital (USD, 6 decimals): $100M, as on Solana.
pub const MAX_SCC: u64 = 100_000_000_000_000;

#[contract]
pub struct TfCompetition;

#[contractimpl]
impl TfCompetition {
    // ------------------------------------------------------------------
    // Config and admin
    // ------------------------------------------------------------------

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

    /// Replaces the config. Admin only. The admin is changed with `set_admin`,
    /// and the USDC token can never change (live duels hold it).
    pub fn set_config(env: Env, config: Config) {
        storage::bump_instance(&env);
        let current = storage::get_config(&env);
        current.admin.require_auth();

        if config.admin != current.admin {
            panic_with_error!(&env, Error::AdminChangeNotAllowed);
        }
        if config.usdc != current.usdc {
            panic_with_error!(&env, Error::TokenChangeNotAllowed);
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

    // ------------------------------------------------------------------
    // Duels
    // ------------------------------------------------------------------

    /// Returns a duel by id, if it exists.
    pub fn duel(env: Env, duel_id: u64) -> Option<Duel> {
        storage::bump_instance(&env);
        storage::get_duel(&env, duel_id)
    }

    /// Creates a duel and escrows the creator's stake. Returns the duel id.
    ///
    /// Signed by `creator`. Fails unless the stake is within the configured
    /// bounds, `scc` is within `[MIN_SCC, MAX_SCC]`, `starts_at` is at least
    /// `min_start_delay` seconds away and `ends_at` is after `starts_at`.
    pub fn create_duel(env: Env, creator: Address, params: DuelParams) -> u64 {
        storage::bump_instance(&env);
        creator.require_auth();
        let config = storage::get_config(&env);
        let now = env.ledger().timestamp();

        if params.stake < config.min_entry_fee || params.stake > config.max_entry_fee {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if params.scc < MIN_SCC || params.scc > MAX_SCC {
            panic_with_error!(&env, Error::SccOutOfRange);
        }
        let earliest_start = now
            .checked_add(config.min_start_delay)
            .unwrap_or_else(|| panic_with_error!(&env, Error::MathOverflow));
        if params.starts_at < earliest_start {
            panic_with_error!(&env, Error::InvalidStart);
        }
        if params.ends_at <= params.starts_at {
            panic_with_error!(&env, Error::InvalidEnd);
        }

        let id = storage::next_duel_id(&env);
        let duel = Duel {
            id,
            creator: creator.clone(),
            opponent: None,
            stake: params.stake,
            total_pool: params.stake,
            scc: params.scc,
            starts_at: params.starts_at,
            ends_at: params.ends_at,
            status: DuelStatus::Open,
            fee_bps: config.fee_bps,
            created_at: now,
        };
        // State first, then the transfer: a failing transfer reverts everything.
        storage::set_duel(&env, &duel);
        token::TokenClient::new(&env, &config.usdc).transfer(
            &creator,
            env.current_contract_address(),
            &params.stake,
        );

        DuelCreated {
            duel_id: id,
            creator,
            stake: duel.stake,
            scc: duel.scc,
            starts_at: duel.starts_at,
            ends_at: duel.ends_at,
            fee_bps: duel.fee_bps,
        }
        .publish(&env);
        id
    }

    /// Joins an open duel and escrows the opponent's stake (same amount as the
    /// creator's). Signed by `player`. Only possible before `starts_at`, so
    /// both players trade the full window.
    pub fn join_duel(env: Env, duel_id: u64, player: Address) {
        storage::bump_instance(&env);
        player.require_auth();
        let config = storage::get_config(&env);
        let mut duel = load_duel(&env, duel_id);

        if duel.status != DuelStatus::Open {
            panic_with_error!(&env, Error::InvalidStatus);
        }
        if player == duel.creator {
            panic_with_error!(&env, Error::CreatorSameAsOpponent);
        }
        if env.ledger().timestamp() >= duel.starts_at {
            panic_with_error!(&env, Error::DuelExpired);
        }

        duel.opponent = Some(player.clone());
        duel.total_pool = checked_add(&env, duel.total_pool, duel.stake);
        duel.status = DuelStatus::Active;
        storage::set_duel(&env, &duel);
        token::TokenClient::new(&env, &config.usdc).transfer(
            &player,
            env.current_contract_address(),
            &duel.stake,
        );

        DuelJoined {
            duel_id,
            opponent: player,
            total_pool: duel.total_pool,
        }
        .publish(&env);
    }

    /// Cancels a duel and refunds every stake in full, no fee.
    ///
    /// Signed by `caller`, who must be:
    /// - the creator or the resolver, while the duel is `Open`;
    /// - the resolver, while the duel is `Active` and has not started.
    ///
    /// Once trading has started, a duel that can't be scored is closed with
    /// `settle_duel(Issue)` instead.
    pub fn cancel_duel(env: Env, duel_id: u64, caller: Address) {
        storage::bump_instance(&env);
        caller.require_auth();
        let config = storage::get_config(&env);
        let mut duel = load_duel(&env, duel_id);
        let is_resolver = caller == config.resolver;

        let allowed = match duel.status {
            DuelStatus::Open => is_resolver || caller == duel.creator,
            DuelStatus::Active => {
                if env.ledger().timestamp() >= duel.starts_at {
                    panic_with_error!(&env, Error::DuelExpired);
                }
                is_resolver
            }
            DuelStatus::Cancelled | DuelStatus::Settled => {
                panic_with_error!(&env, Error::InvalidStatus)
            }
        };
        if !allowed {
            panic_with_error!(&env, Error::Unauthorized);
        }

        let refunded = duel.total_pool;
        duel.status = DuelStatus::Cancelled;
        duel.total_pool = 0;
        storage::set_duel(&env, &duel);

        DuelCancelled {
            duel_id,
            cancelled_by: caller,
            refunded,
        }
        .publish(&env);

        let usdc = token::TokenClient::new(&env, &config.usdc);
        pay(&env, &usdc, duel_id, &duel.creator, duel.stake);
        if let Some(opponent) = &duel.opponent {
            pay(&env, &usdc, duel_id, opponent, duel.stake);
        }
    }

    /// Settles an active duel once it has ended. Signed by the resolver.
    ///
    /// - `Win(player)`: the winner gets the pool minus the platform fee.
    /// - `Draw`: each player gets their stake back minus half the fee.
    /// - `Issue`: the duel couldn't be scored; both stakes back in full, no fee.
    ///
    /// The fee goes to the treasury. The escrow for this duel ends at zero.
    pub fn settle_duel(env: Env, duel_id: u64, outcome: DuelOutcome) {
        storage::bump_instance(&env);
        let config = storage::get_config(&env);
        config.resolver.require_auth();
        let mut duel = load_duel(&env, duel_id);

        if duel.status != DuelStatus::Active {
            panic_with_error!(&env, Error::InvalidStatus);
        }
        if env.ledger().timestamp() < duel.ends_at {
            panic_with_error!(&env, Error::DuelNotEnded);
        }
        let opponent = duel
            .opponent
            .clone()
            .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidStatus));
        if let DuelOutcome::Win(winner) = &outcome {
            if *winner != duel.creator && *winner != opponent {
                panic_with_error!(&env, Error::InvalidWinner);
            }
        }

        let split = split_pool(&env, &duel, &outcome);
        let total_pool = duel.total_pool;
        duel.status = DuelStatus::Settled;
        duel.total_pool = 0;
        storage::set_duel(&env, &duel);

        DuelSettled {
            duel_id,
            outcome,
            total_pool,
            fee: split.fee,
        }
        .publish(&env);

        let usdc = token::TokenClient::new(&env, &config.usdc);
        pay(&env, &usdc, duel_id, &duel.creator, split.to_creator);
        pay(&env, &usdc, duel_id, &opponent, split.to_opponent);
        if split.fee > 0 {
            usdc.transfer(
                &env.current_contract_address(),
                &config.treasury,
                &split.fee,
            );
        }
    }
}

/// Splits a duel's pool for `outcome`. The three parts always add up to
/// `total_pool` exactly, so the escrow for the duel ends at zero.
///
/// The fee is `total_pool * fee_bps / 10_000`, rounded down. On a draw each
/// player pays half of it (rounded down too), as the Solana resolver does.
pub fn split_pool(env: &Env, duel: &Duel, outcome: &DuelOutcome) -> PoolSplit {
    let pool = duel.total_pool;
    let full_fee = checked_mul(env, pool, duel.fee_bps as i128) / BPS_DENOMINATOR as i128;

    let split = match outcome {
        DuelOutcome::Win(winner) => {
            let prize = pool - full_fee;
            if *winner == duel.creator {
                PoolSplit {
                    to_creator: prize,
                    to_opponent: 0,
                    fee: full_fee,
                }
            } else {
                PoolSplit {
                    to_creator: 0,
                    to_opponent: prize,
                    fee: full_fee,
                }
            }
        }
        DuelOutcome::Draw => {
            let half_fee = full_fee / 2;
            let each = duel.stake - half_fee;
            PoolSplit {
                to_creator: each,
                to_opponent: each,
                fee: pool - 2 * each,
            }
        }
        DuelOutcome::Issue => PoolSplit {
            to_creator: duel.stake,
            to_opponent: duel.stake,
            fee: pool - 2 * duel.stake,
        },
    };
    // Conservation and no negative transfer, whatever the inputs.
    let sum = checked_add(
        env,
        checked_add(env, split.to_creator, split.to_opponent),
        split.fee,
    );
    if sum != pool || split.to_creator < 0 || split.to_opponent < 0 || split.fee < 0 {
        panic_with_error!(env, Error::MathOverflow);
    }
    split
}

/// Loads a duel or fails with `DuelNotFound`.
fn load_duel(env: &Env, duel_id: u64) -> Duel {
    storage::get_duel(env, duel_id).unwrap_or_else(|| panic_with_error!(env, Error::DuelNotFound))
}

/// Transfers `amount` from the escrow to `to` and publishes `DuelPayout`.
/// Skips zero amounts (the loser of a `Win`).
fn pay(env: &Env, usdc: &token::TokenClient, duel_id: u64, to: &Address, amount: i128) {
    if amount == 0 {
        return;
    }
    usdc.transfer(&env.current_contract_address(), to, &amount);
    DuelPayout {
        duel_id,
        to: to.clone(),
        amount,
    }
    .publish(env);
}

fn checked_add(env: &Env, a: i128, b: i128) -> i128 {
    a.checked_add(b)
        .unwrap_or_else(|| panic_with_error!(env, Error::MathOverflow))
}

fn checked_mul(env: &Env, a: i128, b: i128) -> i128 {
    a.checked_mul(b)
        .unwrap_or_else(|| panic_with_error!(env, Error::MathOverflow))
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

#[cfg(test)]
mod test_duel;

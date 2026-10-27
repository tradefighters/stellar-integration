//! Data types stored by the contract or passed to its entrypoints.
//!
//! Amounts are `i128` in USDC base units. USDC on Stellar has **7 decimals**
//! (the Solana program uses 6), so 1 USDC = 10_000_000.

use soroban_sdk::{contracttype, Address};

/// Basis-point denominator: 10_000 bps = 100 %.
pub const BPS_DENOMINATOR: u32 = 10_000;

/// Contract-wide settings, kept in instance storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// May change the config and the admin. Cannot move escrowed funds.
    pub admin: Address,
    /// The only address allowed to settle competitions (the TradeFighters resolver).
    pub resolver: Address,
    /// Receives the platform fee.
    pub treasury: Address,
    /// USDC Stellar Asset Contract id. Every stake is held in this token.
    pub usdc: Address,
    /// Platform fee on a settled pool, in basis points.
    pub fee_bps: u32,
    /// Share of the platform fee credited to the competition creator, in basis points.
    pub creator_fee_bps: u32,
    /// Smallest stake / entry fee accepted, in USDC base units.
    pub min_entry_fee: i128,
    /// Largest stake / entry fee accepted, in USDC base units.
    pub max_entry_fee: i128,
    /// Seconds between creation and `starts_at`, so a competition cannot be
    /// created and settled in the same moment.
    pub min_start_delay: u64,
}

/// Lifecycle of a duel.
///
/// The Solana program uses Draft / Active / Cancelled / Completed. Here
/// "waiting for the start" and "trading" are both `Active`: the difference
/// comes from `starts_at`, so it is not stored twice.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DuelStatus {
    /// Created, creator stake escrowed, no opponent yet.
    Open,
    /// Opponent joined, both stakes escrowed.
    Active,
    /// Cancelled before settlement; stakes refunded.
    Cancelled,
    /// Settled by the resolver; payout done.
    Settled,
}

/// Result the resolver submits when settling a duel (same three cases as the
/// Solana `DuelResult`).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DuelOutcome {
    /// This player wins the pool minus the platform fee.
    Win(Address),
    /// Both stakes returned, no fee.
    Draw,
    /// The competition could not be scored (venue outage, data issue): both
    /// stakes returned, no fee.
    Issue,
}

/// Parameters chosen by the creator of a duel.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuelParams {
    /// Stake each player puts in, in USDC base units.
    pub stake: i128,
    /// Virtual starting capital each player trades with (off-chain margin
    /// engine), in USD with 6 decimals, as on Solana.
    pub scc: u64,
    /// Unix seconds when trading starts.
    pub starts_at: u64,
    /// Unix seconds when trading ends.
    pub ends_at: u64,
}

/// One duel, kept in persistent storage under `DataKey::Duel(id)`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Duel {
    pub id: u64,
    pub creator: Address,
    pub opponent: Option<Address>,
    pub stake: i128,
    /// USDC currently held for this duel (stakes plus any top-ups).
    pub total_pool: i128,
    pub scc: u64,
    pub starts_at: u64,
    pub ends_at: u64,
    pub status: DuelStatus,
    /// Platform fee copied from the config at creation, so a later config
    /// change never alters the terms of a duel already running.
    pub fee_bps: u32,
    /// Unix seconds of the creation ledger.
    pub created_at: u64,
}

/// How the pool of a settled duel is split. Computed by `split_pool`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PoolSplit {
    /// Paid to the winner, or to each player on a draw / issue.
    pub to_creator: i128,
    pub to_opponent: i128,
    /// Paid to the treasury.
    pub fee: i128,
}

//! Events published by the contract.
//!
//! The indexer, the monitoring plan (tranche 2) and the on-chain target of
//! tranche 3 (cumulative USDC settled) are all computed from these events,
//! so their names and fields are part of the public interface.

use soroban_sdk::{contractevent, Address};

use crate::types::DuelOutcome;

/// Published once, by the constructor.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigInitialized {
    #[topic]
    pub admin: Address,
    pub resolver: Address,
    pub treasury: Address,
    pub usdc: Address,
    pub fee_bps: u32,
}

/// Published by `set_config`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigUpdated {
    #[topic]
    pub admin: Address,
    pub resolver: Address,
    pub treasury: Address,
    pub fee_bps: u32,
}

/// Published by `set_admin`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChanged {
    pub old_admin: Address,
    pub new_admin: Address,
}

/// Published by `create_duel`. The creator's stake is now escrowed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuelCreated {
    #[topic]
    pub duel_id: u64,
    #[topic]
    pub creator: Address,
    pub stake: i128,
    pub scc: u64,
    pub starts_at: u64,
    pub ends_at: u64,
    pub fee_bps: u32,
}

/// Published by `join_duel`. Both stakes are now escrowed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuelJoined {
    #[topic]
    pub duel_id: u64,
    #[topic]
    pub opponent: Address,
    pub total_pool: i128,
}

/// Published by `cancel_duel`, before the refunds (`DuelPayout`).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuelCancelled {
    #[topic]
    pub duel_id: u64,
    pub cancelled_by: Address,
    pub refunded: i128,
}

/// Published by `settle_duel`, before the transfers (`DuelPayout`).
/// `total_pool` is the USDC this settlement released: summing it over every
/// `duel_settled` event gives the tranche 3 metric (cumulative USDC settled).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuelSettled {
    #[topic]
    pub duel_id: u64,
    pub outcome: DuelOutcome,
    pub total_pool: i128,
    pub fee: i128,
}

/// One USDC transfer out of the escrow to a player: winnings, draw split or
/// refund. The platform fee is reported in `DuelSettled`, not here.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuelPayout {
    #[topic]
    pub duel_id: u64,
    #[topic]
    pub to: Address,
    pub amount: i128,
}

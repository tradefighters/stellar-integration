//! Events published by the contract.
//!
//! The indexer, the monitoring plan (tranche 2) and the on-chain target of
//! tranche 3 (cumulative USDC settled) are all computed from these events,
//! so their names and fields are part of the public interface.

use soroban_sdk::{contractevent, Address};

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

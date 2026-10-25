//! Storage keys and TTL handling.
//!
//! Soroban entries expire unless their TTL is extended. Instance storage
//! (config, counters) is extended on every call; each duel entry is extended
//! whenever it is touched, long enough to outlive its window plus disputes.

use soroban_sdk::{contracttype, Env};

use crate::types::{Config, Duel};

/// Ledgers per day at ~5 s per ledger.
pub const DAY_IN_LEDGERS: u32 = 17_280;

/// Extend instance storage when its TTL drops under 7 days...
pub const INSTANCE_TTL_THRESHOLD: u32 = 7 * DAY_IN_LEDGERS;
/// ...back up to 30 days.
pub const INSTANCE_TTL_EXTEND_TO: u32 = 30 * DAY_IN_LEDGERS;

/// Extend a duel entry when its TTL drops under 30 days...
pub const DUEL_TTL_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;
/// ...back up to 90 days.
pub const DUEL_TTL_EXTEND_TO: u32 = 90 * DAY_IN_LEDGERS;

/// All storage keys used by the contract.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Instance: the `Config`.
    Config,
    /// Instance: id the next duel will get.
    NextDuelId,
    /// Persistent: one `Duel` per id.
    Duel(u64),
}

/// Extends the instance TTL. Called at the start of every entrypoint.
pub fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

/// Reads the config. Always present: the constructor writes it.
pub fn get_config(env: &Env) -> Config {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .expect("config is set by the constructor")
}

/// Writes the config.
pub fn set_config(env: &Env, config: &Config) {
    env.storage().instance().set(&DataKey::Config, config);
}

/// Returns the next duel id and increments the counter.
#[expect(dead_code, reason = "used by create_duel, next step")]
pub fn next_duel_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .instance()
        .get(&DataKey::NextDuelId)
        .unwrap_or(1);
    env.storage()
        .instance()
        .set(&DataKey::NextDuelId, &(id + 1));
    id
}

/// Reads a duel, extending its TTL when found.
pub fn get_duel(env: &Env, id: u64) -> Option<Duel> {
    let key = DataKey::Duel(id);
    let duel: Option<Duel> = env.storage().persistent().get(&key);
    if duel.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, DUEL_TTL_THRESHOLD, DUEL_TTL_EXTEND_TO);
    }
    duel
}

/// Writes a duel and extends its TTL.
#[expect(dead_code, reason = "used by the duel entrypoints, next step")]
pub fn set_duel(env: &Env, duel: &Duel) {
    let key = DataKey::Duel(duel.id);
    env.storage().persistent().set(&key, duel);
    env.storage()
        .persistent()
        .extend_ttl(&key, DUEL_TTL_THRESHOLD, DUEL_TTL_EXTEND_TO);
}

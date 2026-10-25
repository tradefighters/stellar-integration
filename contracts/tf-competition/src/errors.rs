use soroban_sdk::contracterror;

/// Every way an entrypoint can refuse. Codes are stable: the API and the
/// indexer map them, so never renumber an existing variant.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Config failed validation (fee too high, min > max, ...).
    InvalidConfig = 1,
    /// `set_config` tried to change the admin; use `set_admin`.
    AdminChangeNotAllowed = 2,
    /// No duel with this id.
    DuelNotFound = 10,
    /// The duel is not in the status this entrypoint requires.
    InvalidStatus = 11,
    /// Stake outside `[min_entry_fee, max_entry_fee]` or not positive.
    InvalidAmount = 12,
    /// `starts_at` earlier than now + `min_start_delay`.
    InvalidStart = 13,
    /// `ends_at` not after `starts_at`.
    InvalidEnd = 14,
    /// The creator cannot join their own duel.
    CreatorSameAsOpponent = 15,
    /// The duel already started or ended.
    DuelExpired = 16,
    /// The winner is not one of the two players.
    InvalidWinner = 17,
    /// The duel has not ended yet.
    DuelNotEnded = 18,
    /// Checked arithmetic overflowed.
    MathOverflow = 30,
}

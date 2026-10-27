extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    Address, Env, IntoVal,
};

use crate::{
    split_pool, Config, Duel, DuelOutcome, DuelParams, DuelStatus, Error, TfCompetition,
    TfCompetitionClient, MIN_SCC,
};

/// 1 USDC in base units (7 decimals on Stellar).
const USDC: i128 = 10_000_000;
/// Stake used by most tests: 10 USDC.
const STAKE: i128 = 10 * USDC;
/// Ledger time the tests start at.
const T0: u64 = 1_000_000;
/// `min_start_delay` in the test config.
const DELAY: u64 = 60;

struct Setup<'a> {
    env: Env,
    client: TfCompetitionClient<'a>,
    usdc: TokenClient<'a>,
    config: Config,
    alice: Address,
    bob: Address,
}

/// Deploys the contract with a 5 % fee and gives Alice and Bob 1 000 USDC each.
/// Every auth is mocked: tests about auth build their own `Env`.
fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = T0);

    let issuer = Address::generate(&env);
    let usdc_id = env.register_stellar_asset_contract_v2(issuer).address();
    let config = Config {
        admin: Address::generate(&env),
        resolver: Address::generate(&env),
        treasury: Address::generate(&env),
        usdc: usdc_id.clone(),
        fee_bps: 500,
        creator_fee_bps: 0,
        min_entry_fee: USDC,
        max_entry_fee: 1_000 * USDC,
        min_start_delay: DELAY,
    };
    let contract_id = env.register(TfCompetition, (config.clone(),));
    let client = TfCompetitionClient::new(&env, &contract_id);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let minter = StellarAssetClient::new(&env, &usdc_id);
    minter.mint(&alice, &(1_000 * USDC));
    minter.mint(&bob, &(1_000 * USDC));

    Setup {
        usdc: TokenClient::new(&env, &usdc_id),
        env,
        client,
        config,
        alice,
        bob,
    }
}

/// Valid params: starts right after the delay, lasts one hour.
fn params(stake: i128) -> DuelParams {
    DuelParams {
        stake,
        scc: 1_000_000_000, // $1 000 starting capital
        starts_at: T0 + DELAY,
        ends_at: T0 + DELAY + 3_600,
    }
}

fn contract_error(e: Error) -> Result<soroban_sdk::Error, soroban_sdk::InvokeError> {
    Ok(soroban_sdk::Error::from_contract_error(e as u32))
}

fn set_time(env: &Env, t: u64) {
    env.ledger().with_mut(|l| l.timestamp = t);
}

/// Alice creates, Bob joins. Returns the duel id.
fn active_duel(s: &Setup) -> u64 {
    let id = s.client.create_duel(&s.alice, &params(STAKE));
    s.client.join_duel(&id, &s.bob);
    id
}

// ---------------------------------------------------------------------------
// create_duel
// ---------------------------------------------------------------------------

#[test]
fn create_escrows_the_stake() {
    let s = setup();
    let id = s.client.create_duel(&s.alice, &params(STAKE));

    assert_eq!(id, 1);
    assert_eq!(s.usdc.balance(&s.alice), 990 * USDC);
    assert_eq!(s.usdc.balance(&s.client.address), STAKE);

    let duel: Duel = s.client.duel(&id).unwrap();
    assert_eq!(duel.status, DuelStatus::Open);
    assert_eq!(duel.creator, s.alice);
    assert_eq!(duel.opponent, None);
    assert_eq!(duel.total_pool, STAKE);
    assert_eq!(duel.fee_bps, 500);
    assert_eq!(duel.created_at, T0);

    // Ids increase.
    assert_eq!(s.client.create_duel(&s.bob, &params(STAKE)), 2);
}

#[test]
fn create_rejects_bad_params() {
    let s = setup();
    let c = &s.client;

    let below = params(USDC - 1);
    assert_eq!(
        c.try_create_duel(&s.alice, &below),
        Err(contract_error(Error::InvalidAmount))
    );
    let above = params(1_000 * USDC + 1);
    assert_eq!(
        c.try_create_duel(&s.alice, &above),
        Err(contract_error(Error::InvalidAmount))
    );

    let mut low_scc = params(STAKE);
    low_scc.scc = MIN_SCC - 1;
    assert_eq!(
        c.try_create_duel(&s.alice, &low_scc),
        Err(contract_error(Error::SccOutOfRange))
    );

    let mut too_soon = params(STAKE);
    too_soon.starts_at = T0 + DELAY - 1;
    assert_eq!(
        c.try_create_duel(&s.alice, &too_soon),
        Err(contract_error(Error::InvalidStart))
    );

    let mut no_window = params(STAKE);
    no_window.ends_at = no_window.starts_at;
    assert_eq!(
        c.try_create_duel(&s.alice, &no_window),
        Err(contract_error(Error::InvalidEnd))
    );

    // Nothing moved.
    assert_eq!(s.usdc.balance(&c.address), 0);
}

#[test]
#[should_panic]
fn create_needs_the_creator_signature() {
    let s = setup();
    let p = params(STAKE);
    // Bob signs a duel in Alice's name.
    s.client
        .mock_auths(&[MockAuth {
            address: &s.bob,
            invoke: &MockAuthInvoke {
                contract: &s.client.address,
                fn_name: "create_duel",
                args: (s.alice.clone(), p.clone()).into_val(&s.env),
                sub_invokes: &[],
            },
        }])
        .create_duel(&s.alice, &p);
}

#[test]
fn create_fails_without_funds() {
    let s = setup();
    let broke = Address::generate(&s.env);
    assert!(s.client.try_create_duel(&broke, &params(STAKE)).is_err());
    // The failed call left no duel behind.
    assert!(s.client.duel(&1).is_none());
}

// ---------------------------------------------------------------------------
// join_duel
// ---------------------------------------------------------------------------

#[test]
fn join_escrows_the_second_stake() {
    let s = setup();
    let id = active_duel(&s);

    let duel = s.client.duel(&id).unwrap();
    assert_eq!(duel.status, DuelStatus::Active);
    assert_eq!(duel.opponent, Some(s.bob.clone()));
    assert_eq!(duel.total_pool, 2 * STAKE);
    assert_eq!(s.usdc.balance(&s.bob), 990 * USDC);
    assert_eq!(s.usdc.balance(&s.client.address), 2 * STAKE);
}

#[test]
fn join_refusals() {
    let s = setup();
    let c = &s.client;
    let id = c.create_duel(&s.alice, &params(STAKE));

    assert_eq!(
        c.try_join_duel(&id, &s.alice),
        Err(contract_error(Error::CreatorSameAsOpponent))
    );
    assert_eq!(
        c.try_join_duel(&99, &s.bob),
        Err(contract_error(Error::DuelNotFound))
    );

    set_time(&s.env, T0 + DELAY);
    assert_eq!(
        c.try_join_duel(&id, &s.bob),
        Err(contract_error(Error::DuelExpired))
    );

    set_time(&s.env, T0);
    c.join_duel(&id, &s.bob);
    let carol = Address::generate(&s.env);
    assert_eq!(
        c.try_join_duel(&id, &carol),
        Err(contract_error(Error::InvalidStatus))
    );
}

// ---------------------------------------------------------------------------
// cancel_duel
// ---------------------------------------------------------------------------

#[test]
fn creator_cancels_an_open_duel() {
    let s = setup();
    let id = s.client.create_duel(&s.alice, &params(STAKE));
    s.client.cancel_duel(&id, &s.alice);

    assert_eq!(s.client.duel(&id).unwrap().status, DuelStatus::Cancelled);
    assert_eq!(s.usdc.balance(&s.alice), 1_000 * USDC);
    assert_eq!(s.usdc.balance(&s.client.address), 0);
}

#[test]
fn resolver_cancels_an_active_duel_before_start() {
    let s = setup();
    let id = active_duel(&s);
    s.client.cancel_duel(&id, &s.config.resolver);

    assert_eq!(s.usdc.balance(&s.alice), 1_000 * USDC);
    assert_eq!(s.usdc.balance(&s.bob), 1_000 * USDC);
    assert_eq!(s.usdc.balance(&s.client.address), 0);
}

#[test]
fn cancel_refusals() {
    let s = setup();
    let c = &s.client;
    let stranger = Address::generate(&s.env);

    let open = c.create_duel(&s.alice, &params(STAKE));
    assert_eq!(
        c.try_cancel_duel(&open, &stranger),
        Err(contract_error(Error::Unauthorized))
    );

    let active = active_duel(&s);
    // Once both stakes are in, a player alone can't pull out.
    assert_eq!(
        c.try_cancel_duel(&active, &s.alice),
        Err(contract_error(Error::Unauthorized))
    );
    assert_eq!(
        c.try_cancel_duel(&active, &s.bob),
        Err(contract_error(Error::Unauthorized))
    );

    // After the start, use settle_duel(Issue).
    set_time(&s.env, T0 + DELAY);
    let resolver = &s.config.resolver;
    assert_eq!(
        c.try_cancel_duel(&active, resolver),
        Err(contract_error(Error::DuelExpired))
    );

    c.cancel_duel(&open, &s.alice);
    assert_eq!(
        c.try_cancel_duel(&open, &s.alice),
        Err(contract_error(Error::InvalidStatus))
    );
}

// ---------------------------------------------------------------------------
// settle_duel
// ---------------------------------------------------------------------------

/// Moves time to the end of the duel from `params`.
fn end_duel(s: &Setup) {
    set_time(&s.env, T0 + DELAY + 3_600);
}

#[test]
fn win_pays_pool_minus_fee() {
    let s = setup();
    let id = active_duel(&s);
    end_duel(&s);
    s.client.settle_duel(&id, &DuelOutcome::Win(s.bob.clone()));

    // Pool 20, fee 5 % = 1, winner gets 19.
    assert_eq!(s.usdc.balance(&s.bob), 990 * USDC + 19 * USDC);
    assert_eq!(s.usdc.balance(&s.alice), 990 * USDC);
    assert_eq!(s.usdc.balance(&s.config.treasury), USDC);
    assert_eq!(s.usdc.balance(&s.client.address), 0);

    let duel = s.client.duel(&id).unwrap();
    assert_eq!(duel.status, DuelStatus::Settled);
    assert_eq!(duel.total_pool, 0);
}

#[test]
fn draw_splits_the_fee() {
    let s = setup();
    let id = active_duel(&s);
    end_duel(&s);
    s.client.settle_duel(&id, &DuelOutcome::Draw);

    // Fee 1 USDC, each player pays half: 9.5 back each.
    let back = STAKE - USDC / 2;
    assert_eq!(s.usdc.balance(&s.alice), 990 * USDC + back);
    assert_eq!(s.usdc.balance(&s.bob), 990 * USDC + back);
    assert_eq!(s.usdc.balance(&s.config.treasury), USDC);
    assert_eq!(s.usdc.balance(&s.client.address), 0);
}

#[test]
fn issue_refunds_in_full() {
    let s = setup();
    let id = active_duel(&s);
    end_duel(&s);
    s.client.settle_duel(&id, &DuelOutcome::Issue);

    assert_eq!(s.usdc.balance(&s.alice), 1_000 * USDC);
    assert_eq!(s.usdc.balance(&s.bob), 1_000 * USDC);
    assert_eq!(s.usdc.balance(&s.config.treasury), 0);
    assert_eq!(s.usdc.balance(&s.client.address), 0);
}

#[test]
fn settle_refusals() {
    let s = setup();
    let c = &s.client;
    let open = c.create_duel(&s.alice, &params(STAKE));
    let id = active_duel(&s);

    let win_bob = DuelOutcome::Win(s.bob.clone());
    assert_eq!(
        c.try_settle_duel(&id, &win_bob),
        Err(contract_error(Error::DuelNotEnded))
    );

    end_duel(&s);
    let carol = DuelOutcome::Win(Address::generate(&s.env));
    assert_eq!(
        c.try_settle_duel(&id, &carol),
        Err(contract_error(Error::InvalidWinner))
    );
    assert_eq!(
        c.try_settle_duel(&open, &win_bob),
        Err(contract_error(Error::InvalidStatus))
    );
    assert_eq!(
        c.try_settle_duel(&99, &win_bob),
        Err(contract_error(Error::DuelNotFound))
    );

    c.settle_duel(&id, &win_bob);
    assert_eq!(
        c.try_settle_duel(&id, &win_bob),
        Err(contract_error(Error::InvalidStatus))
    );
}

#[test]
#[should_panic]
fn settle_needs_the_resolver_signature() {
    let s = setup();
    let id = active_duel(&s);
    end_duel(&s);
    let outcome = DuelOutcome::Win(s.alice.clone());
    // Alice tries to declare herself the winner.
    s.client
        .mock_auths(&[MockAuth {
            address: &s.alice,
            invoke: &MockAuthInvoke {
                contract: &s.client.address,
                fn_name: "settle_duel",
                args: (id, outcome.clone()).into_val(&s.env),
                sub_invokes: &[],
            },
        }])
        .settle_duel(&id, &outcome);
}

#[test]
fn settle_signed_by_the_resolver_only() {
    let s = setup();
    let id = active_duel(&s);
    end_duel(&s);
    s.client.settle_duel(&id, &DuelOutcome::Draw);

    let auths = s.env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, s.config.resolver);
}

// ---------------------------------------------------------------------------
// Config interplay and pool math
// ---------------------------------------------------------------------------

#[test]
fn fee_is_fixed_at_creation() {
    let s = setup();
    let id = active_duel(&s);

    let mut next = s.config.clone();
    next.fee_bps = 2_000;
    s.client.set_config(&next);

    end_duel(&s);
    s.client
        .settle_duel(&id, &DuelOutcome::Win(s.alice.clone()));
    // Still 5 %, not 20 %.
    assert_eq!(s.usdc.balance(&s.config.treasury), USDC);
}

#[test]
fn usdc_token_cannot_change() {
    let s = setup();
    let mut next = s.config.clone();
    next.usdc = Address::generate(&s.env);
    assert_eq!(
        s.client.try_set_config(&next),
        Err(contract_error(Error::TokenChangeNotAllowed))
    );
}

#[test]
fn split_conserves_the_pool_with_rounding() {
    let env = Env::default();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    // Odd amounts so both roundings kick in: pool 20_000_002, fee floor(1_000_000.1).
    let duel = Duel {
        id: 1,
        creator: alice.clone(),
        opponent: Some(bob.clone()),
        stake: 10_000_001,
        total_pool: 20_000_002,
        scc: MIN_SCC,
        starts_at: 0,
        ends_at: 1,
        status: DuelStatus::Active,
        fee_bps: 500,
        created_at: 0,
    };

    for outcome in [
        DuelOutcome::Win(alice),
        DuelOutcome::Win(bob),
        DuelOutcome::Draw,
        DuelOutcome::Issue,
    ] {
        let split = split_pool(&env, &duel, &outcome);
        assert_eq!(
            split.to_creator + split.to_opponent + split.fee,
            duel.total_pool
        );
    }
    let draw = split_pool(&env, &duel, &DuelOutcome::Draw);
    assert_eq!(draw.fee, 1_000_000);
    assert_eq!(draw.to_creator, 9_500_001);
}

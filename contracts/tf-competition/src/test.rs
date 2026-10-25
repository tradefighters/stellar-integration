extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};

use crate::{Config, Error, TfCompetition, TfCompetitionClient};

/// 1 USDC in base units (7 decimals on Stellar).
const USDC: i128 = 10_000_000;

/// A valid config with a real Stellar Asset Contract standing in for USDC.
fn test_config(env: &Env) -> Config {
    let issuer = Address::generate(env);
    let usdc = env.register_stellar_asset_contract_v2(issuer).address();
    Config {
        admin: Address::generate(env),
        resolver: Address::generate(env),
        treasury: Address::generate(env),
        usdc,
        fee_bps: 500,
        creator_fee_bps: 2_000,
        min_entry_fee: USDC,
        max_entry_fee: 10_000 * USDC,
        min_start_delay: 60,
    }
}

fn deploy<'a>(env: &Env, config: &Config) -> TfCompetitionClient<'a> {
    let id = env.register(TfCompetition, (config.clone(),));
    TfCompetitionClient::new(env, &id)
}

#[test]
fn constructor_stores_config_and_emits_event() {
    let env = Env::default();
    let config = test_config(&env);
    let client = deploy(&env, &config);

    // Events only cover the last invocation: check right after the deploy.
    assert_eq!(env.events().all().events().len(), 1);
    assert_eq!(client.config(), config);
    assert!(client.duel(&1).is_none());
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn constructor_rejects_fee_above_cap() {
    let env = Env::default();
    let mut config = test_config(&env);
    config.fee_bps = crate::MAX_FEE_BPS + 1;
    deploy(&env, &config);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn constructor_rejects_min_above_max() {
    let env = Env::default();
    let mut config = test_config(&env);
    config.min_entry_fee = config.max_entry_fee + 1;
    deploy(&env, &config);
}

#[test]
fn admin_can_update_config() {
    let env = Env::default();
    env.mock_all_auths();
    let config = test_config(&env);
    let client = deploy(&env, &config);

    let mut next = config.clone();
    next.fee_bps = 300;
    next.resolver = Address::generate(&env);
    client.set_config(&next);

    // The admin, and only the admin, had to sign (auths cover the last call only).
    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, config.admin);
    assert_eq!(client.config(), next);
}

#[test]
#[should_panic]
fn set_config_without_admin_auth_fails() {
    let env = Env::default();
    let config = test_config(&env);
    let client = deploy(&env, &config);

    let mut next = config.clone();
    next.fee_bps = 0;
    // A stranger signs instead of the admin.
    let stranger = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &stranger,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "set_config",
                args: (next.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_config(&next);
}

#[test]
fn set_config_cannot_change_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let config = test_config(&env);
    let client = deploy(&env, &config);

    let mut next = config.clone();
    next.admin = Address::generate(&env);
    let result = client.try_set_config(&next);
    let expected = soroban_sdk::Error::from_contract_error(Error::AdminChangeNotAllowed as u32);
    assert_eq!(result, Err(Ok(expected)));
}

#[test]
fn set_admin_needs_both_signatures() {
    let env = Env::default();
    env.mock_all_auths();
    let config = test_config(&env);
    let client = deploy(&env, &config);

    let new_admin = Address::generate(&env);
    client.set_admin(&new_admin);

    let signers: std::vec::Vec<Address> = env.auths().iter().map(|(a, _)| a.clone()).collect();
    assert!(signers.contains(&config.admin));
    assert!(signers.contains(&new_admin));
    assert_eq!(client.config().admin, new_admin);
}

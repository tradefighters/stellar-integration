# Soroban setup — research notes and decisions

How the contract workspace was set up, the versions it is pinned to and why,
and the commands to build, test and deploy it. Facts below were checked on
**2026-10-25**; re-check the versions before a mainnet deploy.

---

## 1. Toolchain and versions

| Tool | Version | Why |
| --- | --- | --- |
| Stellar protocol | **29** on testnet and mainnet | from `getNetwork` on `soroban-testnet.stellar.org` and `mainnet.sorobanrpc.com` |
| `soroban-sdk` | **29.0.1** | latest stable; the SDK major version follows the protocol. `30.0.0-rc.1` exists but targets a protocol that is not live yet |
| `stellar-cli` | **28.1.0** | latest release (2026-09-26); needs `soroban-sdk` ≥ 28 build tooling, which it has |
| Rust | stable (≥ 1.93) | SDK 29 needs ≥ 1.91, stellar-cli 28 needs ≥ 1.93 |
| WASM target | `wasm32v1-none` | the only target Soroban accepts since SDK 22; installed by `rust-toolchain.toml` |

Notes:

- Always build with `stellar contract build`, never with plain
  `cargo build --target wasm32v1-none`: the CLI adds the metadata the network expects.
- SDK 28 breaking changes that apply to us: no `export =` / `lib =` on
  `contracttype` / `contracterror` / `contractevent`; in map-format events
  unset `Option` fields are left out (the indexer must handle missing keys);
  `deploy_contract` and `update_current_contract` replace the `_v2` /
  `_wasm` variants. SDK 29.0.0 → 29.0.1 only fixes the bundled VM version.
- The workspace layout is the one `stellar contract init` generates
  (`contracts/*` members, `opt-level = "z"`, `overflow-checks = true`).
  [Scaffold Stellar](https://github.com/AhaLabs/scaffold-stellar) adds a
  frontend template we don't need. Its watch mode and Registry publishing
  can be added later on top of this layout without moving anything.

## 2. Assets and network addresses

| | Testnet | Mainnet |
| --- | --- | --- |
| USDC issuer (Circle) | `GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5` | `GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN` |
| USDC Stellar Asset Contract | `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` | `stellar contract id asset --asset USDC:GA5Z…KZVN --network mainnet` |
| RPC | `https://soroban-testnet.stellar.org` | provider to be chosen |
| Friendbot | `https://friendbot.stellar.org` | — |
| Explorer | `https://stellar.expert/explorer/testnet` | `https://stellar.expert/explorer/public` |

The testnet SAC id was computed with
`stellar contract id asset --asset USDC:GBBD…LFLA5 --network testnet`.
Issuers are from Circle's
[USDC contract addresses](https://developers.circle.com/stablecoins/usdc-contract-addresses) page.

**Decimals:** USDC on Stellar has **7** decimals; on Solana it has 6. Every
amount crossing between the API and the contract must be scaled
(1 USDC = `10_000_000` on Stellar).

## 3. Porting the production Solana duel

The Solana program (`TFrXd1Fm4FoWdhzqm7qxL7dD8yqzDw3kf8Cjt9gVgZx`, IDL in the
product monorepo at `packages/program-sdk/idl/duel-mainnet.json`) has these
duel instructions. The Soroban side maps them like this:

| Solana instruction | Soroban entrypoint | Status |
| --- | --- | --- |
| `initialize_config` | `__constructor(config)` | done |
| `update_config`, `update_config_admin` | `set_config`, `set_admin` | done |
| `initialize(seed, DuelParams)` | `create_duel(creator, params) -> id` | done |
| `join` | `join_duel(id, player)` | done |
| `cancel` | `cancel_duel(id, caller)` | done |
| `settle(DuelResult, winner)` | `settle_duel(id, DuelOutcome)` | done |
| `add_duel_prize` / `withdraw_duel_contribution` | `add_duel_prize` / `withdraw_duel_contribution` | tranche 1 |
| `close_duel` | not needed: no rent to reclaim; entries expire by TTL once settled | — |

Decisions taken during the init:

- **Constructor instead of `initialize`.** `__constructor` runs in the same
  transaction as the deploy, so nobody can front-run the initialization.
  The config is passed as a deploy argument.
- **Duel id is a counter** (`NextDuelId` in instance storage) instead of a
  client-chosen `seed`. The id is returned by `create_duel` and published in
  the event.
- **Outcomes keep the three production cases:** `Win(address)`, `Draw`,
  `Issue` (could not be scored → full refund, no fee).
- **Statuses:** `Open → Active → Settled`, or `→ Cancelled`. "Waiting for the
  start" vs "trading" comes from `starts_at` and is not stored.
- **`scc`** (virtual starting capital used by the off-chain margin engine) is
  kept as on Solana: `u64`, USD with 6 decimals.
- **Fee bound:** `fee_bps` is capped at 2 000 (20 %) in `validate_config`,
  so a bad config can never take most of a pool. The rate is copied into
  each duel at creation, and the USDC token can't change after the deploy.
- **Fees follow production:** 5 % of the pool (`platformFees` default in the
  product database). On a `Win` the winner gets the pool minus the fee; on a
  `Draw` each player pays half of it, as the Solana resolver does; `Issue`
  refunds in full.
- **Join before the start only**, so both players trade the full window.
- **Cancel:** the creator or the resolver while the duel is `Open`; only the
  resolver once both stakes are in, and only before `starts_at`. After the
  start a duel that can't be scored is closed with `settle_duel(Issue)`.
- **`set_admin` needs both signatures** (current and new admin), so the role
  can't be sent to an address nobody controls.
- **TTL:** instance storage is extended to 30 days on every call; a duel
  entry is extended to 90 days whenever it is touched.

## 4. Commands

```sh
# one-time
rustup toolchain install          # reads rust-toolchain.toml
# install stellar-cli 28.1.0: see .github/workflows/ci.yml, or
#   cargo install --locked stellar-cli@28.1.0

make check                        # fmt + clippy + tests + WASM build (what CI runs)
make test                         # cargo test
make build                        # stellar contract build
```

## 5. Testnet deployment

| | |
| --- | --- |
| Contract id | [`CBQB5S2RCMLO7ZX2CP3CBPCOJZIYVY5AZAZIWP2KAFYJ2BNBRRZOJQXE`](https://stellar.expert/explorer/testnet/contract/CBQB5S2RCMLO7ZX2CP3CBPCOJZIYVY5AZAZIWP2KAFYJ2BNBRRZOJQXE) |
| Token | Circle testnet USDC, SAC `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` |
| Config | fee 5 %, stake 1 – 1 000 USDC, `min_start_delay` 60 s |
| Admin | `GBYCIS4QKGFAK7AQJIDCNDQ55HG7N5N2U637QBTYQKD665FGJCFQRXCP` |
| Resolver | `GA6T2PPNRDHKA5C25KJJ5QQ3GZLRL3KBDFU4AXRNYETE6EHMQDJBXMBG` |
| Treasury | `GAKNLE5PTYPCH34253HG4KWCVJWFN52W3RVHO5DOIB6BPT5AO7G7QNVC` |

Deployed with:

```sh
stellar keys generate tf-admin --network testnet --fund   # same for tf-resolver, tf-treasury
stellar contract deploy \
  --wasm target/wasm32v1-none/release/tf_competition.wasm \
  --source-account tf-admin --network testnet --alias tf-competition \
  -- --config '{
    "admin": "<G… tf-admin>",
    "resolver": "<G… tf-resolver>",
    "treasury": "<G… tf-treasury>",
    "usdc": "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA",
    "fee_bps": 500,
    "creator_fee_bps": 0,
    "min_entry_fee": "10000000",
    "max_entry_fee": "10000000000",
    "min_start_delay": 60
  }'
```

**Getting testnet USDC** without the Circle faucet: testnet has a deep
XLM/USDC liquidity pool, so a funded account can swap.

```sh
USDC=USDC:GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5
stellar keys generate tf-player-a --network testnet --fund
stellar tx new change-trust --source-account tf-player-a --line $USDC --network testnet
stellar tx new path-payment-strict-send --source-account tf-player-a \
  --send-asset native --send-amount 3000000000 \
  --destination "$(stellar keys address tf-player-a)" \
  --dest-asset $USDC --dest-min 200000000 --network testnet
```

The treasury needs the USDC trustline too, to receive fees.

**Demo:** `scripts/testnet-demo.sh <contract-id> [stake]` runs a win, a draw
and a cancellation and prints the transactions as a markdown table (needs
`tf-resolver`, `tf-player-a`, `tf-player-b`). The table from the last run is
in the README.

## 6. Next steps

1. Generated TypeScript client (`stellar contract bindings typescript`) for
   the API and the web app.
2. Battle royales, merkle claims, prize top-ups and `tf-rewards` (tranche 1).
3. A demo video / GIF for the README.

## 7. Open questions

- **`min_start_delay`:** 60 s on testnet for the demo. Which value for mainnet?
- **Keys:** the testnet admin / resolver / treasury keys were generated for
  the demo. Before mainnet: who holds them, and a multisig for the admin?

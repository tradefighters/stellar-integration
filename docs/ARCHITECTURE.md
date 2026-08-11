# Technical Architecture

How the TradeFighters product — already running competitions in production —
extends onto Stellar. This document covers the layering, the runtime flows, the
data model, and the failure handling. Contract internals live in
[CONTRACTS.md](CONTRACTS.md); the exchange adapter in
[NOETHER-ADAPTER.md](NOETHER-ADAPTER.md).

## Contents

1. [Existing foundation](#1-existing-foundation)
2. [Layered view](#2-layered-view)
3. [Trading path](#3-trading-path)
4. [Indexing path](#4-indexing-path)
5. [Settlement path](#5-settlement-path)
6. [Identity model](#6-identity-model)
7. [Deployment topology](#7-deployment-topology)
8. [Failure modes](#8-failure-modes)

---

## 1. Existing foundation

The product monorepo is a Turborepo workspace: Next.js 15 / React 19 web app,
NestJS 11 API, PostgreSQL 16 with Prisma. Its multi-exchange design is
hexagonal — application code speaks only unified domain types, and each venue
is an adapter behind a port.

```mermaid
flowchart TB
    APP["App code — web · api · resolver-v2<br/>speaks ONLY unified domain types"]

    subgraph core["@repo/exchange-core"]
        direction LR
        PORTS["MarketDataPort · AccountDataPort<br/>TradingPort · StreamPort"]
        TYPES["UnifiedMarket · UnifiedCandle · UnifiedPosition<br/>UnifiedFillsPage · PlaceOrderIntent · OrderAck"]
        CAPS["ExchangeCapabilities<br/>feature flags, never venue names"]
        REG["ExchangeRegistry"]
    end

    PAC["PacificaAdapter<br/>Solana"]
    HL["HyperliquidAdapter<br/>EVM"]
    PHX["PhoenixAdapter<br/>Solana"]
    NOE["NoetherAdapter<br/>Stellar / Soroban — NEW"]

    APP --> core
    core --> PAC
    core --> HL
    core --> PHX
    core --> NOE

    style NOE fill:#0d3b66,color:#ffffff,stroke:#7fb2f0
```

Four principles carry over unchanged:

1. **Unified domain types at the boundary.** Adapters normalize wire formats
   once, at the edge. Decimal-string amounts, epoch-ms timestamps, symbols
   scoped per exchange and compared as `(exchange, symbol)`.
2. **Capability flags, not a lowest common denominator.** Each adapter
   publishes an `ExchangeCapabilities` descriptor — combined TP/SL orders?
   TP triggers? candle caps? fill retention depth? App code branches on flags.
   Adding Noether means adding a descriptor, not `if (exchange === ...)`.
3. **Adapters own signing end-to-end.** Callers send intents; the adapter picks
   the signer and the wire mechanics. Pacifica uses sorted-JSON ed25519,
   Hyperliquid uses EIP-712 agent keys, Noether uses signed Soroban XDR.
4. **One composition root per app.** The API's global `ExchangeModule` builds
   the registry with every adapter; consumers inject the registry.

Adding Stellar therefore means: one adapter package, one capability descriptor,
one `ExchangeId`, and two new contracts — not a fork of the product.

### What has to change outside the adapter

| Concern | Change |
| --- | --- |
| `ExchangeId` | add `'noether'` to `EXCHANGE_IDS`; mirror `NOETHER` in the Prisma `Exchange` enum |
| `ExchangeChainType` | add `'stellar'` alongside `'solana' \| 'evm'` |
| Trading identity | `user_exchange_accounts` gains Stellar addresses (G...) |
| Competition gating | `duels.allowed_exchanges` / `tournaments.allowed_exchanges` accept `NOETHER` |
| Escrow | competition rows carry a chain discriminator: Solana program vs `tf-competition` |

Because trading addresses are format-disjoint across chains (base58 vs `0x`
vs `G...`), the mirrored-fills table's `(account, history_id)` uniqueness stays
valid with no schema surgery.

---

## 2. Layered view

```mermaid
flowchart TB
    subgraph L1["Presentation"]
        TERM["Perps terminal<br/>orderbook · chart · order form"]
        COMP["Competition UI<br/>lobby · live ranking · claims"]
        DEP["Deposit widget<br/>bridge + swap routes"]
    end

    subgraph L2["Client integration"]
        WKIT["Stellar Wallets Kit"]
        NSDK["@repo/noether-sdk"]
        GEN["generated contract clients"]
    end

    subgraph L3["Application services — NestJS"]
        AUTH["Auth: challenge / session / API-key provisioning"]
        MIRROR["Fill mirror: WS-first, REST reconcile"]
        INGEST["Event ingestor: Soroban events to Postgres"]
        RES["resolver-v2: virtual margin engine"]
        JOBS["BullMQ schedulers: start / complete jobs"]
    end

    subgraph L4["Persistence"]
        PG[("PostgreSQL 16 + Prisma")]
        REDIS[("Redis — queues, live ranking")]
    end

    subgraph L5["Stellar"]
        RPC["Soroban RPC + Horizon"]
        NOETH["Noether contracts + API"]
        TFC["tf-competition"]
        TFR["tf-rewards"]
        USDC["USDC — Stellar Asset Contract"]
    end

    L1 --> L2
    L2 --> L3
    L2 -->|"signed XDR"| L5
    L3 --> L4
    L3 --> L5
```

The client talks to Stellar directly for anything requiring a signature. The
API never proxies a signing request — it only reads, indexes, and submits its
own operator-signed settlement transactions.

---

## 3. Trading path

Noether exposes a prepare–sign–submit flow so that the venue builds the
transaction and the user authorizes it. The adapter is the orchestrator.

```mermaid
sequenceDiagram
    autonumber
    participant UI as Order form
    participant AD as NoetherAdapter
    participant REST as Noether REST /v1
    participant WK as Wallets Kit
    participant SUB as Noether submit
    participant SOR as Soroban

    Note over UI,AD: intent in, OrderAck out — same as every other venue
    UI->>AD: placeOrder(PlaceOrderIntent)
    AD->>AD: validate against ExchangeCapabilities
    AD->>REST: POST /v1/orders/prepare
    REST-->>AD: unsigned XDR + simulation result
    AD->>AD: assert XDR matches the intent
    AD->>WK: signTransaction(xdr)
    WK-->>AD: signed XDR
    AD->>SUB: POST /v1/tx/submit
    SUB->>SOR: broadcast
    loop until finalized or timeout
        AD->>SUB: poll status
    end
    SUB-->>AD: finalized, tx hash, order id
    AD-->>UI: OrderAck
```

Step 4 is not decoration. Before a user is asked to sign, the adapter decodes
the returned XDR and asserts that the invoked contract, market, side, size, and
price bounds match the intent that produced it. A venue that returns a
different transaction than requested must fail closed, not get signed.

### Streams

The WebSocket client reuses the realtime resilience layer built for the
existing venues: a staleness watchdog that treats "connected but silent" as
failure, wake-from-sleep recovery, unbounded reconnect with backoff, a
snapshot replay flagged `isSnapshot: true` after each reconnect, and a
stale-data badge in the UI when ticks age past threshold.

---

## 4. Indexing path

Two independent ingest streams feed Postgres: contract events for competition
state, and trading fills for PnL.

```mermaid
flowchart LR
    subgraph src["Sources"]
        EV["tf-competition + tf-rewards<br/>contract events"]
        WS["Noether WebSocket<br/>fills, ticks"]
        RST["Noether REST<br/>fills backfill"]
    end

    subgraph ing["Ingestion"]
        CUR["Ledger cursor<br/>persisted per contract"]
        POLL["Soroban RPC getEvents<br/>paged from cursor"]
        NORM["Normalize to unified types"]
        DEDUP["Idempotent upsert<br/>on natural keys"]
    end

    PG[("PostgreSQL")]

    EV --> POLL --> NORM
    CUR <--> POLL
    WS --> NORM
    RST --> NORM
    NORM --> DEDUP --> PG
```

Design notes:

- **Cursor-driven, replayable.** The ingestor stores the last processed ledger
  sequence per contract. A restart resumes from the cursor; a forced replay
  rewinds it. Every write is an idempotent upsert on a natural key
  (`(contract, ledger, event_index)` for events, `(account, history_id)` for
  fills), so replay converges instead of duplicating.
- **WS-first, REST-reconcile.** The WebSocket gives low latency for live
  rankings; a periodic REST sweep over the competition window catches anything
  the socket dropped. This is the same two-source pattern the Hyperliquid fill
  mirror already uses in production.
- **Retention awareness.** `capabilities.fillHistoryDepth` tells the resolver
  how far back Noether's fill history reaches, so a window that predates the
  retention horizon is flagged rather than silently truncated.

---

## 5. Settlement path

The settlement pipeline is scheduled, not polled: creating a competition
schedules a start job at `starts_at` and a complete job shortly after
`ends_at`, giving in-flight fills time to land.

```mermaid
flowchart TB
    START(["complete job fires"]) --> FETCH["load competition + participants"]
    FETCH --> GUARD{"status IN_PROGRESS<br/>and past ends_at?"}
    GUARD -->|no| ABORT["abort, retry with backoff"]
    GUARD -->|yes| PNL["per participant:<br/>replay fills through resolver-v2"]
    PNL --> RANK["rank by PnL desc,<br/>tiebreak volume desc"]
    RANK --> XCHK["re-verify settlement-critical state<br/>against Soroban contract reads"]
    XCHK -->|mismatch beyond tolerance| HALT["halt + alert<br/>no root submitted"]
    XCHK -->|match| SPLIT["split pool:<br/>platform fee · creator share · prizes"]
    SPLIT --> CURVE["power-law payout curve<br/>over the winning cohort"]
    CURVE --> TREE["build merkle tree of<br/>(winner, amount) leaves"]
    TREE --> PERSIST["persist settlement + winners<br/>+ ranks in one DB transaction"]
    PERSIST --> ONCHAIN["settle on-chain:<br/>root + winner count"]
    ONCHAIN --> ACCRUE["tf-rewards accrues<br/>referral + creator balances"]
    ONCHAIN --> NOTIFY["notify: won / lost / settled"]
    ACCRUE --> DONE(["winners claim with proofs"])
    NOTIFY --> DONE
```

### Why the engine needed no changes

`resolver-v2` is a **virtual margin engine**. It simulates an account from a
configured start capital and never reads real account equity, so every
competitor is scored on identical terms regardless of what they hold outside
the competition. Multi-venue support is therefore an *input* problem, already
solved by symbol namespacing: with `namespaceSymbols: true`, positions,
margin, and candle series are keyed `exchange:symbol`, so a single virtual
account can hold `noether:BTC` and `hyperliquid:BTC` as distinct positions
inside one competition.

Adding Noether means supplying two things the existing builders already accept:
normalized `ResolverFill`s and a candle series per traded symbol. If Noether
exposes mark-price candles, they are used; otherwise the engine falls back to
trade candles, exactly as it does for venues without mark-price support.

### The verification gate

The step labelled `XCHK` is the trust boundary. Indexed projections are used
for speed everywhere except the number that decides payouts. Before any root is
submitted, final per-competitor PnL is recomputed from direct Soroban reads —
market contract state plus the SEP-40 oracle shim — and compared against the
replayed figure. Beyond tolerance, the pipeline halts and pages a human. It
never guesses, and it never settles from indexer data alone.

---

## 6. Identity model

A user has one trading identity per venue. Stellar adds a third address column
and a fourth signing scheme.

| | Pacifica | Hyperliquid | Noether |
| --- | --- | --- | --- |
| Chain family | Solana | EVM | Stellar |
| Trading address | Solana pubkey | `0x…` | `G…` |
| Wallet origin | wallet adapter or Privy embedded Solana | Privy embedded Ethereum | Stellar Wallets Kit or Privy |
| Signing | browser wallet / session signer | delegated agent key | browser wallet, signed XDR |
| Custody | non-custodial | delegated agent | non-custodial |

`user_exchange_accounts` stays the canonical `(user, exchange) → tradingAddress`
table, gaining Stellar rows with a per-exchange `is_initialized` flag.

Notably, Noether is the **more** non-custodial of the three: unlike the
Hyperliquid path, no server-held key signs on the user's behalf. Every order
requires a wallet signature.

---

## 7. Deployment topology

```mermaid
flowchart TB
    subgraph edge["Edge"]
        WEB["Next.js — Vercel"]
    end

    subgraph svc["Services"]
        API["NestJS API"]
        WRK["Workers: ingestor · mirror · resolver"]
    end

    subgraph data["Data"]
        PG[("PostgreSQL 16")]
        RD[("Redis — BullMQ")]
    end

    subgraph obs["Observability"]
        MON["Dashboards: ledger lag, WS staleness,<br/>settlement outcomes, claim rates"]
        ALERT["On-call alerting"]
    end

    subgraph net["Stellar"]
        RPC1["Soroban RPC — primary"]
        RPC2["Soroban RPC — fallback"]
        HOR["Horizon"]
        CH["tf-competition · tf-rewards · Noether"]
    end

    WEB --> API
    WEB -->|"signed XDR"| RPC1
    API --> PG
    API --> RD
    WRK --> PG
    WRK --> RD
    WRK --> RPC1
    RPC1 -.failover.-> RPC2
    WRK --> HOR
    RPC1 --> CH
    WRK --> MON --> ALERT
```

Environments: local (Scaffold Stellar watch mode against a local network),
testnet (public staging), mainnet. Contract addresses are configuration, never
compiled constants, so one build targets any network.

---

## 8. Failure modes

| Failure | Detection | Response |
| --- | --- | --- |
| Soroban RPC degraded | health probe, ledger lag threshold | fail over to secondary; ingestor resumes from cursor |
| WebSocket connected but silent | staleness watchdog | force reconnect, snapshot replay, stale badge in UI |
| Missed contract events | cursor gap check on ledger sequence | rewind cursor and replay; upserts are idempotent |
| Indexed PnL disagrees with chain | verification gate before settlement | halt, alert, no root submitted |
| Settlement transaction fails | submit + finalization polling | retry with backoff; settlement is idempotent per competition |
| Competition never gets a second player | start job validation | cancel path returns the creator's stake |
| Winner never claims | claim-rate dashboard | funds stay in escrow, claimable; no forced push |
| Double-claim attempt | claim receipt per `(competition, winner)` | contract rejects |

Two invariants make most of this recoverable: the ingestor is replayable
because writes are idempotent, and settlement is idempotent per competition
because the root is written once and claims are gated by receipts.

# Onboarding: Wallets, Auth, and Funding

Two barriers stand between a Stellar user and their first competition: getting
signed in, and getting USDC into the wallet they signed in with. This document
covers both.

---

## Authentication

TradeFighters already runs dual-auth — external wallets for crypto-native users
and Privy embedded wallets for people who sign up with an email address. Stellar
extends both paths rather than replacing either.

```mermaid
flowchart TB
    START(["User opens the app"]) --> Q{"How do they want<br/>to sign in?"}

    Q -->|"crypto-native"| WK["Stellar Wallets Kit<br/>Freighter · LOBSTR via WalletConnect · xBull · Albedo"]
    Q -->|"email / Google"| PV["Privy embedded wallet<br/>no self-custody setup"]

    WK --> ADDR["Stellar address — G..."]
    PV --> ADDR

    ADDR --> CH["API issues a challenge<br/>bound to the address"]
    CH --> SIG["Wallet signs the challenge"]
    SIG --> VER["API verifies signature"]
    VER --> SESS["Session established"]
    SESS --> KEY["Same signature provisions<br/>the Noether API key"]
    KEY --> READY(["Ready to trade and compete"])
```

The point of the last two steps: **one signing ceremony**. The challenge
signature that establishes the session is also what provisions the Noether API
key through the venue's wallet-challenge flow. The user is not asked to sign
twice for what feels to them like one action.

The API key is scoped to reads and order preparation. It cannot move funds —
every state-changing transaction still requires a fresh wallet signature over
the prepared XDR.

### Sequence

```mermaid
sequenceDiagram
    autonumber
    actor U as User
    participant W as Wallets Kit / Privy
    participant APP as Web app
    participant API as NestJS API
    participant N as Noether

    U->>APP: connect wallet
    APP->>W: request public key
    W-->>APP: G... address
    APP->>API: POST /auth/challenge { address }
    API-->>APP: nonce + expiry
    APP->>W: sign challenge
    W-->>APP: signature
    APP->>API: POST /auth/verify { address, signature }
    API->>API: verify signature, upsert user_exchange_accounts
    API-->>APP: session
    APP->>N: wallet-challenge exchange
    N-->>APP: API key (read + prepare scope)
    APP->>APP: store key in session
    Note over U,N: one signature — session and venue access both live
```

Challenges carry a nonce and a short expiry, and are single-use, so a captured
signature cannot be replayed into a new session.

### Identity storage

`user_exchange_accounts` is the canonical `(user, exchange) → tradingAddress`
table already in production. Stellar rows carry the `G...` address and an
`is_initialized` flag, exactly like the Solana and EVM rows. A user may hold
identities on several venues at once and compete on whichever their competition
allows.

---

## Funding

The biggest funnel drop-off is not signup — it is a user who wants to join a
battle and holds no USDC on Stellar. The answer is to integrate proven
infrastructure rather than write custody-critical bridge code.

```mermaid
flowchart TB
    U(["User wants to join<br/>a competition"]) --> BAL{"USDC on Stellar?"}
    BAL -->|yes| GO(["Enter competition"])
    BAL -->|no| WHAT{"What do they hold?"}

    WHAT -->|"XLM or another<br/>Stellar asset"| SWAP["Swap path<br/>Stellar AMM, e.g. Soroswap"]
    WHAT -->|"USDC on another chain"| BRIDGE["Bridge path<br/>e.g. Allbridge"]
    WHAT -->|"fiat"| FIAT["Anchor Platform<br/>bank transfer on-ramp — later phase"]

    SWAP --> QUOTE["Quote route:<br/>price, fees, slippage, ETA"]
    BRIDGE --> QUOTE
    FIAT --> QUOTE

    QUOTE --> CONF["User confirms"]
    CONF --> EXEC["Execute + track progress"]
    EXEC --> LAND["USDC lands in the<br/>user's Stellar wallet"]
    LAND --> GO
```

All three routes sit behind **one deposit widget**. The user picks an amount
and a source; the widget resolves the route, shows the all-in cost, and tracks
progress to completion. They never choose a bridge provider by name.

### Bridge integration

The monorepo already has `@repo/bridge-core` — the bridge analogue of
`exchange-core`. It defines bridgeable chains, unified quote/transfer/step
types, per-provider capability descriptors, and a `BridgePort` interface that
every provider adapter implements, with signing injected at the API composition
root.

Stellar integration is therefore additive in the same way the trading side is:

| Change | Where |
| --- | --- |
| add `'stellar'` to `BRIDGE_CHAIN_IDS` | `@repo/bridge-core/chain.ts` |
| add a `'stellar'` chain family | `@repo/bridge-core/chain.ts` |
| new provider adapter implementing `BridgePort` | `packages/allbridge-adapter` |
| swap provider behind the same widget | `packages/soroswap-adapter` |

The swap path is not a bridge and does not pretend to be one — it is a separate
provider with its own capability descriptor, surfaced through the same widget
because to the user both answer the same question.

### Progress tracking

Bridges are asynchronous and multi-step, so the widget models them as steps
rather than a spinner:

```mermaid
stateDiagram-v2
    [*] --> Quoted: route resolved
    Quoted --> Signing: user confirms
    Signing --> Submitted: source tx signed
    Submitted --> InFlight: source confirmed
    InFlight --> Landed: destination credited
    InFlight --> Stuck: timeout exceeded
    Stuck --> Landed: recovered
    Stuck --> Support: manual resolution
    Landed --> [*]
```

State is persisted, so closing the tab does not lose the transfer, and a `Stuck`
transfer surfaces with the provider's own recovery path rather than silently
disappearing.

### Rollout

The swap path ships fully functional against testnet liquidity. The bridge path
is implemented and demonstrated against the providers' supported environments
and gated behind a feature flag until mainnet, since bridges have no meaningful
testnet story. Fiat on-ramp via Anchor Platform is scoped after mainnet launch.

---

## Why this matters for the ecosystem

Every funded Stellar account created through this flow is a new Stellar user,
and every bridged transfer is USDC arriving from another ecosystem. The
competition is the reason to come; the deposit widget is what makes coming
possible in one sitting rather than three tabs and a support ticket.

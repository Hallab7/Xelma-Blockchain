# Xelma Contract — Canonical Event Schema

This document is the authoritative reference for all events emitted by the Xelma smart contract.
Indexers, explorers, and client libraries must use these definitions to avoid incompatible
field assumptions.

---

## Versioning strategy

Events are versioned by **schema version tag** in the documentation rather than on-chain.
Breaking field changes will increment the schema version and are announced in `MIGRATION.md`.
Additive changes (new optional events) do not increment the version.

Current schema version: **v1**

---

## Encoding conventions

| Field type   | On-chain encoding                                      |
|--------------|--------------------------------------------------------|
| `u32`        | Raw unsigned 32-bit integer                            |
| `u64`        | Raw unsigned 64-bit integer                            |
| `u128`       | Raw unsigned 128-bit integer                           |
| `i128`       | Raw signed 128-bit integer                             |
| `Address`    | Soroban `Address` (account or contract)                |
| `bool`       | Soroban boolean                                        |

**Amount units**: all token amounts are in stroops (1 vXLM = 10 000 000 stroops, 7 decimal places).

**Price units**: prices are scaled to **4 decimal places** (e.g., 0.2297 XLM → `2297`).

**Ledger sequence**: `u32` counter that increments with each Stellar ledger (~5 s/ledger).

**Timestamp**: Unix epoch seconds (`u64`), sourced from oracle payload or `env.ledger().timestamp()`.

---

## Topic encoding

Each event carries exactly **two topics**, both `Symbol` values.  
They form the canonical `(namespace, action)` pair used for filtering.

---

## Events

### `("round", "created")`

Emitted when a new prediction round is opened.

| Position | Field            | Type   | Description                                                     |
|----------|------------------|--------|-----------------------------------------------------------------|
| 0        | `round_id`       | `u64`  | Monotonically increasing round identifier                       |
| 1        | `start_price`    | `u128` | Starting XLM price (4 decimal places)                          |
| 2        | `start_ledger`   | `u32`  | Ledger sequence number when the round was created               |
| 3        | `bet_end_ledger` | `u32`  | Last ledger at which new bets are accepted                      |
| 4        | `end_ledger`     | `u32`  | Ledger at or after which the round can be resolved              |
| 5        | `mode`           | `u32`  | Round mode: `0` = UpDown, `1` = Precision                      |

---

### `("bet", "placed")`

Emitted when a user places an Up/Down bet.

| Position | Field      | Type      | Description                                      |
|----------|------------|-----------|--------------------------------------------------|
| 0        | `user`     | `Address` | User who placed the bet                          |
| 1        | `round_id` | `u64`     | Round the bet belongs to                         |
| 2        | `amount`   | `i128`    | Bet amount in stroops                            |
| 3        | `side`     | `u32`     | Prediction side: `0` = Up, `1` = Down            |

---

### `("predict", "price")`

Emitted when a user submits a Precision mode price prediction.

| Position | Field             | Type      | Description                                          |
|----------|-------------------|-----------|------------------------------------------------------|
| 0        | `user`            | `Address` | User who submitted the prediction                    |
| 1        | `round_id`        | `u64`     | Round the prediction belongs to                      |
| 2        | `predicted_price` | `u128`    | Predicted price (4 decimal places)                   |
| 3        | `amount`          | `i128`    | Bet amount in stroops                                |

---

### `("round", "resolved")`

Emitted when a round is settled competitively by the oracle.

| Position | Field         | Type   | Description                                      |
|----------|---------------|--------|--------------------------------------------------|
| 0        | `round_id`    | `u64`  | Round that was resolved                          |
| 1        | `final_price` | `u128` | Closing price reported by the oracle (4 dec.)    |
| 2        | `mode`        | `u32`  | Round mode: `0` = UpDown, `1` = Precision        |

---

### `("round", "cancelled")`

Emitted when an admin explicitly cancels an active round. All stakes are refunded.

| Position | Field       | Type   | Description                                             |
|----------|-------------|--------|---------------------------------------------------------|
| 0        | `round_id`  | `u64`  | Round that was cancelled                                |
| 1        | `reason`    | `u32`  | Admin-supplied reason code (application-defined)        |
| 2        | `pool_up`   | `i128` | Total Up-side pool at cancellation time (in stroops)    |
| 3        | `pool_down` | `i128` | Total Down-side pool at cancellation time (in stroops)  |

---

### `("round", "fallback")`

Emitted when a round ends below the configured minimum-participants threshold.
All stakes are refunded; no competitive settlement occurs.

| Position | Field               | Type  | Description                                         |
|----------|---------------------|-------|-----------------------------------------------------|
| 0        | `round_id`          | `u64` | Round that triggered the fallback                   |
| 1        | `participant_count` | `u32` | Actual number of participants at resolution time    |
| 2        | `min_required`      | `u32` | Configured minimum that was not met                 |

---

### `("round", "summary")`

Emitted when a round is resolved, cancelled, or refunded. Contains compact settlement data.

| Position | Field               | Type   | Description                                                           |
|----------|---------------------|--------|-----------------------------------------------------------------------|
| 0        | `round_id`          | `u64`  | Round identifier                                                      |
| 1        | `mode`              | `u32`  | Round mode: `0` = UpDown, `1` = Precision                             |
| 2        | `price_start`       | `u128` | Opening price at round start (4 dec.)                                 |
| 3        | `price_final`       | `u128` | Settlement price (or `0` for administrative cancellation) (4 dec.)    |
| 4        | `participant_count` | `u32`  | Total unique user participants in the round                           |
| 5        | `total_pot`         | `i128` | Total accumulated round pot (in stroops)                              |
| 6        | `status`            | `u32`  | Round status: `0` = Resolved, `1` = Cancelled, `2` = FallbackRefund   |

---


### `("claim", "winnings")`

Emitted when a user successfully claims pending winnings.

| Position | Field    | Type      | Description                             |
|----------|-----------|-----------|-----------------------------------------|
| 0        | `user`   | `Address` | User who claimed                        |
| 1        | `amount` | `i128`    | Amount credited to balance (in stroops) |

---

### `("mint", "initial")`

Emitted when a new user mints their one-time initial vXLM allocation.

| Position | Field    | Type      | Description                             |
|----------|-----------|-----------|-----------------------------------------|
| 0        | `user`   | `Address` | User who received the allocation        |
| 1        | `amount` | `i128`    | Minted amount (always 10 000 000 000 stroops = 1 000 vXLM) |

---

### `("windows", "updated")`

Emitted when the admin reconfigures the bet and run window lengths.

| Position | Field                | Type  | Description                                  |
|----------|----------------------|-------|----------------------------------------------|
| 0        | `bet_window_ledgers` | `u32` | New bet-acceptance window in ledger counts    |
| 1        | `run_window_ledgers` | `u32` | New round-duration window in ledger counts    |

---

### `("oracle", "heartbeat")`

Emitted when the oracle records an on-chain liveness heartbeat.

| Position | Field       | Type  | Description                                                  |
|----------|-------------|-------|--------------------------------------------------------------|
| 0        | `timestamp` | `u64` | Unix epoch seconds when the heartbeat was recorded on-chain  |
| 1        | `status`    | `u32` | Oracle status: `0` = active, `1` = degraded, `2` = offline  |

### `("mode", "transition")`

Emitted when the contract's emergency runtime mode is changed by the admin.

| Position | Field      | Type  | Description                                                         |
|----------|------------|-------|---------------------------------------------------------------------|
| 0        | `old_mode` | `u32` | Previous runtime mode: `0` = Normal, `1` = ClaimsOnly, `2` = Paused |
| 1        | `new_mode` | `u32` | New runtime mode: `0` = Normal, `1` = ClaimsOnly, `2` = Paused      |

---

## Example decode mappings

### JavaScript / TypeScript (Stellar SDK)

```typescript
import { xdr, scValToNative } from "@stellar/stellar-sdk";

function decodeEvent(contractEvent: xdr.DiagnosticEvent) {
  const topics = contractEvent.event().body().v0().topics();
  const ns = scValToNative(topics[0]) as string;    // e.g. "round"
  const action = scValToNative(topics[1]) as string; // e.g. "created"
  const data = scValToNative(contractEvent.event().body().v0().data());
  return { ns, action, data };
}
```

### Rust (soroban-sdk test utilities)

```rust
use soroban_sdk::{symbol_short, testutils::{Events, TryIntoVal}, Env};

let events = env.events().all();
let resolved = events.iter().find(|(_, topics, _)| {
    topics.get(0).and_then(|t| t.try_into_val(&env).ok()) == Some(symbol_short!("round"))
        && topics.get(1).and_then(|t| t.try_into_val(&env).ok()) == Some(symbol_short!("resolved"))
});
```

---

## Field units quick reference

| Concept        | Unit       | Scale factor | Example                              |
|----------------|------------|--------------|--------------------------------------|
| Token amount   | stroops    | × 10 000 000 | 1 vXLM = `10_000_000`               |
| XLM price      | 4 dec.     | × 10 000     | 0.2297 XLM = `2297`                 |
| Duration       | ledgers    | ~5 s/ledger  | 12 ledgers ≈ 60 seconds             |
| Timestamp      | Unix epoch | seconds      | `1_700_000_000` = 2023-11-14 ~22:13 |

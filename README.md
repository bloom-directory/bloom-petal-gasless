# Gasless Relay

Gasless transfers and swaps via Relay's EIP-3009 permit flow. The wallet
signs a gasless authorization instead of an on-chain transaction, so the
origin chain's native gas token is never required.

## VFS Routes

| Route | Method | Purpose |
|---|---|---|
| `/petals/gasless/status.json` | Read | Health check and capability info |
| `/petals/gasless/transactions/<wallet>/<id>.json` | Read | Query transaction state |
| `/petals/gasless/transactions/<wallet>/<id>.json` | Write | Create or advance a transaction |

`<wallet>` is a Bloom wallet alias. `<id>` is a caller-chosen idempotency key
(alphanumeric, `-`, `_`, `.`, max 128 chars).

## Supported Origin Tokens

Only native USDC contracts that implement EIP-3009 work as **origin** currencies.
Other tokens (e.g. USDT) use Permit2 and are rejected. All five USDC variants
below route to each other in every combination (same-chain and cross-chain).

| Chain | Chain ID | USDC Address | Decimals | Permit Domain |
|---|---|---|---|---|
| ethereum | 1 | `0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48` | 6 | `{"name":"USD Coin","version":"2"}` |
| base | 8453 | `0x833589fcd6edb6e08f4c7c32d4f71b54bda02913` | 6 | `{"name":"USD Coin","version":"2"}` |
| optimism | 10 | `0x0b2c639c533813f4aa9d7837caf62653d097ff85` | 6 | `{"name":"USD Coin","version":"2"}` |
| polygon | 137 | `0x3c499c542cef5e3811e1192ce70d8cc03d5c3359` | 6 | `{"name":"USD Coin","version":"2"}` |
| avalanche | 43114 | `0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e` | 6 | `{"name":"USD Coin","version":"2"}` |

The **destination** currency may be any Relay-supported identifier, including
non-EVM chains like Hyperliquid (chain ID 1337). The origin must always be an
EIP-3009 EVM token from the table above.

## Creating a Transaction

```sh
bloom vfs write \
  /petals/gasless/transactions/<wallet>/<id>.json \
  --data '{
    "origin": {
      "chain": "base",
      "chain_id": 8453,
      "currency": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
      "decimals": 6,
      "permit_domain": {"name": "USD Coin", "version": "2"}
    },
    "destination": {
      "chain": "optimism",
      "chain_id": 10,
      "currency": "0x0b2c639c533813f4aa9d7837caf62653d097ff85",
      "decimals": 6
    },
    "amount": "100",
    "minimum_output": "97"
  }'
```

### Fields

- **`origin`** — where funds come from. Must be an EIP-3009 token (see table).
  - `chain` / `chain_id` — Relay chain slug and numeric ID.
  - `currency` — EVM token contract address (lowercase).
  - `decimals` — token precision (USDC = 6).
  - `permit_domain` — EIP-712 domain from the token contract. For all USDC
    variants above: `{"name": "USD Coin", "version": "2"}`.

- **`destination`** — where funds go.
  - `chain` / `chain_id` — Relay chain slug and numeric ID.
  - `currency` — Relay currency identifier. For EVM chains: token address.
    For Hyperliquid: Relay's per-chain currency string.
  - `decimals` — token precision.
  - `recipient` — optional. Defaults to the Bloom wallet address.

- **`amount`** — human-readable input amount using `origin.decimals`
  (e.g. `"100"` = 100 USDC).

- **`minimum_output`** — mandatory floor on the destination amount, using
  `destination.decimals`. The petal rejects any Relay quote whose guaranteed
  minimum falls below this value. See *Choosing `minimum_output`* below.

### Choosing `minimum_output`

This is the caller's slippage tolerance. Relay quotes two values: an expected
output and a guaranteed minimum (the floor Relay will actually deliver). The
petal rejects quotes where Relay's floor is below your `minimum_output`.

A reasonable default: **95–99% of the expected output**. For a 100 USDC
transfer where Relay quotes ~99.5 expected and ~97.5 floor, a `minimum_output`
of `"97"` accepts the quote while limiting slippage to 3%.

- Too high (e.g. `"99"` when Relay's floor is 97.5) → write fails with
  `Relay minimum output 97.5 is below required minimum_output 99`.
- Too low (e.g. `"1"`) → quote always passes but the user may lose value
  to slippage.
- To discover the current floor before committing: read the transaction
  after a failed write — it won't exist. Alternatively, make a quote-only
  test write and read the `quote.relay_minimum_out_units` field.

### Same-Chain Swaps

Set origin and destination to the same chain. The petal forces Relay's solver
execution path so the result is always a permit flow, not an on-chain
transaction.

### Hyperliquid Deposits

```json
{
  "origin": {
    "chain": "ethereum", "chain_id": 1,
    "currency": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
    "decimals": 6,
    "permit_domain": {"name": "USD Coin", "version": "2"}
  },
  "destination": {
    "chain": "hyperliquid", "chain_id": 1337,
    "currency": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
    "decimals": 6
  },
  "amount": "50",
  "minimum_output": "49"
}
```

The destination `currency` for Hyperliquid is the Base USDC contract address
(Relay maps it internally). Hyperliquid-specific UX and presets belong in the
Hyperliquid petal; gasless handles only the generic permit flow.

## Transaction Lifecycle

Read the transaction to check its status and next action:

```sh
bloom vfs cat /petals/gasless/transactions/<wallet>/<id>.json
```

### Statuses

| Status | Meaning | Next Action |
|---|---|---|
| `not_created` | No transaction at this ID. Response includes a write template. | Write to create. |
| `awaiting_signature` | Quote accepted; next write will attempt signing. | Retry write to sign. |
| `approval_required` | Bloom needs the wallet owner's approval. | Open `approval.ceremony_url`, approve, then retry the exact write body from `next.retry_write_body`. |
| `approval_expired` | The approval ceremony timed out. | Retry the write to get a fresh ceremony (same Relay request). |
| `quote_expired` | The Relay permit's `validBefore` has passed. | Use a **new** transaction ID; the petal never silently re-quotes. |
| `submitting` | Signing succeeded; permit is being submitted to Relay. | Poll by reading again. |
| `submission_unknown` | Permit submission hit a transport error. Outcome is ambiguous. | Poll by reading again; Relay may have accepted it. |
| `submitted` | Relay accepted the permit submission. | Poll by reading again until Relay reports a terminal status. |
| `waiting` / `depositing` / `pending` / `delayed` | Relay is processing. | Poll by reading again. |
| `success` | Relay reports successful destination settlement. Terminal. | Done. |
| `refund` | Relay could not complete and issued a refund. Terminal. | Inspect `relay.tx_hashes` for the refund transaction. |
| `failure` | Relay failed to complete. Terminal. | Inspect `relay.tx_hashes` for details. |
| `unavailable` | Relay returned a status the petal cannot interpret. | Poll again; if persistent, contact Relay. |

### Key Rules

- **Idempotency**: a transaction ID is permanently bound to its initial
  wallet, route, amount, and minimum output. A retry with different parameters
  is rejected.
- **Approval**: when `approval_required`, the write is denied but state is
  persisted. Complete the ceremony URL, then retry the **exact same write**.
  The petal retains the same Relay request ID and signing hash across retries.
- **Permit expiry**: if the Relay permit expires before signing, the
  transaction is permanently `quote_expired`. Create a new one with a new ID.
- **Signatures**: never stored or returned. Transport errors during permit
  submission are opaque because the signature is in the request URL.
- **Settlement**: a successful write only means Relay **accepted** the permit.
  Only a later read showing `status: "success"` means funds settled.

## Scope

- ✅ Caller-selected Relay origin and destination
- ✅ Exact-input EIP-3009 transactions (`ReceiveWithAuthorization`,
  `TransferWithAuthorization`)
- ✅ Same-chain and cross-chain solver execution
- ✅ Caller-selected destination recipient
- ✅ Caller-enforced minimum output
- ✅ Durable approval, submission, and reconciliation state

- ❌ Permit2 signatures (e.g. USDT)
- ❌ Destination contract calls (transfer-only)
- ❌ Application fees
- ❌ Relay's enterprise `/execute` sponsorship API
- ❌ Arbitrary sponsored contract calls

## Development

Live validation must stop after requesting Relay quotes. Do not complete an
approval ceremony or submit a permit unless moving funds is separately and
explicitly authorized.

```sh
cargo fmt --manifest-path route/Cargo.toml --check
cargo test --manifest-path route/Cargo.toml --locked
cargo clippy --manifest-path route/Cargo.toml --locked --all-targets -- -D warnings
bash scripts/check-route-architecture.sh
petal build --root .
petal check --root .
bloom petals build .
```

# gasless operating contract

## Routes

| Route | Read | Write |
|---|---|---|
| `/petals/gasless/status.json` | Health + capability info (no side effects) | — |
| `/petals/gasless/transactions/<wallet>/<id>.json` | Transaction state + `next` action | Create or advance transaction |

`<wallet>` is a Bloom wallet alias — resolve its EVM address through the VFS for
Relay, but retain the alias for Bloom signing. `<id>` is a caller-defined,
durable idempotency key (alphanumeric, `-`, `_`, `.`, max 128 chars).

## Supported Origin Tokens

Only these five native USDC contracts implement EIP-3009 and work as origin:

- **ethereum** (1): `0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48`
- **base** (8453): `0x833589fcd6edb6e08f4c7c32d4f71b54bda02913`
- **optimism** (10): `0x0b2c639c533813f4aa9d7837caf62653d097ff85`
- **polygon** (137): `0x3c499c542cef5e3811e1192ce70d8cc03d5c3359`
- **avalanche** (43114): `0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e`

All use decimals 6 and permit domain `{"name": "USD Coin", "version": "2"}`.
All 25 pairs (5 same-chain + 20 cross-chain) are confirmed working. Other
tokens (USDT, bridged USDC) use Permit2 and are rejected. The destination may
be any Relay-supported chain/currency, including Hyperliquid (chain_id 1337).

## Write Body

```json
{
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
    "decimals": 6,
    "recipient": "0x... (optional; defaults to wallet address)"
  },
  "amount": "100",
  "minimum_output": "97"
}
```

- `amount` uses `origin.decimals`. `minimum_output` uses `destination.decimals`.
- `destination.recipient` may be omitted to send to the wallet address.
- `minimum_output` is mandatory. Use 95–99% of the expected output as a
  guideline. The petal rejects quotes where Relay's guaranteed floor is below
  this value.

## State Machine

```
                        ┌──────────────┐
                   write│ (first time) │
                        ▼              │
                 ┌─────────────────┐   │
                 │ not_created     │   │
                 │ (write template)│   │
                 └────────┬────────┘   │
                    write │            │
                          ▼            │
              ┌──────────────────────┐│
              │ awaiting_signature   ││
              │ (quote fetched,      ││
              │  EIP-3009 validated) ││
              └──┬────────────┬──────┘│
       write     │            │ time  │
   (signs)      │            │       │
        ┌───────┘            ▼       │
        │              ┌──────────────┐
        │              │ quote_expired│
        │              │ (terminal)   │
        │              └──────────────┘
        │
        │  write (approval needed)
        ▼
 ┌────────────────┐
 │approval_required│
 └──┬──────────┬──┘
    │write     │time
    │(after    │
    │ceremony) │
    │          ▼
    │   ┌─────────────────┐
    │   │ approval_expired│
    │   │ (retry write →  │
    │   │  fresh ceremony)│
    │   └─────────────────┘
    │
    ▼ (sign succeeds)
 ┌──────────────┐     transport error
 │ submitting   │───────────────────┐
 └──────┬───────┘                   │
        │ accepted                  ▼
        ▼                  ┌─────────────────┐
 ┌──────────────┐         │submission_unknown│
 │  submitted   │         │ (poll to reconcile│
 └──────┬───────┘         │  with Relay)      │
        │                  └─────────────────┘
        │ poll Relay
        ▼
 ┌─────────────────────────────────┐
 │ waiting / depositing / pending  │
 │ / delayed                       │
 └──────────────┬──────────────────┘
        │ terminal
    ┌───┼───────────┐
    ▼   ▼           ▼
 success   refund   failure
(✅ done) (terminal) (terminal)
```

### Reading Status

Every read returns a JSON object with:
- `status` — one of the states above
- `next` — object with `action` and `instruction` (and `retry_write_body` when
  applicable)
- `request` — the original bound request
- `quote` — Relay's quote details (amounts, fees, timing, permit expiry)
- `approval` — ceremony URL and expiry (only when `approval_required`)
- `submission` — `"accepted"` or `"unknown"` (after signing)
- `relay` — projected Relay status with tx hashes (after submission)

### Next Actions

| `next.action` | When | What to do |
|---|---|---|
| `review_route_then_approve` | `approval_required` | Review origin/destination/amount/output. Open `approval.ceremony_url`. After approval, retry the exact `next.retry_write_body`. |
| `retry_write` | `approval_expired` or unknown status | Retry the write with `next.retry_write_body` to get a fresh ceremony or re-attempt signing. |
| `create_new_transaction` | `quote_expired` | This ID is dead. Create a new transaction with a new ID. |
| `poll` | `submitting` through `delayed` | Read again later. Only Relay `success` means done. |
| `complete` | `success` | Settlement confirmed. |
| `inspect` | `refund` or `failure` | Check `relay.tx_hashes`. Terminal — funds were returned or lost. |

## Safety Validation (Before Signing)

The petal enforces all of these before requesting a wallet signature. An agent
reviewing the transaction for a human should verify the same things:

1. **Exactly one step** with `id: "authorize1"`, `kind: "signature"`
2. **EIP-3009 type**: `ReceiveWithAuthorization` or `TransferWithAuthorization`
3. **Permit receiver** is Relay's pinned ApprovalProxy
   (`0xccc88a9d1b4ed6b0eaba998850414b24f1c315be`)
4. **Domain** matches caller's `permit_domain` (name, version, chainId,
   verifyingContract = origin currency)
5. **Value**: `from` = wallet, `to` = permit receiver, `value` = exact input
6. **Relay order**: one input payment, two refund branches (origin + destination
   → wallet), one output payment, no destination calls, no fees
7. **minimum_output**: Relay's guaranteed floor ≥ caller's requirement

## Error Recovery

| Error message | Cause | Resolution |
|---|---|---|
| `Relay minimum output X is below required minimum_output Y` | Slippage too high for the caller's floor | Lower `minimum_output` or increase `amount`. |
| `Relay did not offer exactly one EIP-3009 gasless authorization` | Token doesn't support EIP-3009, or pair unsupported | Check origin token is native USDC from the table. |
| `Relay returned an unsupported EIP-3009 type` | Token uses Permit2 (e.g. USDT) | Use a native USDC contract instead. |
| `transaction ID already belongs to a different wallet, route, amount, or output constraint` | Tried to change parameters on existing ID | Use a new transaction ID. |
| `Relay quote permit has expired` | Too much time passed since quoting | Use a new transaction ID. |
| `permission denied` (after `approval_required`) | Wallet requires ceremony, not yet approved | Open ceremony URL, approve, retry write. |

## Hyperliquid

Use the canonical generic route with `destination.chain_id: 1337`. The
destination `currency` is a Relay-safe string (e.g. the Base USDC address).
Hyperliquid-specific presets and deposit UX belong in the Hyperliquid petal
and should compose this canonical route.

## Constraints

Never expose or persist a wallet signature. Treat permit-submission transport
errors as opaque — Relay requires the signature in the request URL, so error
bodies may contain replayable signatures.

Durable initialization is atomic and scoped to `<wallet>/<id>`. Approval
retries retain the original Relay request ID, signing hash, and exact route.
Attempted submissions remain readable and reconcile through Relay even after
local or permit expiry. A write only means Relay accepted the permit
submission; only a later Relay `success` status means settlement.

Do not execute live fund-moving tests without separate explicit authorization.
Live quote-only validation is allowed.

## Development Skill

Petals are authored against the `bloom-petal-development` skill:
https://github.com/bloom-directory/petal/tree/main/skills/bloom-petal-development
Load it into your agent before extending this Petal.

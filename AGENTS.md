# gasless operating contract

## Routes

| Route | Read | Write |
|---|---|---|
| `/petals/gasless/status.json` | Health + capability info (no side effects) | — |
| `/petals/gasless/transactions/<wallet>/<index>/<id>.json` | Transaction state + `next` action | Create or advance transaction |

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
- `approval` — action ID and expiry when approval is required, or recovery details when signing needs reconciliation
- `submission` — `"accepted"` or `"unknown"` (after signing)
- `relay` — projected Relay status with tx hashes (after submission)

### Next Actions

| `next.action` | When | What to do |
|---|---|---|
| `review_route_then_approve` | `approval_required` | Review origin/destination/amount/output. Approve `approval.action_id` in Bloom. After approval, retry the exact `next.retry_write_body`. |
| `retry_write` | Unfinished non-terminal signing | Retry the exact write to resume the same signing operation. |
| `create_new_transaction` | `quote_expired`, `approval_expired` or `approval_cancelled` | The operation is terminal. Create a fresh transaction with a new ID. |
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
| `permission denied` (after `approval_required`) | Wallet requires ceremony, not yet approved | Approve the action in Bloom, then retry the exact write. An expired or cancelled operation requires a new transaction ID. |

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

## Prepared signing and numbered accounts

`<index>` selects the numbered Bloom account, including account 0. The route
reads `wallets/<wallet>/<index>/address.evm`; each account has separate private
transaction state. Signing uses the canonical prepared-action API with complete
EIP-712 input, an exact token debit and a durable operation key bound to the
transaction and Relay request. Retrying after approval resumes that operation.
Recovery outcomes stay visible in transaction status. Reads never expose the
signature. Ambiguous submissions must be reconciled by reading status; another
write does not sign or submit again.

Covered Trader/Degen authorization requires explicit owner opt-in for the
exact installed package. The package declares its exact origin-token debit;
Broker trusts this installer-signed declaration and applies the generic account,
network, token-policy and budget checks. The Petal validates the EIP-3009
payload and Relay route. Generic review binds the displayed typed data to the
signing bytes; it does not prove destination settlement or economic value.
Institutional requires individual approval. Status is read from Bloom's
prepared-signing status API, never a cached private approval record.

Before upgrading from the unnumbered route, reconcile pending operations with
the installed build and retain its package/private records. This build does not
import unnumbered state or reinterpret old approvals, even for account 0.

Expired or cancelled prepared signing operations retain their original identity.
Status reports `approval_expired` or `approval_cancelled` and requests a new
transaction ID; repeating the same write does not create a fresh ceremony.
Submitted or ambiguous transactions keep their Relay reconciliation path even
after local expiry. Never replace one merely because its authorization expired.

# gasless operating contract

`/petals/gasless/status.json` is a read-only health and capability route with
no side effects.

The canonical operation is:

`/petals/gasless/transactions/<wallet>/<id>.json`

`<wallet>` is a Bloom wallet alias. Resolve its EVM address through the VFS for
Relay, but retain the alias for Bloom signing. `<id>` is a caller-defined,
durable idempotency key. Its write body is:

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
    "recipient": "0x..."
  },
  "amount": "100",
  "minimum_output": "99"
}
```

`destination.recipient` may be omitted only to use the resolved wallet
address. The refund address is always the resolved wallet address.

The canonical route is a generic Relay EIP-3009 permit flow for same-chain and
cross-chain transfers/swaps. It accepts caller-selected origin and destination
chains and currencies, but the origin must be an EVM token for which Relay
returns EIP-3009 typed data. Permit2, arbitrary destination calls, Relay's
enterprise `/execute` sponsorship API, application fees, and credentials are
out of scope.

Before signing, validate the complete caller request against Relay's response:
chain slugs and IDs, currency identifiers and decimals, recipient, refund
address, exact input amount, caller minimum output, EIP-712 domain, EIP-3009
primary type and fields, request ID, status endpoint, permit API, and the
pinned Relay permit receiver. The Relay order must contain one exact origin
payment, exactly the expected origin and destination refund branches, one
exact destination payment, no destination calls, and no application fees.

Durable initialization is atomic and scoped to `<wallet>/<id>`. Approval
retries must retain the original Relay request ID, signing hash, and exact
route. Attempted submissions must remain readable and reconcile through Relay
even after local or permit expiry. A write only means Relay accepted the permit
submission; only a later Relay `success` status means settlement.

Never expose or persist a wallet signature. Treat permit-submission transport
errors as opaque because Relay requires the signature in the request URL.

Hyperliquid deposits flow through the canonical generic route:
destination.chain_id 1337 and non-EVM destination currency identifiers are
accepted as safe Relay strings. Hyperliquid-specific presets and deposit UX
belong in the Hyperliquid petal and should compose the canonical generic route.

Do not execute live fund-moving tests without separate explicit authorization.
Live quote-only validation is allowed.

## Development skill

Petals are authored against the `bloom-petal-development` skill:
https://github.com/bloom-directory/petal/tree/main/skills/bloom-petal-development
Load it into your agent before extending this Petal.

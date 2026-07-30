# gasless operating contract

`/petals/gasless/status.json` is a read-only health route with no side effects.

Canonical multichain deposits exist at
`/petals/gasless/chains/<source>/deposits/<wallet>/<id>.json`, where `<source>`
is exactly one of `ethereum`, `base`, `arbitrum`, `optimism`, `polygon`, or
`avalanche`. The compatibility route
`/petals/gasless/deposits/<wallet>/<id>.json` remains Ethereum-only so existing
deposits can still be read, retried, and reconciled.

Every source is pinned to its native six-decimal USDC contract:

- `ethereum` (`1`): `0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48`
- `base` (`8453`): `0x833589fcd6edb6e08f4c7c32d4f71b54bda02913`
- `arbitrum` (`42161`): `0xaf88d065e77c8cc2239327c5edb3a432268e5831`
- `optimism` (`10`): `0x0b2c639c533813f4aa9d7837caf62653d097ff85`
- `polygon` (`137`): `0x3c499c542cef5e3811e1192ce70d8cc03d5c3359`
- `avalanche` (`43114`): `0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e`

All deposits pin HyperCore perps USDC as output and the resolved wallet address
as recipient and refund address. The write body contains `amount` and a
mandatory caller-defined `minimum_output`.

`<wallet>` is a Bloom wallet alias: resolve its address through the VFS for
Relay, but retain the alias for Bloom signing. Each route must use Relay's
EIP-3009 permit flow and validate the selected chain, native USDC contract,
EIP-712 domain, exact input amount, output, recipient, request ID, and pinned
Relay permit receiver before signing. The returned Relay refund plan must
contain exactly the pinned source-USDC and HyperCore-USDC branches, both
returning to the resolved wallet address.

Durable state and atomic initialization must be source-chain scoped. The
Ethereum compatibility and canonical routes must share one authoritative
Ethereum operation so the same wallet/id cannot create two Relay requests.
Approval retries must retain the original Relay request ID and signing hash.
Attempted submissions must remain readable and reconcile through Relay even
after local or permit expiry.

Never expose or persist a wallet signature. Treat transport errors from permit
submission as opaque because Relay requires the signature in the request URL.
Do not execute live fund-moving tests without separate explicit authorization;
live quote-only validation is allowed.

## Development skill

Petals are authored against the `bloom-petal-development` skill:
https://github.com/bloom-directory/petal/tree/main/skills/bloom-petal-development
Load it into your agent before extending this Petal.

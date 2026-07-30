# Gasless

This standalone Bloom Petal moves native USDC from Ethereum, Base, Arbitrum,
Optimism, Polygon, or Avalanche to Hyperliquid HyperCore without requiring the
source chain's gas token. It requests a Relay gasless quote, verifies the exact
EIP-3009 authorization and destination, asks Bloom for owner approval, submits
the permit, and exposes durable status at the same path.

Supported source slugs and pinned native-USDC contracts:

| Source | Chain ID | Native USDC |
| --- | ---: | --- |
| `ethereum` | 1 | `0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48` |
| `base` | 8453 | `0x833589fcd6edb6e08f4c7c32d4f71b54bda02913` |
| `arbitrum` | 42161 | `0xaf88d065e77c8cc2239327c5edb3a432268e5831` |
| `optimism` | 10 | `0x0b2c639c533813f4aa9d7837caf62653d097ff85` |
| `polygon` | 137 | `0x3c499c542cef5e3811e1192ce70d8cc03d5c3359` |
| `avalanche` | 43114 | `0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e` |

```sh
bloom vfs write \
  /petals/gasless/chains/<source>/deposits/<wallet-alias>/<id>.json \
  --data '{"amount":"100","minimum_output":"99"}'
```

The older `/petals/gasless/deposits/<wallet-alias>/<id>.json` route is retained
as an Ethereum-only compatibility alias. It shares the canonical Ethereum
operation and state; it cannot create a second request for the same wallet/id.

`minimum_output` is the caller's minimum acceptable HyperCore perps USDC,
expressed with at most 8 decimal places. Before requesting a wallet signature,
the Petal rejects any Relay quote whose slippage-adjusted minimum output is
below this value.

If Bloom requests approval, first read the operation and review `quote`.
Then open `approval.ceremony_url`, complete the ceremony, and repeat the exact
write using `next.retry_write_body` with the same wallet and id:

```sh
bloom vfs cat \
  /petals/gasless/chains/<source>/deposits/<wallet-alias>/<id>.json
```

A ceremony URL is single-use and may expire. Retrying the exact write safely
returns a fresh actionable ceremony when needed while retaining the same Relay
request and permit signing hash. If the Relay permit itself expires, use a new
deposit id; the Petal never silently replaces an accepted quote.

A successful write means Relay accepted the permit submission. Only Relay
status `success` on a later read means the transfer completed. The Petal never
stores or returns the wallet signature.

Each source has an independent durable idempotency namespace. Reusing an id on
another source chain creates a different operation, while concurrent first
writes to the same source/wallet/id atomically converge on one persisted Relay
request. Unknown source slugs, bridged USDC variants, changed chain IDs,
unexpected permit receivers, token substitutions, and quotes below
`minimum_output` are rejected before signing. Relay's returned refund plan
must contain exactly two branches—native source USDC and HyperCore USDC—and
both must return to the same resolved wallet.

Live validation should stop after requesting Relay quotes. Do not complete an
approval ceremony or submit a permit unless moving funds is separately and
explicitly authorized.

Build with the nearby Petal CLI and validate with Bloom:

```sh
petal build --root .
petal check --root .
bloom petals build .
```

Before pushing, enforce the route architecture rules from the
`bloom-petal-development` skill:

```sh
bash scripts/check-route-architecture.sh
```

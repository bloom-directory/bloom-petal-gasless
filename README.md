# Gasless Relay

This Bloom Petal executes caller-bounded Relay transfers and swaps without
requiring the wallet to hold the origin chain's gas token. It requests a Relay
quote with `usePermit`, verifies the complete route and EIP-3009 authorization,
asks Bloom for owner approval, submits the permit, and exposes durable status
at the same virtual file.

The canonical route is:

```text
/petals/gasless/transactions/<wallet-alias>/<id>.json
```

For example, a gasless Base-USDC to Optimism-USDC transfer can be created with:

```sh
bloom vfs write \
  /petals/gasless/transactions/<wallet-alias>/<id>.json \
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
    "minimum_output": "99"
  }'
```

`chain` is Relay's canonical chain slug and `chain_id` is its numeric Relay
chain ID. Origin currencies must be EVM tokens for which Relay returns an
EIP-3009 authorization; native USDC is the principal fully gasless case.
Destination currency and recipient identifiers may use another Relay-supported
chain's format. Omitting `destination.recipient` sends to the resolved Bloom
wallet address. Refunds always return to that wallet.

`amount` uses `origin.decimals`. `minimum_output` uses
`destination.decimals` and is mandatory. Before requesting a wallet signature,
the Petal rejects a quote when Relay's output floor is below this caller
constraint.

If Bloom requests approval, inspect the same operation:

```sh
bloom vfs cat \
  /petals/gasless/transactions/<wallet-alias>/<id>.json
```

Review the exact origin, destination, recipient, input, output floor, and quote.
Open `approval.ceremony_url`, complete the ceremony, and repeat the exact body
shown in `next.retry_write_body`.

A ceremony URL is single-use and may expire. Retrying safely produces another
ceremony while retaining the same Relay request and signing hash. If the Relay
permit expires, use a new transaction ID; the Petal never silently re-quotes an
approved operation.

A successful write means Relay accepted permit submission. Only Relay status
`success` on a later read means settlement completed. Signatures are never
stored or returned, and ambiguous submission failures remain readable for
status reconciliation.

## Scope

This Petal supports Relay's public permit-based gasless transfer/swap path:

- caller-selected Relay origin and destination;
- exact-input EIP-3009 transactions;
- same-chain and cross-chain solver execution;
- caller-selected destination recipient;
- caller-enforced minimum output;
- durable approval, submission, and reconciliation state.

It deliberately rejects Permit2, destination contract calls, application fees,
and Relay's enterprise `/execute` endpoint. Arbitrary sponsored contract calls
require an enterprise API key, funded sponsorship policy, and a smart-account
or other explicit execution model; those are a separate trust boundary.

## Hyperliquid compatibility

HyperCore deposits are a Hyperliquid product workflow, not the generic Relay
contract. New Hyperliquid deposit routes and presets should live in the
Hyperliquid petal and invoke this canonical transaction shape with HyperCore's
pinned destination.

The former routes remain available solely so existing durable operations and
clients keep working:

```text
/petals/gasless/deposits/<wallet-alias>/<id>.json
/petals/gasless/chains/<source>/deposits/<wallet-alias>/<id>.json
```

Those compatibility routes retain their original native-USDC-to-HyperCore
validation and `gasless.deposit` signing intent.

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

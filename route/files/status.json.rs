petal::route_file!(
    spec: petal::static_read_spec(),
    read: |_ctx: &petal::Ctx| petal::read_json_value(&serde_json::json!({
        "petal": "gasless",
        "status": "ok",
        "description": "Gasless EIP-3009 permit transfers and swaps via Relay. The wallet signs a permit instead of an on-chain transaction, so no origin gas token is required.",
        "canonical_route": "transactions/<wallet>/<id>.json",
        "operations": ["same-chain-swap", "cross-chain-swap", "transfer"],
        "provider": "relay",
        "signing_kind": "eip3009-permit",
        "arbitrary_sponsored_calls": false,
        "origin_tokens": [
            {"chain": "ethereum",  "chain_id": 1,     "currency": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", "decimals": 6, "symbol": "USDC", "permit_domain": {"name": "USD Coin", "version": "2"}},
            {"chain": "base",      "chain_id": 8453,  "currency": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", "decimals": 6, "symbol": "USDC", "permit_domain": {"name": "USD Coin", "version": "2"}},
            {"chain": "optimism",  "chain_id": 10,    "currency": "0x0b2c639c533813f4aa9d7837caf62653d097ff85", "decimals": 6, "symbol": "USDC", "permit_domain": {"name": "USD Coin", "version": "2"}},
            {"chain": "polygon",   "chain_id": 137,   "currency": "0x3c499c542cef5e3811e1192ce70d8cc03d5c3359", "decimals": 6, "symbol": "USDC", "permit_domain": {"name": "USD Coin", "version": "2"}},
            {"chain": "avalanche", "chain_id": 43114, "currency": "0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e", "decimals": 6, "symbol": "USDC", "permit_domain": {"name": "USD Coin", "version": "2"}}
        ],
        "destination_note": "Destination may be any Relay-supported chain/currency, including non-EVM chains like Hyperliquid (chain_id 1337). See README.md for the full write body format.",
        "docs": ["README.md", "AGENTS.md"]
    }))
);

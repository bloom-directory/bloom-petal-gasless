petal::route_file!(
    spec: petal::static_read_spec(),
    read: |_ctx: &petal::Ctx| petal::read_json_value(&serde_json::json!({
        "petal": "gasless",
        "status": "ok",
        "canonical_route": "transactions/<wallet>/<id>.json",
        "execution": {
            "provider": "relay",
            "kind": "eip3009-permit",
            "operations": ["same-chain-swap", "cross-chain-swap", "transfer"],
            "arbitrary_sponsored_calls": false
        },
        "legacy_hypercore_sources": crate::SOURCE_CHAINS.iter().map(|chain| serde_json::json!({
            "slug": chain.slug,
            "chain_id": chain.chain_id,
            "token": "USDC",
            "token_address": chain.usdc
        })).collect::<Vec<_>>(),
        "legacy_routes": [
            "deposits/<wallet>/<id>.json",
            "chains/<source>/deposits/<wallet>/<id>.json"
        ]
    }))
);

petal::route_file!(
    spec: petal::static_read_spec(),
    read: |_ctx: &petal::Ctx| petal::read_json_value(&serde_json::json!({
        "petal": "gasless",
        "status": "ok",
        "sources": crate::SOURCE_CHAINS.iter().map(|chain| serde_json::json!({
            "slug": chain.slug,
            "chain_id": chain.chain_id,
            "token": "USDC",
            "token_address": chain.usdc
        })).collect::<Vec<_>>(),
        "destination": {"chain_id": 1337, "token": "USDC"}
    }))
);

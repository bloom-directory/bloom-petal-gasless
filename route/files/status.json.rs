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
        }
    }))
);

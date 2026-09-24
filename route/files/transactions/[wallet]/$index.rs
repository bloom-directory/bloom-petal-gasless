petal::route_file!(
    spec: petal::static_dir_spec(),
    ctx_list: |ctx: &petal::Ctx| {
        let wallet = petal::wallet_param(ctx)?;
        if !petal::is_safe_segment(wallet) || wallet.len() > 128 {
            return Err(petal::error(-3, "wallet alias is unsafe"));
        }
        Ok(Vec::new())
    }
);

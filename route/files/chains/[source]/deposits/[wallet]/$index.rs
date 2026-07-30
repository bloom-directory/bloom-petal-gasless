petal::route_file!(
    spec: petal::static_dir_spec(),
    ctx_list: |ctx: &petal::Ctx| {
        let source = petal::param(ctx, "source")?;
        crate::source_chain(source)?;
        let wallet = petal::param(ctx, "wallet")?;
        if !petal::is_safe_segment(wallet) || wallet.len() > 128 {
            return Err(petal::error(-3, "wallet alias is unsafe"));
        }
        Ok(Vec::new())
    }
);

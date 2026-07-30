petal::route_file!(
    spec: petal::static_dir_spec(),
    ctx_list: |ctx: &petal::Ctx| {
        let source = petal::param(ctx, "source")?;
        crate::source_chain(source)?;
        Ok(vec![petal::dir("deposits")])
    }
);

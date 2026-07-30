petal::route_file!(
    spec: petal::static_dir_spec(),
    list: crate::SOURCE_CHAINS
        .iter()
        .map(|chain| petal::dir(chain.slug))
        .collect::<Vec<_>>()
);

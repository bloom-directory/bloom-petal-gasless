petal::route_file!(
    spec: petal::static_dir_spec(),
    list: vec![
        petal::dir("transactions"),
        petal::file("status.json"),
        petal::file("README.md"),
        petal::file("AGENTS.md"),
    ]
);

petal::route_file!(
    spec: petal::static_dir_spec(),
    list: vec![
        petal::dir("chains"),
        petal::dir("deposits"),
        petal::file("status.json"),
    ]
);

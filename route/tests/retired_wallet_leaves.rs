//! Bloom v0.3 removed the wallet-root leaves (`address`, `addresses.json`,
//! `public_key`, `policy.toml`) in favour of the account-scoped projection
//! under `wallets/<wallet>/<index>/`. Every route source, `src/lib.rs`
//! included, must read the account leaf and never one of the retired paths.
//!
//! This lives outside `src/` so the retired spellings it searches for are not
//! themselves part of the scanned tree.

fn visit(dir: &std::path::Path, hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            visit(&path, hits);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let source = std::fs::read_to_string(&path).expect("read source");
            for retired in [
                "wallets/{wallet}/address\"",
                "/addresses.json",
                "/public_key\"",
                "/policy.toml",
            ] {
                if source.contains(retired) {
                    hits.push(format!("{} reads {retired}", path.display()));
                }
            }
        }
    }
}

#[test]
fn no_route_source_reads_the_retired_wallet_root_leaves() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    visit(&root.join("src"), &mut hits);
    visit(&root.join("files"), &mut hits);
    assert!(hits.is_empty(), "{hits:?}");
}

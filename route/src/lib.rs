//! Gasless Relay permit protocol and generic transaction state.

mod common;
mod relay;

pub use relay::{
    PermitDomain, RelayDestination, RelayOrigin, RelayTransactionRequest, gasless_transaction,
    gasless_transaction_status,
};
pub use serde_json;

/// Bloom account whose EVM key this Petal acts for. The Petal is mounted at
/// `petals/gasless/`, not under an account, so it uses the wallet's root
/// account.
const WALLET_ACCOUNT: u32 = 0;

/// The wallet's EVM address, read from Bloom's account-scoped projection.
/// Bloom v0.3 removed the wallet-root `address` leaf; the account leaf is the
/// only source.
pub fn wallet_address(wallet: &str) -> Result<String, petal::DispatchResponse> {
    petal::validate_wallet_id(wallet).map_err(|message| petal::error(-3, message))?;
    let path = wallet_address_path(wallet);
    let bytes = petal::sdk::vfs_read(&path, 128).map_err(|error| match error {
        petal::sdk::SdkError::Host(petal::sdk::HostStatus::Denied) => {
            petal::error(-2, format!("wallet {wallet}: read of {path} was denied"))
        }
        // Bloom answers a missing account leaf with `NotAFile`, which reaches
        // the Petal as `Invalid`.
        petal::sdk::SdkError::Host(
            petal::sdk::HostStatus::NotFound | petal::sdk::HostStatus::Invalid,
        ) => petal::error(
            -1,
            format!(
                "wallet {wallet} has no EVM address at {path}; the account may be missing or have no EVM key"
            ),
        ),
        other => petal::error(-4, format!("wallet {wallet}: read {path}: {}", other.message())),
    })?;
    parse_wallet_address(&path, &bytes)
        .map_err(|error| petal::error(-4, format!("wallet {wallet}: {error}")))
}

fn wallet_address_path(wallet: &str) -> String {
    format!("wallets/{wallet}/{WALLET_ACCOUNT}/address.evm")
}

fn parse_wallet_address(path: &str, bytes: &[u8]) -> Result<String, String> {
    let address = core::str::from_utf8(bytes)
        .map_err(|_| format!("EVM address at {path} is not UTF-8"))?
        .trim();
    normalize_address(address).map_err(|error| format!("invalid EVM address at {path}: {error}"))
}

pub fn normalize_address(value: &str) -> Result<String, String> {
    let value = value.to_ascii_lowercase();
    if value.len() == 42
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(value)
    } else {
        Err("wallet must be a 20-byte EVM address".into())
    }
}

#[cfg(test)]
mod wallet_tests {
    use super::*;

    #[test]
    fn the_address_is_read_from_the_account_scoped_evm_leaf() {
        assert_eq!(wallet_address_path("main"), "wallets/main/0/address.evm");
    }

    #[test]
    fn a_missing_or_malformed_leaf_names_its_path() {
        let path = wallet_address_path("main");
        assert_eq!(
            parse_wallet_address(&path, b"0x52908400098527886E0F7030069857D2E4169EE7\n").unwrap(),
            "0x52908400098527886e0f7030069857d2e4169ee7"
        );
        for bad in [&b"not-an-address"[..], &[0xff, 0xfe][..]] {
            let err = parse_wallet_address(&path, bad).unwrap_err();
            assert!(err.contains("wallets/main/0/address.evm"), "{err}");
        }
    }

    #[test]
    fn no_route_source_reads_the_retired_wallet_root_leaves() {
        fn visit(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("read source dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    visit(&path, hits);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && !path.ends_with("src/lib.rs")
                {
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
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut hits = Vec::new();
        visit(&root.join("src"), &mut hits);
        visit(&root.join("files"), &mut hits);
        assert!(hits.is_empty(), "{hits:?}");
    }
}

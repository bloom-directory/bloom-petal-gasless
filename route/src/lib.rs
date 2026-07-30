//! Relay permit protocol, generic transaction state, and legacy deposit compatibility.

mod legacy;
mod relay;

pub use legacy::{
    GaslessDepositRequest, SOURCE_CHAINS, SourceChain, gasless_deposit, gasless_deposit_status,
    source_chain,
};
pub use relay::{
    PermitDomain, RelayDestination, RelayOrigin, RelayTransactionRequest, gasless_transaction,
    gasless_transaction_status,
};
pub use serde_json;

pub fn wallet_address(wallet: &str) -> Result<String, petal::DispatchResponse> {
    let path = format!("wallets/{wallet}/address");
    let bytes =
        petal::sdk::vfs_read(&path, 128).map_err(|error| petal::error(-4, error.message()))?;
    let address = core::str::from_utf8(&bytes)
        .map_err(|_| petal::error(-4, "wallet address is not UTF-8"))?
        .trim();
    normalize_address(address).map_err(|error| petal::error(-4, error))
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

//! Shared Relay protocol constants, host abstraction, and utilities.

use alloy_dyn_abi::eip712::TypedData;
use alloy_primitives::B256;
use serde_json::{Map, Value, json};

use petal::{
    DispatchResponse, HostStatus, HttpRequest, HttpResponse, SdkError, SignHashOutcome, SignRequest,
};

pub(crate) const RELAY: &str = "https://api.relay.link";
pub(crate) const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";
// Relay v3 ApprovalProxy. Every permit quote must independently prove that the
// authorization is addressed to this receiver before Bloom asks the owner to
// sign it.
pub(crate) const RELAY_PERMIT_RECEIVER: &str = "0xccc88a9d1b4ed6b0eaba998850414b24f1c315be";
pub(crate) const MAX_BODY: usize = 512 * 1024;
pub(crate) const MAX_DECIMALS: u8 = 38;
pub(crate) const PERMIT_SUBMISSION_MARGIN_SECONDS: u64 = 30;

/// Host trait abstracting store, HTTP, signing, and time so that the module
/// shares a single production implementation and test mocks.
pub(crate) trait Host {
    fn store_get(&mut self, key: &str, max_bytes: usize) -> Result<Option<Vec<u8>>, String>;
    fn store_put(&mut self, key: &str, value: &[u8]) -> Result<(), String>;
    fn store_put_new(&mut self, key: &str, value: &[u8]) -> Result<bool, String>;
    fn http_fetch(
        &mut self,
        request: &HttpRequest,
        max_bytes: usize,
    ) -> Result<HttpResponse, String>;
    fn sign_hash(&mut self, request: &SignRequest) -> Result<SignHashOutcome, String>;
    fn now_ms(&mut self) -> Result<u64, String>;
}

pub(crate) struct BloomHost;

impl Host for BloomHost {
    fn store_get(&mut self, key: &str, max_bytes: usize) -> Result<Option<Vec<u8>>, String> {
        match petal::sdk::store_get(key, max_bytes) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(SdkError::Host(HostStatus::NotFound)) => Ok(None),
            Err(error) => Err(error.message()),
        }
    }

    fn store_put(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
        petal::sdk::store_put(key, value, false).map_err(|error| error.message())
    }

    fn store_put_new(&mut self, key: &str, value: &[u8]) -> Result<bool, String> {
        match petal::sdk::store_put_new(key, value, false) {
            Ok(()) => Ok(true),
            Err(SdkError::Host(HostStatus::Denied)) => Ok(false),
            Err(error) => Err(error.message()),
        }
    }

    fn http_fetch(
        &mut self,
        request: &HttpRequest,
        max_bytes: usize,
    ) -> Result<HttpResponse, String> {
        petal::sdk::http_fetch(request, max_bytes).map_err(|error| error.message())
    }

    fn sign_hash(&mut self, request: &SignRequest) -> Result<SignHashOutcome, String> {
        petal::sdk::sign_hash(request).map_err(|error| error.message())
    }

    fn now_ms(&mut self) -> Result<u64, String> {
        petal::sdk::try_now_ms().map_err(|error| error.message())
    }
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

pub(crate) fn invalid(message: impl Into<String>) -> DispatchResponse {
    petal::error(-3, message)
}

pub(crate) fn denied(message: impl Into<String>) -> DispatchResponse {
    petal::error(-2, message)
}

pub(crate) fn backend(message: impl Into<String>) -> DispatchResponse {
    petal::error(-4, message)
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

pub(crate) fn fetch<H: Host>(
    host: &mut H,
    method: &str,
    path: &str,
    body: Vec<u8>,
) -> Result<Value, DispatchResponse> {
    let response = host
        .http_fetch(
            &HttpRequest {
                method: method.into(),
                url: format!("{RELAY}{path}"),
                headers: vec![("content-type".into(), "application/json".into())],
                body,
            },
            MAX_BODY,
        )
        .map_err(backend)?;
    let value: Value = serde_json::from_slice(&response.body)
        .map_err(|error| backend(format!("Relay returned invalid JSON: {error}")))?;
    if !(200..300).contains(&response.status) {
        return Err(backend(format!(
            "Relay API status {}: {}",
            response.status,
            compact(&value)
        )));
    }
    Ok(value)
}

pub(crate) fn submit_permit<H: Host>(
    host: &mut H,
    signature: &str,
    body: Vec<u8>,
) -> Result<(), ()> {
    // Relay requires the signature in the URL. Never return the HTTP host's
    // error because it may contain the complete replayable URL.
    let response = host
        .http_fetch(
            &HttpRequest {
                method: "POST".into(),
                url: format!("{RELAY}/execute/permits?signature={signature}"),
                headers: vec![("content-type".into(), "application/json".into())],
                body,
            },
            MAX_BODY,
        )
        .map_err(|_| ())?;
    if !(200..300).contains(&response.status) {
        return Err(());
    }
    serde_json::from_slice::<Value>(&response.body)
        .map(|_| ())
        .map_err(|_| ())
}

pub(crate) fn compact(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "<invalid>".into())
        .chars()
        .take(4096)
        .collect()
}

// ---------------------------------------------------------------------------
// Validation utilities
// ---------------------------------------------------------------------------

pub(crate) fn is_bytes32(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn uint64_value(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

pub(crate) fn is_safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

// ---------------------------------------------------------------------------
// Signing utilities
// ---------------------------------------------------------------------------

pub(crate) fn signature_hex(mut bytes: Vec<u8>) -> Result<String, DispatchResponse> {
    if bytes.len() != 65 {
        return Err(backend("wallet returned a non-EVM signature"));
    }
    if bytes[64] < 27 {
        bytes[64] += 27;
    }
    if !matches!(bytes[64], 27 | 28) {
        return Err(backend("wallet returned an invalid EVM recovery ID"));
    }
    Ok(format!("0x{}", hex::encode(bytes)))
}

pub(crate) fn signing_hash(sign: &Value) -> Result<B256, DispatchResponse> {
    let primary_type = sign
        .get("primaryType")
        .and_then(Value::as_str)
        .ok_or_else(|| backend("Relay typed data omitted its primary type"))?;
    let mut types = Map::new();
    types.insert(
        "EIP712Domain".into(),
        json!([
            {"name":"name","type":"string"},
            {"name":"version","type":"string"},
            {"name":"chainId","type":"uint256"},
            {"name":"verifyingContract","type":"address"}
        ]),
    );
    types.insert(
        primary_type.into(),
        sign.pointer(&format!("/types/{primary_type}"))
            .cloned()
            .ok_or_else(|| backend("Relay typed data omitted its authorization type"))?,
    );
    let typed: TypedData = serde_json::from_value(json!({
        "types": types,
        "primaryType": primary_type,
        "domain": sign.get("domain"),
        "message": sign.get("value")
    }))
    .map_err(|error| backend(format!("invalid Relay typed data: {error}")))?;
    typed
        .eip712_signing_hash()
        .map_err(|error| backend(format!("cannot hash Relay typed data: {error}")))
}

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    #[derive(Default)]
    pub(crate) struct MockHost {
        pub(crate) store: HashMap<String, Vec<u8>>,
        pub(crate) http_results: VecDeque<Result<HttpResponse, String>>,
        pub(crate) sign_results: VecDeque<Result<SignHashOutcome, String>>,
        pub(crate) requests: Vec<HttpRequest>,
        pub(crate) sign_requests: Vec<SignRequest>,
        pub(crate) now_ms: u64,
    }

    impl MockHost {
        pub(crate) fn push_json(&mut self, status: u16, value: Value) {
            self.http_results.push_back(Ok(HttpResponse {
                status,
                headers: Vec::new(),
                body: serde_json::to_vec(&value).unwrap(),
            }));
        }
    }

    impl Host for MockHost {
        fn store_get(&mut self, key: &str, _max_bytes: usize) -> Result<Option<Vec<u8>>, String> {
            Ok(self.store.get(key).cloned())
        }

        fn store_put(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
            self.store.insert(key.into(), value.to_vec());
            Ok(())
        }

        fn store_put_new(&mut self, key: &str, value: &[u8]) -> Result<bool, String> {
            if self.store.contains_key(key) {
                return Ok(false);
            }
            self.store.insert(key.into(), value.to_vec());
            Ok(true)
        }

        fn http_fetch(
            &mut self,
            request: &HttpRequest,
            _max_bytes: usize,
        ) -> Result<HttpResponse, String> {
            self.requests.push(request.clone());
            self.http_results
                .pop_front()
                .expect("unexpected HTTP request")
        }

        fn sign_hash(&mut self, request: &SignRequest) -> Result<SignHashOutcome, String> {
            self.sign_requests.push(request.clone());
            self.sign_results
                .pop_front()
                .expect("unexpected signing request")
        }

        fn now_ms(&mut self) -> Result<u64, String> {
            Ok(self.now_ms)
        }
    }

    pub(crate) fn approval() -> SignHashOutcome {
        SignHashOutcome::ApprovalRequired {
            action_id: "approval-1".into(),
            ceremony_url: "http://127.0.0.1/approve/approval-1".into(),
            expires_ms: 1_500_000,
        }
    }

    pub(crate) fn signature() -> SignHashOutcome {
        let mut bytes = vec![0xab; 65];
        bytes[64] = 0;
        SignHashOutcome::Signature(bytes)
    }
}

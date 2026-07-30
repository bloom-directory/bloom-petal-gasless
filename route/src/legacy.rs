//! Compatibility implementation for pre-generic HyperCore deposit operations.

use alloy_dyn_abi::eip712::TypedData;
use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use petal::{
    DispatchResponse, HostStatus, HttpRequest, HttpResponse, SdkError, SignHashOutcome, SignRequest,
};

const RELAY: &str = "https://api.relay.link";
const HYPERLIQUID_USDC: &str = "0x00000000000000000000000000000000";
// Relay v3 ApprovalProxy. Live quotes currently use the same receiver on every
// supported source, but the receiver remains part of each registry entry so a
// future chain-specific change must be reviewed explicitly.
const RELAY_PERMIT_RECEIVER: &str = "0xccc88a9d1b4ed6b0eaba998850414b24f1c315be";
const MAX_BODY: usize = 512 * 1024;
const HYPERCORE_USDC_DECIMALS: usize = 8;
const PERMIT_SUBMISSION_MARGIN_SECONDS: u64 = 30;
const SUBMISSION_UNKNOWN: &str =
    "Relay permit submission outcome is unknown; read this deposit to reconcile its status";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceChain {
    pub slug: &'static str,
    pub chain_id: u64,
    pub usdc: &'static str,
    pub usdc_decimals: usize,
    pub permit_receiver: &'static str,
}

pub const SOURCE_CHAINS: [SourceChain; 6] = [
    SourceChain {
        slug: "ethereum",
        chain_id: 1,
        usdc: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        usdc_decimals: 6,
        permit_receiver: RELAY_PERMIT_RECEIVER,
    },
    SourceChain {
        slug: "base",
        chain_id: 8453,
        usdc: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        usdc_decimals: 6,
        permit_receiver: RELAY_PERMIT_RECEIVER,
    },
    SourceChain {
        slug: "arbitrum",
        chain_id: 42161,
        usdc: "0xaf88d065e77c8cc2239327c5edb3a432268e5831",
        usdc_decimals: 6,
        permit_receiver: RELAY_PERMIT_RECEIVER,
    },
    SourceChain {
        slug: "optimism",
        chain_id: 10,
        usdc: "0x0b2c639c533813f4aa9d7837caf62653d097ff85",
        usdc_decimals: 6,
        permit_receiver: RELAY_PERMIT_RECEIVER,
    },
    SourceChain {
        slug: "polygon",
        chain_id: 137,
        usdc: "0x3c499c542cef5e3811e1192ce70d8cc03d5c3359",
        usdc_decimals: 6,
        permit_receiver: RELAY_PERMIT_RECEIVER,
    },
    SourceChain {
        slug: "avalanche",
        chain_id: 43114,
        usdc: "0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e",
        usdc_decimals: 6,
        permit_receiver: RELAY_PERMIT_RECEIVER,
    },
];

pub fn source_chain(slug: &str) -> Result<SourceChain, DispatchResponse> {
    SOURCE_CHAINS
        .iter()
        .copied()
        .find(|chain| chain.slug == slug)
        .ok_or_else(|| invalid(format!("unsupported source chain: {slug}")))
}

trait Host {
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

struct BloomHost;

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

fn default_source_chain() -> String {
    "ethereum".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaslessDepositRequest {
    pub amount: String,
    pub minimum_output: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DepositState {
    schema: String,
    #[serde(default = "default_source_chain")]
    source_chain: String,
    wallet: String,
    address: String,
    id: String,
    amount: String,
    amount_units: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    minimum_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    minimum_output_units: Option<String>,
    request_id: String,
    phase: String,
    sign: Value,
    quote: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission: Option<String>,
}

fn invalid(message: impl Into<String>) -> DispatchResponse {
    petal::error(-3, message)
}

fn denied(message: impl Into<String>) -> DispatchResponse {
    petal::error(-2, message)
}

fn backend(message: impl Into<String>) -> DispatchResponse {
    petal::error(-4, message)
}

fn key(chain: SourceChain, wallet: &str, id: &str) -> String {
    if chain.slug == "ethereum" {
        // The historical namespace is permanently Ethereum-only. Keeping it
        // authoritative makes legacy and canonical Ethereum routes converge.
        format!("state/gasless-deposits/{wallet}/{id}.json")
    } else {
        format!(
            "state/gasless-deposits/by-chain/{}/{wallet}/{id}.json",
            chain.slug
        )
    }
}

fn load<H: Host>(
    host: &mut H,
    chain: SourceChain,
    wallet: &str,
    id: &str,
) -> Result<Option<DepositState>, DispatchResponse> {
    match host.store_get(&key(chain, wallet, id), MAX_BODY) {
        Ok(Some(bytes)) => {
            let state: DepositState = serde_json::from_slice(&bytes)
                .map_err(|error| backend(format!("stored deposit is invalid: {error}")))?;
            if state.source_chain != chain.slug {
                return Err(backend(
                    "stored deposit source chain does not match its path",
                ));
            }
            Ok(Some(state))
        }
        Ok(None) => Ok(None),
        Err(error) => Err(backend(error)),
    }
}

fn save<H: Host>(host: &mut H, state: &DepositState) -> Result<(), DispatchResponse> {
    let chain = source_chain(&state.source_chain)
        .map_err(|_| backend("stored deposit has an unsupported source chain"))?;
    let bytes = serde_json::to_vec(state).map_err(|error| backend(error.to_string()))?;
    host.store_put(&key(chain, &state.wallet, &state.id), &bytes)
        .map_err(backend)
}

fn save_new<H: Host>(host: &mut H, state: &DepositState) -> Result<bool, DispatchResponse> {
    let chain = source_chain(&state.source_chain)
        .map_err(|_| backend("stored deposit has an unsupported source chain"))?;
    let bytes = serde_json::to_vec(state).map_err(|error| backend(error.to_string()))?;
    host.store_put_new(&key(chain, &state.wallet, &state.id), &bytes)
        .map_err(backend)
}

fn fetch<H: Host>(
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

fn submit_permit<H: Host>(host: &mut H, signature: &str, body: Vec<u8>) -> Result<(), ()> {
    // Relay requires the signature query parameter. Do not propagate any host
    // error from this request: Bloom's HTTP host may include the complete URL.
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

fn compact(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "<invalid>".into())
        .chars()
        .take(4096)
        .collect()
}

fn is_bytes32(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn decimal_units(amount: &str, decimals: usize, field: &str) -> Result<String, String> {
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > decimals
    {
        return Err(format!(
            "{field} must be a positive decimal with at most {decimals} places"
        ));
    }
    let whole = whole
        .parse::<u128>()
        .map_err(|_| format!("{field} is too large"))?;
    let fraction = format!("{fraction:0<decimals$}")
        .parse::<u128>()
        .map_err(|_| format!("invalid {field}"))?;
    let scale = 10_u128
        .checked_pow(decimals as u32)
        .ok_or_else(|| format!("{field} has unsupported precision"))?;
    let units = whole
        .checked_mul(scale)
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(|| format!("{field} is too large"))?;
    if units == 0 {
        return Err(format!("{field} must be positive"));
    }
    Ok(units.to_string())
}

fn format_units(units: u128, decimals: usize) -> String {
    let scale = 10_u128.pow(decimals as u32);
    let whole = units / scale;
    let fraction = units % scale;
    if fraction == 0 {
        return whole.to_string();
    }
    format!("{whole}.{fraction:0>decimals$}")
        .trim_end_matches('0')
        .to_string()
}

#[derive(Clone, Copy)]
struct QuoteInput<'a> {
    wallet: &'a str,
    address: &'a str,
    amount: &'a str,
    units: &'a str,
    minimum_output: &'a str,
    minimum_output_units: &'a str,
}

fn quote<H: Host>(
    host: &mut H,
    chain: SourceChain,
    input: QuoteInput<'_>,
) -> Result<DepositState, DispatchResponse> {
    let body = serde_json::to_vec(&json!({
        "user": input.address,
        "originChainId": chain.chain_id,
        "destinationChainId": 1337,
        "originCurrency": chain.usdc,
        "destinationCurrency": HYPERLIQUID_USDC,
        "recipient": input.address,
        "tradeType": "EXACT_INPUT",
        "amount": input.units,
        "refundTo": input.address,
        "usePermit": true,
        "slippageTolerance": "50"
    }))
    .map_err(|error| backend(error.to_string()))?;
    let response = fetch(host, "POST", "/quote/v2", body)?;
    prepare_quote(chain, input, response)
}

fn prepare_quote(
    chain: SourceChain,
    input: QuoteInput<'_>,
    response: Value,
) -> Result<DepositState, DispatchResponse> {
    let step = response
        .get("steps")
        .and_then(Value::as_array)
        .and_then(|steps| {
            steps
                .iter()
                .find(|step| step.get("id").and_then(Value::as_str) == Some("authorize1"))
        })
        .ok_or_else(|| {
            backend(format!(
                "Relay offered no gasless USDC authorization: {}",
                compact(&response)
            ))
        })?;
    if step.get("kind").and_then(Value::as_str) != Some("signature") {
        return Err(backend("Relay gasless step was not a signature"));
    }
    let item = step
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| backend("Relay gasless step had no item"))?;
    let sign = item
        .pointer("/data/sign")
        .cloned()
        .ok_or_else(|| backend("Relay gasless step omitted signing data"))?;
    let request_id = item
        .pointer("/data/post/body/requestId")
        .and_then(Value::as_str)
        .ok_or_else(|| backend("Relay gasless step omitted request id"))?;
    if !is_bytes32(request_id) {
        return Err(backend("Relay gasless step returned an invalid request id"));
    }

    let (_, permit_valid_before) = validate_sign(chain, input.address, input.units, &sign)?;
    if item.pointer("/data/post/endpoint").and_then(Value::as_str) != Some("/execute/permits")
        || item.pointer("/data/post/method").and_then(Value::as_str) != Some("POST")
        || item.pointer("/data/post/body/kind").and_then(Value::as_str) != Some("eip3009")
        || item.pointer("/data/post/body/api").and_then(Value::as_str) != Some("swap")
        || step.get("requestId").and_then(Value::as_str) != Some(request_id)
    {
        return Err(backend(
            "Relay returned an unexpected permit submission contract",
        ));
    }
    let expected_check = format!("/intents/status/v3?requestId={request_id}");
    if item.pointer("/check/endpoint").and_then(Value::as_str) != Some(&expected_check) {
        return Err(backend("Relay returned an unexpected status endpoint"));
    }
    let details = response
        .get("details")
        .ok_or_else(|| backend("Relay quote omitted details"))?;
    if details
        .pointer("/currencyIn/currency/chainId")
        .and_then(Value::as_u64)
        != Some(chain.chain_id)
        || details
            .pointer("/currencyIn/currency/address")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some(chain.usdc)
        || details
            .pointer("/currencyIn/currency/decimals")
            .and_then(Value::as_u64)
            != Some(chain.usdc_decimals as u64)
        || details
            .pointer("/currencyOut/currency/chainId")
            .and_then(Value::as_u64)
            != Some(1337)
        || details
            .pointer("/currencyOut/currency/address")
            .and_then(Value::as_str)
            != Some(HYPERLIQUID_USDC)
        || details
            .pointer("/currencyOut/currency/decimals")
            .and_then(Value::as_u64)
            != Some(HYPERCORE_USDC_DECIMALS as u64)
        || details
            .pointer("/refundCurrency/currency/chainId")
            .and_then(Value::as_u64)
            != Some(chain.chain_id)
        || details
            .pointer("/refundCurrency/currency/address")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some(chain.usdc)
        || details
            .pointer("/refundCurrency/currency/decimals")
            .and_then(Value::as_u64)
            != Some(chain.usdc_decimals as u64)
        || details.get("recipient").and_then(Value::as_str) != Some(input.address)
    {
        return Err(backend(
            "Relay quote changed the requested asset, chain, or recipient",
        ));
    }
    if details
        .pointer("/currencyIn/amount")
        .and_then(Value::as_str)
        != Some(input.units)
        || details
            .pointer("/refundCurrency/amount")
            .and_then(Value::as_str)
            != Some(input.units)
    {
        return Err(backend("Relay quote changed the requested input amount"));
    }
    validate_refunds(&response, chain, input.address)?;
    let amount_out_units = details
        .pointer("/currencyOut/amount")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u128>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| backend("Relay quote omitted a valid output amount"))?;
    let relay_minimum_out_units = details
        .pointer("/currencyOut/minimumAmount")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u128>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| backend("Relay quote omitted a valid minimum output"))?;
    let required_minimum_out_units = input
        .minimum_output_units
        .parse::<u128>()
        .map_err(|_| invalid("minimum_output is too large"))?;
    if relay_minimum_out_units > amount_out_units {
        return Err(backend(
            "Relay quote minimum output exceeded its quoted output",
        ));
    }
    if relay_minimum_out_units < required_minimum_out_units {
        return Err(invalid(format!(
            "Relay slippage-adjusted minimum output {} is below required minimum_output {}",
            format_units(relay_minimum_out_units, HYPERCORE_USDC_DECIMALS),
            input.minimum_output,
        )));
    }

    Ok(DepositState {
        schema: "bloom.gasless.deposit.v2".into(),
        source_chain: chain.slug.into(),
        wallet: input.wallet.into(),
        address: input.address.into(),
        id: String::new(),
        amount: input.amount.into(),
        amount_units: input.units.into(),
        minimum_output: Some(input.minimum_output.into()),
        minimum_output_units: Some(input.minimum_output_units.into()),
        request_id: request_id.into(),
        phase: "awaiting_signature".into(),
        sign,
        quote: json!({
            "amount_in": details.pointer("/currencyIn/amountFormatted"),
            "amount_out": details.pointer("/currencyOut/amountFormatted"),
            "minimum_out_units": details.pointer("/currencyOut/minimumAmount"),
            "required_minimum_out": input.minimum_output,
            "required_minimum_out_units": input.minimum_output_units,
            "total_impact": details.get("totalImpact"),
            "time_estimate_seconds": details.get("timeEstimate"),
            "permit_valid_before_unix_seconds": permit_valid_before,
            "source_chain": chain.slug,
            "source_chain_id": chain.chain_id,
            "source_currency": chain.usdc,
            "provider": "relay"
        }),
        approval: None,
        submission: None,
    })
}

fn validate_refunds(
    response: &Value,
    chain: SourceChain,
    address: &str,
) -> Result<(), DispatchResponse> {
    let refunds = response
        .pointer("/protocol/v2/orderData/inputs/0/refunds")
        .and_then(Value::as_array)
        .ok_or_else(|| backend("Relay quote omitted its refund plan"))?;
    let matches = |refund: &Value, expected_chain: &str, expected_currency: &str| {
        refund.get("chainId").and_then(Value::as_str) == Some(expected_chain)
            && refund
                .get("recipient")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
                .as_deref()
                == Some(address)
            && refund
                .get("currency")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
                .as_deref()
                == Some(expected_currency)
    };
    if refunds.len() != 2
        || !refunds
            .iter()
            .any(|refund| matches(refund, chain.slug, chain.usdc))
        || !refunds
            .iter()
            .any(|refund| matches(refund, "hyperliquid", HYPERLIQUID_USDC))
    {
        return Err(backend(
            "Relay quote changed the requested refund address, asset, or chain",
        ));
    }
    Ok(())
}

fn uint64_value(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn validate_sign(
    chain: SourceChain,
    wallet: &str,
    units: &str,
    sign: &Value,
) -> Result<(u64, u64), DispatchResponse> {
    let expected_types = json!([
        {"name":"from","type":"address"},
        {"name":"to","type":"address"},
        {"name":"value","type":"uint256"},
        {"name":"validAfter","type":"uint256"},
        {"name":"validBefore","type":"uint256"},
        {"name":"nonce","type":"bytes32"}
    ]);
    let from = sign.pointer("/value/from").and_then(Value::as_str);
    let to = sign.pointer("/value/to").and_then(Value::as_str);
    let nonce = sign.pointer("/value/nonce").and_then(Value::as_str);
    let valid_after = uint64_value(sign.pointer("/value/validAfter"));
    let valid_before = uint64_value(sign.pointer("/value/validBefore"));
    if sign.get("signatureKind").and_then(Value::as_str) != Some("eip712")
        || sign.get("primaryType").and_then(Value::as_str) != Some("ReceiveWithAuthorization")
        || sign.pointer("/domain/name").and_then(Value::as_str) != Some("USD Coin")
        || sign.pointer("/domain/version").and_then(Value::as_str) != Some("2")
        || uint64_value(sign.pointer("/domain/chainId")) != Some(chain.chain_id)
        || sign
            .pointer("/domain/verifyingContract")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some(chain.usdc)
        || sign.pointer("/types/ReceiveWithAuthorization") != Some(&expected_types)
        || from.map(str::to_ascii_lowercase).as_deref() != Some(wallet)
        || sign.pointer("/value/value").and_then(Value::as_str) != Some(units)
        || to.map(str::to_ascii_lowercase).as_deref() != Some(chain.permit_receiver)
        || nonce.is_none_or(|value| !is_bytes32(value))
        || valid_after
            .zip(valid_before)
            .is_none_or(|(after, before)| after >= before)
    {
        return Err(backend("Relay returned unsafe EIP-3009 signing data"));
    }
    Ok((valid_after.unwrap(), valid_before.unwrap()))
}

fn signing_hash(sign: &Value) -> Result<B256, DispatchResponse> {
    let typed: TypedData = serde_json::from_value(json!({
        "types": {
            "EIP712Domain": [
                {"name":"name","type":"string"},
                {"name":"version","type":"string"},
                {"name":"chainId","type":"uint256"},
                {"name":"verifyingContract","type":"address"}
            ],
            "ReceiveWithAuthorization": sign.pointer("/types/ReceiveWithAuthorization")
        },
        "primaryType": "ReceiveWithAuthorization",
        "domain": sign.get("domain"),
        "message": sign.get("value")
    }))
    .map_err(|error| backend(format!("invalid Relay typed data: {error}")))?;
    typed
        .eip712_signing_hash()
        .map_err(|error| backend(format!("cannot hash Relay typed data: {error}")))
}

fn signature_hex(mut bytes: Vec<u8>) -> Result<String, DispatchResponse> {
    if bytes.len() != 65 {
        return Err(backend("wallet returned a non-EVM signature"));
    }
    if bytes[64] < 27 {
        bytes[64] += 27;
    }
    if !matches!(bytes[64], 27 | 28) {
        return Err(backend("wallet returned an invalid EVM recovery id"));
    }
    Ok(format!("0x{}", hex::encode(bytes)))
}

fn ensure_permit_live<H: Host>(
    host: &mut H,
    chain: SourceChain,
    wallet: &str,
    units: &str,
    sign: &Value,
) -> Result<(), DispatchResponse> {
    let (valid_after, valid_before) = validate_sign(chain, wallet, units, sign)?;
    let now_seconds = host
        .now_ms()
        .map_err(|error| backend(format!("cannot check Relay permit expiry: {error}")))?
        / 1_000;
    if now_seconds < valid_after {
        return Err(invalid("Relay quote permit is not valid yet"));
    }
    if now_seconds.saturating_add(PERMIT_SUBMISSION_MARGIN_SECONDS) >= valid_before {
        return Err(invalid(
            "Relay quote permit has expired or is too close to expiry; use a new deposit id",
        ));
    }
    Ok(())
}

fn retry_write_body(state: &DepositState) -> Value {
    match state.minimum_output.as_deref() {
        Some(minimum_output) => json!({
            "amount": state.amount,
            "minimum_output": minimum_output
        }),
        None => json!({"amount": state.amount}),
    }
}

fn validate_request_constraints(
    state: &DepositState,
    address: &str,
    units: &str,
    minimum_output_units: &str,
) -> Result<(), DispatchResponse> {
    if state.address != address
        || state.amount_units != units
        || state.minimum_output_units.as_deref() != Some(minimum_output_units)
    {
        return Err(invalid(
            "deposit id already belongs to a different wallet address, amount, or minimum_output constraint",
        ));
    }
    Ok(())
}

fn resolve_initialization_conflict(
    existing: Option<DepositState>,
    address: &str,
    units: &str,
    minimum_output_units: &str,
) -> Result<DepositState, DispatchResponse> {
    let existing = existing
        .ok_or_else(|| backend("deposit initialization conflicted but no state was found"))?;
    validate_request_constraints(&existing, address, units, minimum_output_units)?;
    Ok(existing)
}

pub fn gasless_deposit(
    source: &str,
    wallet: String,
    address: String,
    id: String,
    request: GaslessDepositRequest,
) -> DispatchResponse {
    gasless_deposit_with_host(&mut BloomHost, source, wallet, address, id, request)
}

fn gasless_deposit_with_host<H: Host>(
    host: &mut H,
    source: &str,
    wallet: String,
    address: String,
    id: String,
    request: GaslessDepositRequest,
) -> DispatchResponse {
    let chain = match source_chain(source) {
        Ok(chain) => chain,
        Err(error) => return error,
    };
    let units = match decimal_units(&request.amount, chain.usdc_decimals, "amount") {
        Ok(units) => units,
        Err(error) => return invalid(error),
    };
    let minimum_output_units = match decimal_units(
        &request.minimum_output,
        HYPERCORE_USDC_DECIMALS,
        "minimum_output",
    ) {
        Ok(units) => units,
        Err(error) => return invalid(error),
    };
    let mut state = match load(host, chain, &wallet, &id) {
        Ok(Some(state)) => {
            if let Err(error) =
                validate_request_constraints(&state, &address, &units, &minimum_output_units)
            {
                return error;
            }
            if state.phase == "submitted" {
                return DispatchResponse::Write;
            }
            state
        }
        Ok(None) => match quote(
            host,
            chain,
            QuoteInput {
                wallet: &wallet,
                address: &address,
                amount: &request.amount,
                units: &units,
                minimum_output: &request.minimum_output,
                minimum_output_units: &minimum_output_units,
            },
        ) {
            Ok(mut state) => {
                state.id.clone_from(&id);
                match save_new(host, &state) {
                    Ok(true) => state,
                    Ok(false) => match load(host, chain, &wallet, &id) {
                        Ok(existing) => match resolve_initialization_conflict(
                            existing,
                            &address,
                            &units,
                            &minimum_output_units,
                        ) {
                            Ok(existing) => existing,
                            Err(error) => return error,
                        },
                        Err(error) => return error,
                    },
                    Err(error) => return error,
                }
            }
            Err(error) => return error,
        },
        Err(error) => return error,
    };
    if let Err(error) = ensure_permit_live(
        host,
        chain,
        &state.address,
        &state.amount_units,
        &state.sign,
    ) {
        return error;
    }
    let hash = match signing_hash(&state.sign) {
        Ok(hash) => hash,
        Err(error) => return error,
    };
    let mut hash32 = [0_u8; 32];
    hash32.copy_from_slice(hash.as_slice());
    let signature = match host.sign_hash(&SignRequest {
        wallet: wallet.clone(),
        hash32,
        purpose: "gasless.deposit".into(),
    }) {
        Ok(SignHashOutcome::Signature(bytes)) => match signature_hex(bytes) {
            Ok(signature) => signature,
            Err(error) => return error,
        },
        Ok(SignHashOutcome::ApprovalRequired {
            action_id,
            ceremony_url,
            expires_ms,
        }) => {
            let retry_write_body = retry_write_body(&state);
            state.phase = "approval_required".into();
            state.approval = Some(json!({
                "action_id": action_id,
                "ceremony_url": ceremony_url,
                "expires_ms": expires_ms,
                "retry_write_body": retry_write_body
            }));
            if let Err(error) = save(host, &state) {
                return error;
            }
            return denied(format!(
                "review quote, approve the gasless Hyperliquid deposit, then retry the exact write: {}",
                compact(state.approval.as_ref().unwrap())
            ));
        }
        Err(error) => return denied(format!("signing denied: {error}")),
    };
    let body = serde_json::to_vec(&json!({
        "kind": "eip3009",
        "requestId": state.request_id,
        "api": "swap"
    }))
    .unwrap();
    state.phase = "submitting".into();
    state.approval = None;
    if let Err(error) = save(host, &state) {
        return error;
    }
    match submit_permit(host, &signature, body) {
        Ok(()) => {}
        Err(()) => {
            state.phase = "submission_unknown".into();
            state.submission = Some("unknown".into());
            if let Err(save_error) = save(host, &state) {
                return save_error;
            }
            return backend(SUBMISSION_UNKNOWN);
        }
    }
    state.phase = "submitted".into();
    state.approval = None;
    state.submission = Some("accepted".into());
    if let Err(error) = save(host, &state) {
        return error;
    }
    DispatchResponse::Write
}

fn attempted_submission(phase: &str) -> bool {
    matches!(
        phase,
        "submitting" | "submission_unknown" | "submission_failed" | "submitted"
    )
}

fn relay_status_projection(value: &Value) -> Option<Value> {
    let status = value.get("status").and_then(Value::as_str)?;
    if !matches!(
        status,
        "waiting"
            | "depositing"
            | "pending"
            | "submitted"
            | "success"
            | "delayed"
            | "refund"
            | "failure"
    ) {
        return None;
    }
    let valid_hashes = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_array)
            .map(|hashes| {
                hashes
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|hash| is_bytes32(hash))
                    .map(|hash| Value::String(hash.into()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    Some(json!({
        "status": status,
        "in_tx_hashes": valid_hashes("inTxHashes"),
        "tx_hashes": valid_hashes("txHashes"),
        "updated_at": value.get("updatedAt").and_then(Value::as_u64),
        "origin_chain_id": value.get("originChainId").and_then(Value::as_u64),
        "destination_chain_id": value.get("destinationChainId").and_then(Value::as_u64)
    }))
}

fn public_status(phase: &str, relay_status: Option<&Value>) -> String {
    relay_status
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .filter(|status| *status != "unavailable")
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if phase == "submission_failed" {
                "submission_unknown".into()
            } else {
                phase.into()
            }
        })
}

fn effective_local_phase(state: &DepositState, now_ms: u64) -> String {
    if attempted_submission(&state.phase) || now_ms == 0 {
        return state.phase.clone();
    }
    if state.minimum_output_units.is_none() {
        return "quote_unbounded".into();
    }
    let now_seconds = now_ms / 1_000;
    if uint64_value(state.sign.pointer("/value/validBefore")).is_some_and(|valid_before| {
        now_seconds.saturating_add(PERMIT_SUBMISSION_MARGIN_SECONDS) >= valid_before
    }) {
        return "quote_expired".into();
    }
    if state.phase == "approval_required"
        && state
            .approval
            .as_ref()
            .and_then(|approval| approval.get("expires_ms"))
            .and_then(Value::as_u64)
            .is_some_and(|expires_ms| now_ms >= expires_ms)
    {
        return "approval_expired".into();
    }
    state.phase.clone()
}

fn next_action(state: &DepositState, status: &str) -> Value {
    match status {
        "approval_required" => json!({
            "action": "review_quote_then_approve",
            "instruction": "Review quote and required minimum output, open approval.ceremony_url, then retry the exact write body.",
            "retry_write_body": retry_write_body(state)
        }),
        "approval_expired" => json!({
            "action": "retry_write",
            "instruction": "The ceremony expired. Retry this exact write body to obtain a fresh ceremony for the same Relay request.",
            "retry_write_body": retry_write_body(state)
        }),
        "quote_expired" => json!({
            "action": "create_new_deposit",
            "instruction": "The Relay permit expired. Use a new deposit id to obtain and review a new quote; this deposit will not silently re-quote."
        }),
        "quote_unbounded" => json!({
            "action": "create_new_deposit",
            "instruction": "This legacy quote has no caller-defined minimum output. Use a new deposit id with minimum_output; it is unsafe to continue this deposit."
        }),
        "submitting" | "submission_unknown" | "submission_failed" | "submitted" | "waiting"
        | "depositing" | "pending" | "delayed" => json!({
            "action": "poll",
            "instruction": "Read this deposit again; only Relay status success means completion."
        }),
        "success" => json!({
            "action": "complete",
            "instruction": "Relay reports successful destination settlement."
        }),
        "refund" | "failure" => json!({
            "action": "inspect",
            "instruction": "Relay reached a terminal non-success status; inspect the projected transaction hashes."
        }),
        _ => json!({
            "action": "retry_write",
            "retry_write_body": retry_write_body(state)
        }),
    }
}

pub fn gasless_deposit_status(source: &str, wallet: &str, id: &str) -> DispatchResponse {
    gasless_deposit_status_with_host(&mut BloomHost, source, wallet, id)
}

fn gasless_deposit_status_with_host<H: Host>(
    host: &mut H,
    source: &str,
    wallet: &str,
    id: &str,
) -> DispatchResponse {
    let chain = match source_chain(source) {
        Ok(chain) => chain,
        Err(error) => return error,
    };
    let state = match load(host, chain, wallet, id) {
        Ok(Some(state)) => state,
        Ok(None) => return petal::error(-1, "gasless deposit not found"),
        Err(error) => return error,
    };
    let relay_status = if attempted_submission(&state.phase) {
        match fetch(
            host,
            "GET",
            &format!("/intents/status/v3?requestId={}", state.request_id),
            Vec::new(),
        ) {
            Ok(value) => {
                relay_status_projection(&value).or_else(|| Some(json!({"status": "unavailable"})))
            }
            Err(DispatchResponse::Error { .. }) => Some(json!({"status": "unavailable"})),
            Err(_) => Some(json!({"status": "unavailable"})),
        }
    } else {
        None
    };
    let now_ms = host.now_ms().unwrap_or(0);
    let local_phase = effective_local_phase(&state, now_ms);
    let public_status = public_status(&local_phase, relay_status.as_ref());
    let next = next_action(&state, &public_status);
    petal::read_json_value(&json!({
        "schema": state.schema,
        "source_chain": state.source_chain,
        "source_chain_id": chain.chain_id,
        "wallet": state.wallet,
        "address": state.address,
        "id": state.id,
        "amount": state.amount,
        "status": public_status,
        "request_id": state.request_id,
        "quote": state.quote,
        "approval": state.approval,
        "submission": state.submission,
        "relay": relay_status,
        "next": next
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    const WALLET: &str = "0x03508bb71268bba25ecacc8f620e01866650532c";

    #[derive(Default)]
    struct MockHost {
        store: HashMap<String, Vec<u8>>,
        http_results: VecDeque<Result<HttpResponse, String>>,
        sign_results: VecDeque<Result<SignHashOutcome, String>>,
        requests: Vec<HttpRequest>,
        sign_requests: Vec<SignRequest>,
        now_ms: u64,
    }

    impl MockHost {
        fn push_json(&mut self, status: u16, value: Value) {
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

    fn request() -> GaslessDepositRequest {
        GaslessDepositRequest {
            amount: "100".into(),
            minimum_output: "99".into(),
        }
    }

    fn approval() -> SignHashOutcome {
        SignHashOutcome::ApprovalRequired {
            action_id: "approval-1".into(),
            ceremony_url: "http://127.0.0.1/approve/approval-1".into(),
            expires_ms: 1_500_000,
        }
    }

    fn signature() -> SignHashOutcome {
        let mut bytes = vec![0xab; 65];
        bytes[64] = 0;
        SignHashOutcome::Signature(bytes)
    }

    fn ethereum() -> SourceChain {
        source_chain("ethereum").unwrap()
    }

    fn response(chain: SourceChain) -> Value {
        json!({
            "steps":[{
                "id":"authorize1",
                "kind":"signature",
                "requestId":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "items":[{
                    "data":{
                        "sign":{
                            "signatureKind":"eip712",
                            "types":{"ReceiveWithAuthorization":[
                                {"name":"from","type":"address"},{"name":"to","type":"address"},
                                {"name":"value","type":"uint256"},{"name":"validAfter","type":"uint256"},
                                {"name":"validBefore","type":"uint256"},{"name":"nonce","type":"bytes32"}
                            ]},
                            "domain":{"name":"USD Coin","version":"2","chainId":chain.chain_id,"verifyingContract":chain.usdc},
                            "primaryType":"ReceiveWithAuthorization",
                            "value":{"from":WALLET,"to":chain.permit_receiver,"value":"100000000","validAfter":0,"validBefore":1999999999,"nonce":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
                        },
                        "post":{"endpoint":"/execute/permits","method":"POST","body":{"kind":"eip3009","requestId":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","api":"swap"}}
                    },
                    "check":{"endpoint":"/intents/status/v3?requestId=0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
                }]
            }],
            "details":{
                "recipient":WALLET,
                "currencyIn":{
                    "currency":{"chainId":chain.chain_id,"address":chain.usdc,"decimals":chain.usdc_decimals},
                    "amount":"100000000",
                    "amountFormatted":"100.0"
                },
                "currencyOut":{
                    "currency":{"chainId":1337,"address":HYPERLIQUID_USDC,"decimals":8},
                    "amount":"9980000000",
                    "amountFormatted":"99.8",
                    "minimumAmount":"9930000000"
                },
                "refundCurrency":{
                    "currency":{"chainId":chain.chain_id,"address":chain.usdc,"decimals":chain.usdc_decimals},
                    "amount":"100000000",
                    "amountFormatted":"100.0",
                    "minimumAmount":"100000000"
                },
                "totalImpact":{"percent":"-0.2"},
                "timeEstimate":3
            },
            "protocol":{
                "v2":{
                    "orderData":{
                        "inputs":[{
                            "refunds":[
                                {"chainId":chain.slug,"recipient":WALLET,"currency":chain.usdc,"minimumAmount":"0"},
                                {"chainId":"hyperliquid","recipient":WALLET,"currency":HYPERLIQUID_USDC,"minimumAmount":"0"}
                            ]
                        }]
                    }
                }
            }
        })
    }

    fn prepare_for(chain: SourceChain, response: Value) -> Result<DepositState, DispatchResponse> {
        prepare_quote(
            chain,
            QuoteInput {
                wallet: "minnow-passkey",
                address: WALLET,
                amount: "100",
                units: "100000000",
                minimum_output: "99",
                minimum_output_units: "9900000000",
            },
            response,
        )
    }

    fn prepare(response: Value) -> Result<DepositState, DispatchResponse> {
        prepare_for(ethereum(), response)
    }

    #[test]
    fn accepts_only_the_expected_gasless_route_and_hashes_it() {
        let state = prepare(response(ethereum())).unwrap();
        assert_eq!(state.request_id.len(), 66);
        assert_eq!(state.minimum_output.as_deref(), Some("99"));
        assert_eq!(state.quote["required_minimum_out_units"], "9900000000");
        assert_eq!(
            format!("{:#x}", signing_hash(&state.sign).unwrap()),
            "0xd58cc3c8271c0e82cff651c96d492dd2f0854cecd5e654b8c92bc378184b0fa6"
        );
    }

    #[test]
    fn registry_pins_every_supported_native_usdc_contract() {
        let expected = [
            ("ethereum", 1, "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"),
            ("base", 8453, "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"),
            (
                "arbitrum",
                42161,
                "0xaf88d065e77c8cc2239327c5edb3a432268e5831",
            ),
            ("optimism", 10, "0x0b2c639c533813f4aa9d7837caf62653d097ff85"),
            ("polygon", 137, "0x3c499c542cef5e3811e1192ce70d8cc03d5c3359"),
            (
                "avalanche",
                43114,
                "0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e",
            ),
        ];
        assert_eq!(SOURCE_CHAINS.len(), expected.len());
        for (slug, chain_id, usdc) in expected {
            let chain = source_chain(slug).unwrap();
            assert_eq!(chain.chain_id, chain_id);
            assert_eq!(chain.usdc, usdc);
            assert_eq!(chain.usdc_decimals, 6);
            assert_eq!(chain.permit_receiver, RELAY_PERMIT_RECEIVER);
        }
        assert!(source_chain("Ethereum").is_err());
        assert!(source_chain("solana").is_err());
        assert!(source_chain("../ethereum").is_err());
    }

    #[test]
    fn accepts_a_fully_pinned_quote_for_every_source_chain() {
        for chain in SOURCE_CHAINS {
            let state = prepare_for(chain, response(chain)).unwrap();
            assert_eq!(state.source_chain, chain.slug);
            assert_eq!(state.quote["source_chain"], chain.slug);
            assert_eq!(state.quote["source_chain_id"], chain.chain_id);
            assert_eq!(state.quote["source_currency"], chain.usdc);
            assert!(signing_hash(&state.sign).is_ok());
        }
    }

    #[test]
    fn full_mock_lifecycle_reuses_one_quote_and_reconciles_success() {
        let chain = source_chain("base").unwrap();
        let mut host = MockHost {
            now_ms: 1_000_000,
            ..MockHost::default()
        };
        host.push_json(200, response(chain));
        host.sign_results.push_back(Ok(approval()));

        let first = gasless_deposit_with_host(
            &mut host,
            chain.slug,
            "minnow-passkey".into(),
            WALLET.into(),
            "lifecycle".into(),
            request(),
        );
        assert!(matches!(first, DispatchResponse::Error { code: -2, .. }));
        assert_eq!(host.requests.len(), 1);
        assert_eq!(host.sign_requests.len(), 1);

        let read =
            gasless_deposit_status_with_host(&mut host, chain.slug, "minnow-passkey", "lifecycle");
        let DispatchResponse::Read(bytes) = read else {
            panic!("approval state must be readable");
        };
        let public: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(public["source_chain"], "base");
        assert_eq!(public["status"], "approval_required");

        host.sign_results.push_back(Ok(signature()));
        host.push_json(200, json!({"accepted": true}));
        let retry = gasless_deposit_with_host(
            &mut host,
            chain.slug,
            "minnow-passkey".into(),
            WALLET.into(),
            "lifecycle".into(),
            request(),
        );
        assert_eq!(retry, DispatchResponse::Write);
        assert_eq!(host.sign_requests.len(), 2);
        assert_eq!(
            host.sign_requests[0].hash32, host.sign_requests[1].hash32,
            "approval retry must sign the exact persisted quote"
        );
        assert_eq!(
            host.requests
                .iter()
                .filter(|request| request.url.ends_with("/quote/v2"))
                .count(),
            1,
            "approval retry must not request a second quote"
        );

        host.push_json(
            200,
            json!({
                "status": "success",
                "originChainId": chain.chain_id,
                "destinationChainId": 1337,
                "inTxHashes": ["0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
                "txHashes": ["0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]
            }),
        );
        let settled =
            gasless_deposit_status_with_host(&mut host, chain.slug, "minnow-passkey", "lifecycle");
        let DispatchResponse::Read(bytes) = settled else {
            panic!("submitted operation must be readable");
        };
        let public: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(public["status"], "success");
        assert_eq!(public["next"]["action"], "complete");

        let stored = host
            .store
            .get(&key(chain, "minnow-passkey", "lifecycle"))
            .unwrap();
        let stored_text = String::from_utf8_lossy(stored);
        assert!(!stored_text.contains(&"ab".repeat(64)));
        assert!(!stored_text.contains("signature="));
    }

    #[test]
    fn ambiguous_mock_submission_never_leaks_signature_and_reads_reconcile_it() {
        let chain = source_chain("optimism").unwrap();
        let mut host = MockHost {
            now_ms: 1_000_000,
            ..MockHost::default()
        };
        host.push_json(200, response(chain));
        host.sign_results.push_back(Ok(signature()));
        host.http_results
            .push_back(Err("transport failed at ?signature=0xsecret".into()));

        let result = gasless_deposit_with_host(
            &mut host,
            chain.slug,
            "minnow-passkey".into(),
            WALLET.into(),
            "ambiguous".into(),
            request(),
        );
        assert_eq!(result, backend(SUBMISSION_UNKNOWN));
        let stored = host
            .store
            .get(&key(chain, "minnow-passkey", "ambiguous"))
            .unwrap();
        let stored_text = String::from_utf8_lossy(stored);
        assert!(!stored_text.contains("0xsecret"));
        assert!(!stored_text.contains("transport failed"));

        host.push_json(
            200,
            json!({
                "status": "success",
                "originChainId": chain.chain_id,
                "destinationChainId": 1337
            }),
        );
        let status =
            gasless_deposit_status_with_host(&mut host, chain.slug, "minnow-passkey", "ambiguous");
        let DispatchResponse::Read(bytes) = status else {
            panic!("ambiguous submission must be readable");
        };
        let public: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(public["status"], "success");
    }

    #[test]
    fn same_wallet_and_id_are_isolated_across_mock_source_chains() {
        let base = source_chain("base").unwrap();
        let arbitrum = source_chain("arbitrum").unwrap();
        let mut host = MockHost {
            now_ms: 1_000_000,
            ..MockHost::default()
        };
        host.push_json(200, response(base));
        host.push_json(200, response(arbitrum));
        host.sign_results.push_back(Ok(approval()));
        host.sign_results.push_back(Ok(approval()));

        for chain in [base, arbitrum] {
            let result = gasless_deposit_with_host(
                &mut host,
                chain.slug,
                "minnow-passkey".into(),
                WALLET.into(),
                "same-id".into(),
                request(),
            );
            assert!(matches!(result, DispatchResponse::Error { code: -2, .. }));
        }
        assert!(
            host.store
                .contains_key(&key(base, "minnow-passkey", "same-id"))
        );
        assert!(
            host.store
                .contains_key(&key(arbitrum, "minnow-passkey", "same-id"))
        );
        assert_ne!(
            key(base, "minnow-passkey", "same-id"),
            key(arbitrum, "minnow-passkey", "same-id")
        );
    }

    #[test]
    fn rejects_chain_token_receiver_and_input_metadata_substitution_on_every_chain() {
        for chain in SOURCE_CHAINS {
            let mut wrong_domain_chain = response(chain);
            wrong_domain_chain["steps"][0]["items"][0]["data"]["sign"]["domain"]["chainId"] =
                json!(chain.chain_id + 1);
            assert!(prepare_for(chain, wrong_domain_chain).is_err());

            let mut wrong_contract = response(chain);
            wrong_contract["steps"][0]["items"][0]["data"]["sign"]["domain"]["verifyingContract"] =
                Value::String("0x0000000000000000000000000000000000000002".into());
            assert!(prepare_for(chain, wrong_contract).is_err());

            let mut wrong_receiver = response(chain);
            wrong_receiver["steps"][0]["items"][0]["data"]["sign"]["value"]["to"] =
                Value::String("0x0000000000000000000000000000000000000002".into());
            assert!(prepare_for(chain, wrong_receiver).is_err());

            let mut wrong_detail_chain = response(chain);
            wrong_detail_chain["details"]["currencyIn"]["currency"]["chainId"] =
                json!(chain.chain_id + 1);
            assert!(prepare_for(chain, wrong_detail_chain).is_err());

            let mut wrong_detail_token = response(chain);
            wrong_detail_token["details"]["currencyIn"]["currency"]["address"] =
                Value::String("0x0000000000000000000000000000000000000002".into());
            assert!(prepare_for(chain, wrong_detail_token).is_err());

            let mut wrong_refund_recipient = response(chain);
            wrong_refund_recipient["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"][0]["recipient"] =
                Value::String("0x0000000000000000000000000000000000000002".into());
            assert!(prepare_for(chain, wrong_refund_recipient).is_err());

            let mut extra_refund = response(chain);
            extra_refund["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "chainId": chain.slug,
                    "recipient": WALLET,
                    "currency": chain.usdc
                }));
            assert!(prepare_for(chain, extra_refund).is_err());
        }
    }

    #[test]
    fn durable_keys_are_chain_scoped_and_ethereum_routes_share_legacy_state() {
        let wallet = "minnow-passkey";
        let id = "same-id";
        let ethereum_key = key(ethereum(), wallet, id);
        assert_eq!(
            ethereum_key,
            "state/gasless-deposits/minnow-passkey/same-id.json"
        );
        assert_eq!(
            key(source_chain("base").unwrap(), wallet, id),
            "state/gasless-deposits/by-chain/base/minnow-passkey/same-id.json"
        );
        for left in SOURCE_CHAINS {
            for right in SOURCE_CHAINS {
                if left != right {
                    assert_ne!(key(left, wallet, id), key(right, wallet, id));
                }
            }
        }

        let mut old_value = serde_json::to_value(prepare(response(ethereum())).unwrap()).unwrap();
        old_value.as_object_mut().unwrap().remove("source_chain");
        let old_state: DepositState = serde_json::from_value(old_value).unwrap();
        assert_eq!(old_state.source_chain, "ethereum");
    }

    #[test]
    fn concurrent_initialization_loser_adopts_the_atomically_persisted_winner() {
        let mut losing_quote = prepare(response(ethereum())).unwrap();
        losing_quote.request_id =
            "0x1111111111111111111111111111111111111111111111111111111111111111".into();
        let mut winning_quote = prepare(response(ethereum())).unwrap();
        winning_quote.request_id =
            "0x2222222222222222222222222222222222222222222222222222222222222222".into();

        let selected =
            resolve_initialization_conflict(Some(winning_quote), WALLET, "100000000", "9900000000")
                .unwrap();
        assert_eq!(
            selected.request_id,
            "0x2222222222222222222222222222222222222222222222222222222222222222"
        );
        assert_ne!(selected.request_id, losing_quote.request_id);

        let mut incompatible_winner = selected;
        incompatible_winner.minimum_output_units = Some("9910000000".into());
        assert!(
            resolve_initialization_conflict(
                Some(incompatible_winner),
                WALLET,
                "100000000",
                "9900000000"
            )
            .is_err()
        );
        assert!(resolve_initialization_conflict(None, WALLET, "100000000", "9900000000").is_err());

        let winner = prepare(response(ethereum())).unwrap();
        assert!(
            resolve_initialization_conflict(
                Some(winner),
                "0x0000000000000000000000000000000000000002",
                "100000000",
                "9900000000"
            )
            .is_err(),
            "a wallet alias that resolves to a new address must not reuse old signing state"
        );
    }

    #[test]
    fn rejects_quote_redirection_and_bad_amounts() {
        let mut redirected = response(ethereum());
        redirected["details"]["recipient"] =
            Value::String("0x0000000000000000000000000000000000000002".into());
        assert!(prepare(redirected).is_err());

        let mut injected_request_id = response(ethereum());
        injected_request_id["steps"][0]["requestId"] =
            Value::String("request&signature=0xsecret".into());
        injected_request_id["steps"][0]["items"][0]["data"]["post"]["body"]["requestId"] =
            Value::String("request&signature=0xsecret".into());
        assert!(prepare(injected_request_id).is_err());

        assert!(decimal_units("1.0000001", 6, "amount").is_err());
        assert_eq!(decimal_units("12.5", 6, "amount").unwrap(), "12500000");
        assert_eq!(
            decimal_units("0.934945", HYPERCORE_USDC_DECIMALS, "minimum_output").unwrap(),
            "93494500"
        );
        assert_eq!(format_units(93_027_000, HYPERCORE_USDC_DECIMALS), "0.93027");
    }

    #[test]
    fn rejects_an_untrusted_eip3009_receiver() {
        let mut redirected = response(ethereum());
        redirected["steps"][0]["items"][0]["data"]["sign"]["value"]["to"] =
            Value::String("0x0000000000000000000000000000000000000002".into());
        assert!(prepare(redirected).is_err());
    }

    #[test]
    fn enforces_the_callers_minimum_output_against_relays_slippage_floor() {
        let rejected = prepare_quote(
            ethereum(),
            QuoteInput {
                wallet: "minnow-passkey",
                address: WALLET,
                amount: "100",
                units: "100000000",
                minimum_output: "99.4",
                minimum_output_units: "9940000000",
            },
            response(ethereum()),
        );
        assert!(rejected.is_err());

        let mut inflated_floor = response(ethereum());
        inflated_floor["details"]["currencyOut"]["minimumAmount"] =
            Value::String("9990000000".into());
        inflated_floor["details"]["currencyOut"]["amount"] = Value::String("9980000000".into());
        assert!(prepare(inflated_floor).is_err());
    }

    #[test]
    fn rejects_quote_amount_or_output_precision_changes() {
        let mut changed_input = response(ethereum());
        changed_input["details"]["currencyIn"]["amount"] = Value::String("99999999".into());
        assert!(prepare(changed_input).is_err());

        let mut changed_decimals = response(ethereum());
        changed_decimals["details"]["currencyOut"]["currency"]["decimals"] = json!(6);
        assert!(prepare(changed_decimals).is_err());
    }

    #[test]
    fn rejects_invalid_eip3009_validity_windows() {
        let mut invalid_window = response(ethereum());
        invalid_window["steps"][0]["items"][0]["data"]["sign"]["value"]["validAfter"] =
            json!(2_000_000_000_u64);
        assert!(prepare(invalid_window).is_err());
    }

    #[test]
    fn approval_projection_is_self_documenting_and_retriable() {
        let state = prepare(response(ethereum())).unwrap();
        assert_eq!(
            retry_write_body(&state),
            json!({"amount":"100","minimum_output":"99"})
        );
        let next = next_action(&state, "approval_required");
        assert_eq!(next["action"], "review_quote_then_approve");
        assert_eq!(
            next["retry_write_body"],
            json!({"amount":"100","minimum_output":"99"})
        );
    }

    #[test]
    fn distinguishes_retryable_ceremony_expiry_from_quote_expiry() {
        let mut state = prepare(response(ethereum())).unwrap();
        state.phase = "approval_required".into();
        state.approval = Some(json!({"expires_ms": 2_000_u64}));
        assert_eq!(effective_local_phase(&state, 2_001), "approval_expired");
        assert_eq!(
            next_action(&state, "approval_expired")["action"],
            "retry_write"
        );

        let quote_deadline_ms = (1_999_999_999_u64 - PERMIT_SUBMISSION_MARGIN_SECONDS) * 1_000;
        assert_eq!(
            effective_local_phase(&state, quote_deadline_ms),
            "quote_expired"
        );
        assert_eq!(
            next_action(&state, "quote_expired")["action"],
            "create_new_deposit"
        );

        state.phase = "submitted".into();
        assert_eq!(
            effective_local_phase(&state, quote_deadline_ms),
            "submitted"
        );
    }

    #[test]
    fn legacy_unbounded_quotes_fail_closed_without_blocking_reconciliation() {
        let mut state = prepare(response(ethereum())).unwrap();
        state.minimum_output = None;
        state.minimum_output_units = None;
        assert_eq!(effective_local_phase(&state, 1), "quote_unbounded");
        assert_eq!(
            next_action(&state, "quote_unbounded")["action"],
            "create_new_deposit"
        );

        state.phase = "submission_unknown".into();
        assert_eq!(
            effective_local_phase(&state, 1),
            "submission_unknown",
            "attempted legacy submissions must still reconcile through reads"
        );
    }

    #[test]
    fn reconciles_every_phase_that_may_have_reached_relay() {
        for phase in [
            "submitting",
            "submission_unknown",
            "submission_failed",
            "submitted",
        ] {
            assert!(attempted_submission(phase));
        }
        assert!(!attempted_submission("approval_required"));

        let relay_success = json!({"status": "success"});
        assert_eq!(
            public_status("submission_unknown", Some(&relay_success)),
            "success"
        );
        assert_eq!(
            public_status("submission_failed", Some(&json!({"status": "unavailable"}))),
            "submission_unknown"
        );
    }

    #[test]
    fn projects_only_non_secret_relay_status_fields() {
        let projected = relay_status_projection(&json!({
            "status": "success",
            "signature": "0xsecret",
            "message": "signature=0xsecret",
            "inTxHashes": [
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "not-a-hash"
            ],
            "txHashes": ["0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
            "updatedAt": 123,
            "originChainId": 1,
            "destinationChainId": 1337
        }))
        .unwrap();
        let encoded = serde_json::to_string(&projected).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("signature"));
        assert!(!encoded.contains("not-a-hash"));
    }

    #[test]
    fn permit_submission_errors_are_always_opaque() {
        assert!(!SUBMISSION_UNKNOWN.contains("signature"));
        assert!(!SUBMISSION_UNKNOWN.contains("http"));
        assert!(!SUBMISSION_UNKNOWN.contains("api.relay.link"));
    }
}

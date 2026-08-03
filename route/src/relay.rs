use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use petal::{DispatchResponse, SignHashOutcome, SignRequest};

use crate::common::{
    BloomHost, Host, MAX_BODY, MAX_DECIMALS, PERMIT_SUBMISSION_MARGIN_SECONDS,
    RELAY_PERMIT_RECEIVER, ZERO_ADDRESS, backend, compact, denied, fetch, invalid, is_bytes32,
    is_safe_segment, signature_hex, signing_hash, submit_permit, uint64_value,
};

const SUBMISSION_UNKNOWN: &str =
    "Relay permit submission outcome is unknown; read this transaction to reconcile its status";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PermitDomain {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayOrigin {
    /// Relay's canonical chain slug, used to validate the signed order.
    pub chain: String,
    pub chain_id: u64,
    /// EIP-3009 token contract on the origin EVM chain.
    pub currency: String,
    pub decimals: u8,
    pub permit_domain: PermitDomain,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayDestination {
    /// Relay's canonical chain slug, used to validate the signed order.
    pub chain: String,
    pub chain_id: u64,
    /// Relay currency identifier on the destination chain.
    pub currency: String,
    pub decimals: u8,
    /// Defaults to the resolved Bloom wallet address when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayTransactionRequest {
    pub origin: RelayOrigin,
    pub destination: RelayDestination,
    /// Human-readable exact-input amount, using `origin.decimals`.
    pub amount: String,
    /// Caller-required output floor, using `destination.decimals`.
    pub minimum_output: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct BoundRequest {
    origin: RelayOrigin,
    destination: RelayDestination,
    amount: String,
    minimum_output: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct RelayTransactionState {
    schema: String,
    wallet: String,
    address: String,
    id: String,
    request: BoundRequest,
    amount_units: String,
    minimum_output_units: String,
    request_id: String,
    permit_api: String,
    phase: String,
    sign: Value,
    quote: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission: Option<String>,
}

fn key(wallet: &str, id: &str) -> String {
    format!("state/relay-transactions/{wallet}/{id}.json")
}

fn load<H: Host>(
    host: &mut H,
    wallet: &str,
    id: &str,
) -> Result<Option<RelayTransactionState>, DispatchResponse> {
    match host.store_get(&key(wallet, id), MAX_BODY) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| backend(format!("stored Relay transaction is invalid: {error}"))),
        Ok(None) => Ok(None),
        Err(error) => Err(backend(error)),
    }
}

fn save<H: Host>(host: &mut H, state: &RelayTransactionState) -> Result<(), DispatchResponse> {
    let bytes = serde_json::to_vec(state).map_err(|error| backend(error.to_string()))?;
    host.store_put(&key(&state.wallet, &state.id), &bytes)
        .map_err(backend)
}

fn save_new<H: Host>(
    host: &mut H,
    state: &RelayTransactionState,
) -> Result<bool, DispatchResponse> {
    let bytes = serde_json::to_vec(state).map_err(|error| backend(error.to_string()))?;
    host.store_put_new(&key(&state.wallet, &state.id), &bytes)
        .map_err(backend)
}

fn is_evm_address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalize_evm_address(value: &str, field: &str) -> Result<String, DispatchResponse> {
    if !is_evm_address(value) {
        return Err(invalid(format!("{field} must be an EVM address")));
    }
    Ok(value.to_ascii_lowercase())
}

fn normalize_relay_identifier(value: &str, field: &str) -> Result<String, DispatchResponse> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-'))
    {
        return Err(invalid(format!("{field} is not a safe Relay identifier")));
    }
    if value.starts_with("0x") {
        Ok(value.to_ascii_lowercase())
    } else {
        Ok(value.to_owned())
    }
}

fn normalize_chain(value: &str, field: &str) -> Result<String, DispatchResponse> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(invalid(format!(
            "{field} must be a lowercase Relay chain slug"
        )));
    }
    Ok(value.to_owned())
}

fn validate_domain_part(value: &str, field: &str) -> Result<(), DispatchResponse> {
    // EIP-712 domain names/versions are free-form strings, but we reject
    // characters that could enable injection in downstream JSON processing.
    // Allowed: alphanumeric, space, and common token-name punctuation.
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'-' | b'_' | b'.' | b'(' | b')')
        })
    {
        return Err(invalid(format!("{field} is invalid")));
    }
    Ok(())
}

fn decimal_units(amount: &str, decimals: u8, field: &str) -> Result<String, String> {
    if decimals > MAX_DECIMALS {
        return Err(format!("{field} uses unsupported token precision"));
    }
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > decimals as usize
    {
        return Err(format!(
            "{field} must be a positive decimal with at most {decimals} places"
        ));
    }
    let whole = whole
        .parse::<u128>()
        .map_err(|_| format!("{field} is too large"))?;
    let fraction = if decimals == 0 {
        0
    } else {
        format!("{fraction:0<width$}", width = decimals as usize)
            .parse::<u128>()
            .map_err(|_| format!("invalid {field}"))?
    };
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

fn format_units(units: u128, decimals: u8) -> String {
    let scale = 10_u128.pow(decimals as u32);
    let whole = units / scale;
    let fraction = units % scale;
    if fraction == 0 {
        return whole.to_string();
    }
    format!("{whole}.{fraction:0>width$}", width = decimals as usize)
        .trim_end_matches('0')
        .to_string()
}

fn bind_request(
    request: RelayTransactionRequest,
    wallet_address: &str,
) -> Result<(BoundRequest, String, String), DispatchResponse> {
    if request.origin.chain_id == 0 || request.destination.chain_id == 0 {
        return Err(invalid("origin and destination chain IDs must be positive"));
    }
    if request.origin.decimals > MAX_DECIMALS || request.destination.decimals > MAX_DECIMALS {
        return Err(invalid("token precision is unsupported"));
    }
    validate_domain_part(
        &request.origin.permit_domain.name,
        "origin.permit_domain.name",
    )?;
    validate_domain_part(
        &request.origin.permit_domain.version,
        "origin.permit_domain.version",
    )?;

    let origin_currency = normalize_evm_address(&request.origin.currency, "origin.currency")?;
    if origin_currency == ZERO_ADDRESS {
        return Err(invalid(
            "origin.currency must be an EIP-3009 token contract, not a native currency",
        ));
    }
    let origin = RelayOrigin {
        chain: normalize_chain(&request.origin.chain, "origin.chain")?,
        chain_id: request.origin.chain_id,
        currency: origin_currency,
        decimals: request.origin.decimals,
        permit_domain: request.origin.permit_domain,
    };
    let recipient = match request.destination.recipient {
        Some(recipient) => normalize_relay_identifier(&recipient, "destination.recipient")?,
        None => wallet_address.to_ascii_lowercase(),
    };
    if recipient == ZERO_ADDRESS {
        return Err(invalid("destination.recipient cannot be the zero address"));
    }
    let destination = RelayDestination {
        chain: normalize_chain(&request.destination.chain, "destination.chain")?,
        chain_id: request.destination.chain_id,
        currency: normalize_relay_identifier(
            &request.destination.currency,
            "destination.currency",
        )?,
        decimals: request.destination.decimals,
        recipient: Some(recipient),
    };
    let amount_units =
        decimal_units(&request.amount, origin.decimals, "amount").map_err(invalid)?;
    let minimum_output_units = decimal_units(
        &request.minimum_output,
        destination.decimals,
        "minimum_output",
    )
    .map_err(invalid)?;

    Ok((
        BoundRequest {
            origin,
            destination,
            amount: request.amount,
            minimum_output: request.minimum_output,
        },
        amount_units,
        minimum_output_units,
    ))
}

fn identifier_eq(actual: Option<&str>, expected: &str) -> bool {
    actual.is_some_and(|actual| {
        if expected.starts_with("0x") {
            actual.eq_ignore_ascii_case(expected)
        } else {
            actual == expected
        }
    })
}

#[derive(Clone, Copy)]
struct QuoteInput<'a> {
    wallet: &'a str,
    address: &'a str,
    request: &'a BoundRequest,
    amount_units: &'a str,
    minimum_output_units: &'a str,
}

fn quote<H: Host>(
    host: &mut H,
    input: QuoteInput<'_>,
) -> Result<RelayTransactionState, DispatchResponse> {
    let recipient = input.request.destination.recipient.as_deref().unwrap();
    let body = serde_json::to_vec(&json!({
        "user": input.address,
        "originChainId": input.request.origin.chain_id,
        "destinationChainId": input.request.destination.chain_id,
        "originCurrency": input.request.origin.currency,
        "destinationCurrency": input.request.destination.currency,
        "recipient": recipient,
        "tradeType": "EXACT_INPUT",
        "amount": input.amount_units,
        "refundTo": input.address,
        "usePermit": true,
        // Same-chain swaps otherwise commonly return an onchain transaction
        // step instead of the requested permit flow.
        "forceSolverExecution": true
    }))
    .map_err(|error| backend(error.to_string()))?;
    let response = fetch(host, "POST", "/quote/v2", body)?;
    prepare_quote(input, response)
}

fn prepare_quote(
    input: QuoteInput<'_>,
    response: Value,
) -> Result<RelayTransactionState, DispatchResponse> {
    let step = response
        .get("steps")
        .and_then(Value::as_array)
        .filter(|steps| steps.len() == 1)
        .and_then(|steps| steps.first())
        .filter(|step| step.get("id").and_then(Value::as_str) == Some("authorize1"))
        .ok_or_else(|| {
            backend(format!(
                "Relay did not offer exactly one EIP-3009 gasless authorization: {}",
                compact(&response)
            ))
        })?;
    if step.get("kind").and_then(Value::as_str) != Some("signature") {
        return Err(backend("Relay gasless authorization was not a signature"));
    }
    let item = step
        .get("items")
        .and_then(Value::as_array)
        .filter(|items| items.len() == 1)
        .and_then(|items| items.first())
        .ok_or_else(|| backend("Relay gasless authorization did not contain exactly one item"))?;
    let sign = item
        .pointer("/data/sign")
        .cloned()
        .ok_or_else(|| backend("Relay gasless authorization omitted signing data"))?;
    let request_id = item
        .pointer("/data/post/body/requestId")
        .and_then(Value::as_str)
        .ok_or_else(|| backend("Relay gasless authorization omitted request ID"))?;
    if !is_bytes32(request_id) {
        return Err(backend(
            "Relay gasless authorization returned an invalid request ID",
        ));
    }
    if step.get("requestId").and_then(Value::as_str) != Some(request_id) {
        return Err(backend("Relay returned inconsistent request IDs"));
    }

    let (_, permit_valid_before) = validate_sign(input, &sign)?;
    // Relay's `api` field selects its internal permit routing. Live quotes
    // for USDC transfers return "swap" for both same-chain and cross-chain.
    // "bridge" and "user-swap" are accepted for routing variants Relay may
    // use for other token pairs or execution paths; an unexpected value is
    // rejected by the filter below.
    let permit_api = item
        .pointer("/data/post/body/api")
        .and_then(Value::as_str)
        .filter(|api| matches!(*api, "bridge" | "swap" | "user-swap"))
        .ok_or_else(|| backend("Relay returned an unsupported permit API"))?;
    if item.pointer("/data/post/endpoint").and_then(Value::as_str) != Some("/execute/permits")
        || item.pointer("/data/post/method").and_then(Value::as_str) != Some("POST")
        || item.pointer("/data/post/body/kind").and_then(Value::as_str) != Some("eip3009")
    {
        return Err(backend(
            "Relay returned an unexpected permit submission contract",
        ));
    }
    let expected_check = format!("/intents/status/v3?requestId={request_id}");
    if item.pointer("/check/endpoint").and_then(Value::as_str) != Some(&expected_check)
        || item.pointer("/check/method").and_then(Value::as_str) != Some("GET")
    {
        return Err(backend("Relay returned an unexpected status endpoint"));
    }

    let details = response
        .get("details")
        .ok_or_else(|| backend("Relay quote omitted details"))?;
    validate_quote_details(input, details)?;
    let (amount_out_units, relay_minimum_out_units) = validate_output_floor(input, details)?;
    validate_order(input, &response, amount_out_units, relay_minimum_out_units)?;

    Ok(RelayTransactionState {
        schema: "bloom.gasless.relay-transaction.v1".into(),
        wallet: input.wallet.into(),
        address: input.address.into(),
        id: String::new(),
        request: input.request.clone(),
        amount_units: input.amount_units.into(),
        minimum_output_units: input.minimum_output_units.into(),
        request_id: request_id.into(),
        permit_api: permit_api.into(),
        phase: "awaiting_signature".into(),
        sign,
        quote: json!({
            "amount_in": details.pointer("/currencyIn/amountFormatted"),
            "amount_in_units": input.amount_units,
            "amount_out": details.pointer("/currencyOut/amountFormatted"),
            "amount_out_units": amount_out_units.to_string(),
            "relay_minimum_out_units": relay_minimum_out_units.to_string(),
            "required_minimum_out": input.request.minimum_output,
            "required_minimum_out_units": input.minimum_output_units,
            "total_impact": details.get("totalImpact"),
            "expanded_price_impact": details.get("expandedPriceImpact"),
            "time_estimate_seconds": details.get("timeEstimate"),
            "permit_valid_before_unix_seconds": permit_valid_before,
            "provider": "relay"
        }),
        approval: None,
        submission: None,
    })
}

fn validate_quote_details(input: QuoteInput<'_>, details: &Value) -> Result<(), DispatchResponse> {
    let origin = &input.request.origin;
    let destination = &input.request.destination;
    let recipient = destination.recipient.as_deref().unwrap();
    if !identifier_eq(details.get("sender").and_then(Value::as_str), input.address) {
        return Err(backend("Relay quote changed the sender"));
    }
    if !identifier_eq(details.get("recipient").and_then(Value::as_str), recipient)
        || uint64_value(details.pointer("/currencyIn/currency/chainId")) != Some(origin.chain_id)
        || !identifier_eq(
            details
                .pointer("/currencyIn/currency/address")
                .and_then(Value::as_str),
            &origin.currency,
        )
        || uint64_value(details.pointer("/currencyIn/currency/decimals"))
            != Some(origin.decimals as u64)
        || uint64_value(details.pointer("/currencyOut/currency/chainId"))
            != Some(destination.chain_id)
        || !identifier_eq(
            details
                .pointer("/currencyOut/currency/address")
                .and_then(Value::as_str),
            &destination.currency,
        )
        || uint64_value(details.pointer("/currencyOut/currency/decimals"))
            != Some(destination.decimals as u64)
        || uint64_value(details.pointer("/refundCurrency/currency/chainId"))
            != Some(origin.chain_id)
        || !identifier_eq(
            details
                .pointer("/refundCurrency/currency/address")
                .and_then(Value::as_str),
            &origin.currency,
        )
        || uint64_value(details.pointer("/refundCurrency/currency/decimals"))
            != Some(origin.decimals as u64)
    {
        return Err(backend(
            "Relay quote changed the requested chain, currency, precision, or recipient",
        ));
    }
    if details
        .pointer("/currencyIn/amount")
        .and_then(Value::as_str)
        != Some(input.amount_units)
        || details
            .pointer("/refundCurrency/amount")
            .and_then(Value::as_str)
            != Some(input.amount_units)
    {
        return Err(backend("Relay quote changed the exact input amount"));
    }
    Ok(())
}

fn validate_output_floor(
    input: QuoteInput<'_>,
    details: &Value,
) -> Result<(u128, u128), DispatchResponse> {
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
            "Relay minimum output {} is below required minimum_output {}",
            format_units(relay_minimum_out_units, input.request.destination.decimals),
            input.request.minimum_output,
        )));
    }
    Ok((amount_out_units, relay_minimum_out_units))
}

fn validate_order(
    input: QuoteInput<'_>,
    response: &Value,
    amount_out_units: u128,
    relay_minimum_out_units: u128,
) -> Result<(), DispatchResponse> {
    let order = response
        .pointer("/protocol/v2/orderData")
        .ok_or_else(|| backend("Relay quote omitted signed order data"))?;
    let inputs = order
        .get("inputs")
        .and_then(Value::as_array)
        .filter(|inputs| inputs.len() == 1)
        .ok_or_else(|| backend("Relay order did not contain exactly one input"))?;
    let order_input = &inputs[0];
    if order_input
        .pointer("/payment/chainId")
        .and_then(Value::as_str)
        != Some(&input.request.origin.chain)
        || !identifier_eq(
            order_input
                .pointer("/payment/currency")
                .and_then(Value::as_str),
            &input.request.origin.currency,
        )
        || order_input
            .pointer("/payment/amount")
            .and_then(Value::as_str)
            != Some(input.amount_units)
    {
        return Err(backend("Relay order changed the origin payment"));
    }

    let refunds = order_input
        .get("refunds")
        .and_then(Value::as_array)
        .filter(|refunds| refunds.len() == 2)
        .ok_or_else(|| backend("Relay order did not contain exactly two refund branches"))?;
    let refund_matches = |refund: &Value, chain: &str, currency: &str| {
        refund.get("chainId").and_then(Value::as_str) == Some(chain)
            && identifier_eq(refund.get("currency").and_then(Value::as_str), currency)
            && identifier_eq(
                refund.get("recipient").and_then(Value::as_str),
                input.address,
            )
    };
    if !refunds.iter().any(|refund| {
        refund_matches(
            refund,
            &input.request.origin.chain,
            &input.request.origin.currency,
        )
    }) || !refunds.iter().any(|refund| {
        refund_matches(
            refund,
            &input.request.destination.chain,
            &input.request.destination.currency,
        )
    }) {
        return Err(backend(
            "Relay order changed the refund address, currency, or chain",
        ));
    }

    let output = order
        .get("output")
        .ok_or_else(|| backend("Relay order omitted its output"))?;
    if output.get("chainId").and_then(Value::as_str) != Some(&input.request.destination.chain) {
        return Err(backend("Relay order changed the destination chain"));
    }
    let payments = output
        .get("payments")
        .and_then(Value::as_array)
        .filter(|payments| payments.len() == 1)
        .ok_or_else(|| backend("Relay order did not contain exactly one output payment"))?;
    let payment = &payments[0];
    if !identifier_eq(
        payment.get("recipient").and_then(Value::as_str),
        input.request.destination.recipient.as_deref().unwrap(),
    ) || !identifier_eq(
        payment.get("currency").and_then(Value::as_str),
        &input.request.destination.currency,
    ) || payment.get("minimumAmount").and_then(Value::as_str)
        != Some(&relay_minimum_out_units.to_string())
        || payment.get("expectedAmount").and_then(Value::as_str)
            != Some(&amount_out_units.to_string())
    {
        return Err(backend("Relay order changed the destination payment"));
    }
    if output
        .get("calls")
        .and_then(Value::as_array)
        .is_none_or(|calls| !calls.is_empty())
    {
        return Err(backend(
            "Relay returned destination calls for a transfer-only route",
        ));
    }
    if order
        .get("fees")
        .and_then(Value::as_array)
        .is_none_or(|fees| !fees.is_empty())
    {
        return Err(backend("Relay returned unexpected application fees"));
    }
    if response.pointer("/fees/app").is_some_and(|app_fee| {
        app_fee.get("amount").and_then(Value::as_str) != Some("0")
            || app_fee.get("minimumAmount").and_then(Value::as_str) != Some("0")
    }) {
        return Err(backend("Relay returned an unexpected application fee"));
    }
    Ok(())
}

fn validate_sign(input: QuoteInput<'_>, sign: &Value) -> Result<(u64, u64), DispatchResponse> {
    let primary_type = sign.get("primaryType").and_then(Value::as_str);
    if !matches!(
        primary_type,
        Some("ReceiveWithAuthorization" | "TransferWithAuthorization")
    ) {
        return Err(backend("Relay returned an unsupported EIP-3009 type"));
    }
    let expected_types = json!([
        {"name":"from","type":"address"},
        {"name":"to","type":"address"},
        {"name":"value","type":"uint256"},
        {"name":"validAfter","type":"uint256"},
        {"name":"validBefore","type":"uint256"},
        {"name":"nonce","type":"bytes32"}
    ]);
    let type_path = format!("/types/{}", primary_type.unwrap());
    let valid_after = uint64_value(sign.pointer("/value/validAfter"));
    let valid_before = uint64_value(sign.pointer("/value/validBefore"));
    if sign.get("signatureKind").and_then(Value::as_str) != Some("eip712")
        || sign.pointer("/domain/name").and_then(Value::as_str)
            != Some(&input.request.origin.permit_domain.name)
        || sign.pointer("/domain/version").and_then(Value::as_str)
            != Some(&input.request.origin.permit_domain.version)
        || uint64_value(sign.pointer("/domain/chainId")) != Some(input.request.origin.chain_id)
        || !identifier_eq(
            sign.pointer("/domain/verifyingContract")
                .and_then(Value::as_str),
            &input.request.origin.currency,
        )
        || sign.pointer(&type_path) != Some(&expected_types)
        || !identifier_eq(
            sign.pointer("/value/from").and_then(Value::as_str),
            input.address,
        )
        || !identifier_eq(
            sign.pointer("/value/to").and_then(Value::as_str),
            RELAY_PERMIT_RECEIVER,
        )
        || sign.pointer("/value/value").and_then(Value::as_str) != Some(input.amount_units)
        || sign
            .pointer("/value/nonce")
            .and_then(Value::as_str)
            .is_none_or(|value| !is_bytes32(value))
        || valid_after
            .zip(valid_before)
            .is_none_or(|(after, before)| after >= before)
    {
        return Err(backend("Relay returned unsafe EIP-3009 signing data"));
    }
    Ok((valid_after.unwrap(), valid_before.unwrap()))
}

fn ensure_permit_live<H: Host>(
    host: &mut H,
    state: &RelayTransactionState,
) -> Result<(), DispatchResponse> {
    let input = QuoteInput {
        wallet: &state.wallet,
        address: &state.address,
        request: &state.request,
        amount_units: &state.amount_units,
        minimum_output_units: &state.minimum_output_units,
    };
    let (valid_after, valid_before) = validate_sign(input, &state.sign)?;
    let now_seconds = host
        .now_ms()
        .map_err(|error| backend(format!("cannot check Relay permit expiry: {error}")))?
        / 1_000;
    if now_seconds < valid_after {
        return Err(invalid("Relay quote permit is not valid yet"));
    }
    if now_seconds.saturating_add(PERMIT_SUBMISSION_MARGIN_SECONDS) >= valid_before {
        return Err(invalid(
            "Relay quote permit has expired or is too close to expiry; use a new transaction ID",
        ));
    }
    Ok(())
}

fn retry_write_body(state: &RelayTransactionState) -> Value {
    serde_json::to_value(&state.request).unwrap_or(Value::Null)
}

fn validate_request_constraints(
    state: &RelayTransactionState,
    address: &str,
    request: &BoundRequest,
    amount_units: &str,
    minimum_output_units: &str,
) -> Result<(), DispatchResponse> {
    if state.address != address
        || &state.request != request
        || state.amount_units != amount_units
        || state.minimum_output_units != minimum_output_units
    {
        return Err(invalid(
            "transaction ID already belongs to a different wallet, route, amount, or output constraint",
        ));
    }
    Ok(())
}

pub fn gasless_transaction(
    wallet: String,
    address: String,
    id: String,
    request: RelayTransactionRequest,
) -> DispatchResponse {
    gasless_transaction_with_host(&mut BloomHost, wallet, address, id, request)
}

fn gasless_transaction_with_host<H: Host>(
    host: &mut H,
    wallet: String,
    address: String,
    id: String,
    request: RelayTransactionRequest,
) -> DispatchResponse {
    if !is_safe_segment(&wallet) || !is_safe_segment(&id) {
        return invalid("wallet or id contains invalid characters");
    }
    let (request, amount_units, minimum_output_units) = match bind_request(request, &address) {
        Ok(bound) => bound,
        Err(error) => return error,
    };
    let mut state = match load(host, &wallet, &id) {
        Ok(Some(state)) => {
            if let Err(error) = validate_request_constraints(
                &state,
                &address,
                &request,
                &amount_units,
                &minimum_output_units,
            ) {
                return error;
            }
            if state.phase == "submitted" {
                return DispatchResponse::Write;
            }
            state
        }
        Ok(None) => {
            let input = QuoteInput {
                wallet: &wallet,
                address: &address,
                request: &request,
                amount_units: &amount_units,
                minimum_output_units: &minimum_output_units,
            };
            match quote(host, input) {
                Ok(mut state) => {
                    state.id.clone_from(&id);
                    match save_new(host, &state) {
                        Ok(true) => state,
                        Ok(false) => match load(host, &wallet, &id) {
                            Ok(Some(existing)) => {
                                if let Err(error) = validate_request_constraints(
                                    &existing,
                                    &address,
                                    &request,
                                    &amount_units,
                                    &minimum_output_units,
                                ) {
                                    return error;
                                }
                                existing
                            }
                            Ok(None) => {
                                return backend(
                                    "transaction initialization conflicted but no state was found",
                                );
                            }
                            Err(error) => return error,
                        },
                        Err(error) => return error,
                    }
                }
                Err(error) => return error,
            }
        }
        Err(error) => return error,
    };

    if let Err(error) = ensure_permit_live(host, &state) {
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
        purpose: "gasless.relay".into(),
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
                "review the Relay route and required minimum output, approve it, then retry the exact write: {}",
                compact(state.approval.as_ref().unwrap())
            ));
        }
        Err(error) => return denied(format!("signing denied: {error}")),
    };

    let body = match serde_json::to_vec(&json!({
        "kind": "eip3009",
        "requestId": state.request_id,
        "api": state.permit_api
    })) {
        Ok(body) => body,
        Err(error) => return backend(format!("cannot serialize permit body: {error}")),
    };
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
            if let Err(error) = save(host, &state) {
                return error;
            }
            return backend(SUBMISSION_UNKNOWN);
        }
    }
    state.phase = "submitted".into();
    state.submission = Some("accepted".into());
    if let Err(error) = save(host, &state) {
        return error;
    }
    DispatchResponse::Write
}

fn attempted_submission(phase: &str) -> bool {
    matches!(phase, "submitting" | "submission_unknown" | "submitted")
}

fn relay_status_projection(state: &RelayTransactionState, value: &Value) -> Option<Value> {
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
    let origin_chain_id = uint64_value(value.get("originChainId"));
    let destination_chain_id = uint64_value(value.get("destinationChainId"));
    if origin_chain_id.is_some_and(|chain_id| chain_id != state.request.origin.chain_id)
        || destination_chain_id
            .is_some_and(|chain_id| chain_id != state.request.destination.chain_id)
        || (matches!(status, "success" | "refund" | "failure")
            && (origin_chain_id.is_none() || destination_chain_id.is_none()))
    {
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
        "origin_chain_id": origin_chain_id,
        "destination_chain_id": destination_chain_id
    }))
}

fn public_status(phase: &str, relay_status: Option<&Value>) -> String {
    relay_status
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .filter(|status| *status != "unavailable")
        .map(str::to_owned)
        .unwrap_or_else(|| phase.into())
}

fn effective_local_phase(state: &RelayTransactionState, now_ms: u64) -> String {
    if attempted_submission(&state.phase) || now_ms == 0 {
        return state.phase.clone();
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

fn next_action(state: &RelayTransactionState, status: &str) -> Value {
    match status {
        "approval_required" => json!({
            "action": "review_route_then_approve",
            "instruction": "Review the exact origin, destination, recipient, quote, and minimum output; open approval.ceremony_url; then retry the exact write body.",
            "retry_write_body": retry_write_body(state)
        }),
        "approval_expired" => json!({
            "action": "retry_write",
            "instruction": "The ceremony expired. Retry this exact write body to obtain a fresh ceremony for the same Relay request.",
            "retry_write_body": retry_write_body(state)
        }),
        "quote_expired" => json!({
            "action": "create_new_transaction",
            "instruction": "The Relay permit expired. Use a new transaction ID; this transaction will not silently re-quote."
        }),
        "submitting" | "submission_unknown" | "submitted" | "waiting" | "depositing"
        | "pending" | "delayed" => json!({
            "action": "poll",
            "instruction": "Read this transaction again; only Relay status success means completion."
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

pub fn gasless_transaction_status(wallet: &str, id: &str) -> DispatchResponse {
    gasless_transaction_status_with_host(&mut BloomHost, wallet, id)
}

fn gasless_transaction_status_with_host<H: Host>(
    host: &mut H,
    wallet: &str,
    id: &str,
) -> DispatchResponse {
    if !is_safe_segment(wallet) || !is_safe_segment(id) {
        return invalid("wallet or id contains invalid characters");
    }
    let state = match load(host, wallet, id) {
        Ok(Some(state)) => state,
        Ok(None) => {
            return petal::read_json_value(&json!({
                "schema": "bloom.gasless.relay-transaction.v1",
                "status": "not_created",
                "wallet": wallet,
                "id": id,
                "write": {
                    "origin": {
                        "chain": "Relay chain slug",
                        "chain_id": "positive Relay chain ID",
                        "currency": "EIP-3009 EVM token address",
                        "decimals": "0..38",
                        "permit_domain": {"name": "EIP-712 domain name", "version": "EIP-712 domain version"}
                    },
                    "destination": {
                        "chain": "Relay chain slug",
                        "chain_id": "positive Relay chain ID",
                        "currency": "Relay currency identifier",
                        "decimals": "0..38",
                        "recipient": "optional; defaults to the resolved wallet address"
                    },
                    "amount": "positive origin-token decimal",
                    "minimum_output": "positive destination-token decimal"
                }
            }));
        }
        Err(error) => return error,
    };
    let relay_status = if attempted_submission(&state.phase) {
        match fetch(
            host,
            "GET",
            &format!("/intents/status/v3?requestId={}", state.request_id),
            Vec::new(),
        ) {
            Ok(value) => relay_status_projection(&state, &value).or_else(|| {
                let raw = value.get("status").and_then(Value::as_str);
                Some(match raw {
                    Some(s) => json!({"status": "unavailable", "relay_status_raw": s}),
                    None => json!({"status": "unavailable"}),
                })
            }),
            Err(DispatchResponse::Error { .. }) => Some(json!({"status": "unavailable"})),
            Err(_) => Some(json!({"status": "unavailable"})),
        }
    } else {
        None
    };
    let now_ms = host.now_ms().unwrap_or(0);
    let local_phase = effective_local_phase(&state, now_ms);
    let status = public_status(&local_phase, relay_status.as_ref());
    let next = next_action(&state, &status);
    petal::read_json_value(&json!({
        "schema": state.schema,
        "wallet": state.wallet,
        "address": state.address,
        "id": state.id,
        "request": state.request,
        "status": status,
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
    use crate::common::signing_hash;
    use crate::common::test_helpers::{MockHost, approval, signature};

    const WALLET: &str = "0x03508bb71268bba25ecacc8f620e01866650532c";
    const RECIPIENT: &str = "0x1111111111111111111111111111111111111111";
    const BASE_USDC: &str = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
    const OPTIMISM_USDC: &str = "0x0b2c639c533813f4aa9d7837caf62653d097ff85";

    fn request() -> RelayTransactionRequest {
        RelayTransactionRequest {
            origin: RelayOrigin {
                chain: "base".into(),
                chain_id: 8453,
                currency: BASE_USDC.into(),
                decimals: 6,
                permit_domain: PermitDomain {
                    name: "USD Coin".into(),
                    version: "2".into(),
                },
            },
            destination: RelayDestination {
                chain: "optimism".into(),
                chain_id: 10,
                currency: OPTIMISM_USDC.into(),
                decimals: 6,
                recipient: None,
            },
            amount: "100".into(),
            minimum_output: "97".into(),
        }
    }

    fn bound_request() -> (BoundRequest, String, String) {
        bind_request(request(), WALLET).unwrap()
    }

    fn response() -> Value {
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
                            "domain":{"name":"USD Coin","version":"2","chainId":8453,"verifyingContract":BASE_USDC},
                            "primaryType":"ReceiveWithAuthorization",
                            "value":{"from":WALLET,"to":RELAY_PERMIT_RECEIVER,"value":"100000000","validAfter":0,"validBefore":1999999999,"nonce":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
                        },
                        "post":{"endpoint":"/execute/permits","method":"POST","body":{"kind":"eip3009","requestId":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","api":"swap"}}
                    },
                    "check":{"endpoint":"/intents/status/v3?requestId=0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","method":"GET"}
                }]
            }],
            "details":{
                "sender":WALLET,
                "recipient":WALLET,
                "currencyIn":{
                    "currency":{"chainId":8453,"address":BASE_USDC,"decimals":6},
                    "amount":"100000000",
                    "amountFormatted":"100.0"
                },
                "currencyOut":{
                    "currency":{"chainId":10,"address":OPTIMISM_USDC,"decimals":6},
                    "amount":"98000000",
                    "amountFormatted":"98.0",
                    "minimumAmount":"97500000"
                },
                "refundCurrency":{
                    "currency":{"chainId":8453,"address":BASE_USDC,"decimals":6},
                    "amount":"100000000",
                    "amountFormatted":"100.0",
                    "minimumAmount":"100000000"
                },
                "totalImpact":{"percent":"-2.0"},
                "expandedPriceImpact":{"execution":{"percent":"-1.0"}},
                "timeEstimate":3
            },
            "fees":{
                "app":{"amount":"0","minimumAmount":"0"}
            },
            "protocol":{"v2":{"orderData":{
                "inputs":[{
                    "payment":{"chainId":"base","currency":BASE_USDC,"amount":"100000000"},
                    "refunds":[
                        {"chainId":"base","recipient":WALLET,"currency":BASE_USDC},
                        {"chainId":"optimism","recipient":WALLET,"currency":OPTIMISM_USDC}
                    ]
                }],
                "output":{
                    "chainId":"optimism",
                    "payments":[{
                        "recipient":WALLET,
                        "currency":OPTIMISM_USDC,
                        "minimumAmount":"97500000",
                        "expectedAmount":"98000000"
                    }],
                    "calls":[]
                },
                "fees":[]
            }}}
        })
    }

    fn prepare(value: Value) -> Result<RelayTransactionState, DispatchResponse> {
        let (request, amount_units, minimum_output_units) = bound_request();
        prepare_quote(
            QuoteInput {
                wallet: "minnow-passkey",
                address: WALLET,
                request: &request,
                amount_units: &amount_units,
                minimum_output_units: &minimum_output_units,
            },
            value,
        )
    }

    #[test]
    fn validates_and_hashes_a_generic_relay_route() {
        let state = prepare(response()).unwrap();
        assert_eq!(state.request.origin.chain, "base");
        assert_eq!(state.request.destination.chain, "optimism");
        assert_eq!(state.quote["required_minimum_out_units"], "97000000");
        assert!(signing_hash(&state.sign).is_ok());
    }

    #[test]
    fn supports_both_eip3009_authorization_primary_types() {
        let mut transfer = response();
        let fields =
            transfer["steps"][0]["items"][0]["data"]["sign"]["types"]["ReceiveWithAuthorization"]
                .take();
        transfer["steps"][0]["items"][0]["data"]["sign"]["types"]["TransferWithAuthorization"] =
            fields;
        transfer["steps"][0]["items"][0]["data"]["sign"]["primaryType"] =
            json!("TransferWithAuthorization");
        assert!(prepare(transfer).is_ok());
    }

    #[test]
    fn sends_the_exact_generic_quote_request() {
        let (request, amount_units, minimum_output_units) = bound_request();
        let mut host = MockHost::default();
        host.push_json(200, response());
        quote(
            &mut host,
            QuoteInput {
                wallet: "minnow-passkey",
                address: WALLET,
                request: &request,
                amount_units: &amount_units,
                minimum_output_units: &minimum_output_units,
            },
        )
        .unwrap();
        let sent: Value = serde_json::from_slice(&host.requests[0].body).unwrap();
        assert_eq!(sent["originChainId"], 8453);
        assert_eq!(sent["destinationChainId"], 10);
        assert_eq!(sent["originCurrency"], BASE_USDC);
        assert_eq!(sent["destinationCurrency"], OPTIMISM_USDC);
        assert_eq!(sent["recipient"], WALLET);
        assert_eq!(sent["amount"], "100000000");
        assert_eq!(sent["usePermit"], true);
        assert_eq!(sent["forceSolverExecution"], true);
    }

    #[test]
    fn binds_a_caller_selected_recipient_while_refunds_stay_with_the_wallet() {
        let mut request = request();
        request.destination.recipient = Some(RECIPIENT.into());
        let (request, amount_units, minimum_output_units) = bind_request(request, WALLET).unwrap();
        let mut value = response();
        value["details"]["recipient"] = json!(RECIPIENT);
        value["protocol"]["v2"]["orderData"]["output"]["payments"][0]["recipient"] =
            json!(RECIPIENT);
        let state = prepare_quote(
            QuoteInput {
                wallet: "minnow-passkey",
                address: WALLET,
                request: &request,
                amount_units: &amount_units,
                minimum_output_units: &minimum_output_units,
            },
            value,
        )
        .unwrap();
        assert_eq!(
            state.request.destination.recipient.as_deref(),
            Some(RECIPIENT)
        );
    }

    #[test]
    fn full_lifecycle_reuses_the_quote_and_reconciles_success() {
        let mut host = MockHost {
            now_ms: 1_000_000,
            ..MockHost::default()
        };
        host.push_json(200, response());
        host.sign_results.push_back(Ok(approval()));
        let first = gasless_transaction_with_host(
            &mut host,
            "minnow-passkey".into(),
            WALLET.into(),
            "route-1".into(),
            request(),
        );
        assert!(matches!(first, DispatchResponse::Error { code: -2, .. }));

        host.sign_results.push_back(Ok(signature()));
        host.push_json(200, json!({"accepted":true}));
        let retry = gasless_transaction_with_host(
            &mut host,
            "minnow-passkey".into(),
            WALLET.into(),
            "route-1".into(),
            request(),
        );
        assert_eq!(retry, DispatchResponse::Write);
        assert_eq!(host.sign_requests[0].purpose, "gasless.relay");
        assert_eq!(host.sign_requests[0].hash32, host.sign_requests[1].hash32);
        assert_eq!(
            host.requests
                .iter()
                .filter(|request| request.url.ends_with("/quote/v2"))
                .count(),
            1
        );

        host.push_json(
            200,
            json!({
                "status":"success",
                "originChainId":8453,
                "destinationChainId":10,
                "txHashes":["0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"]
            }),
        );
        let status = gasless_transaction_status_with_host(&mut host, "minnow-passkey", "route-1");
        let DispatchResponse::Read(bytes) = status else {
            panic!("submitted transaction must remain readable");
        };
        let public: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(public["status"], "success");
        assert_eq!(public["next"]["action"], "complete");

        let stored = host.store.get(&key("minnow-passkey", "route-1")).unwrap();
        let stored = String::from_utf8_lossy(stored);
        assert!(!stored.contains(&"ab".repeat(64)));
        assert!(!stored.contains("signature="));
    }

    #[test]
    fn rejects_every_material_quote_or_order_substitution() {
        type QuoteMutation = Box<dyn Fn(&mut Value)>;
        let mutations: Vec<QuoteMutation> = vec![
            Box::new(|value| {
                value["steps"][0]["items"][0]["data"]["sign"]["domain"]["verifyingContract"] =
                    json!("0x0000000000000000000000000000000000000002")
            }),
            Box::new(|value| {
                value["steps"][0]["items"][0]["data"]["sign"]["value"]["to"] =
                    json!("0x0000000000000000000000000000000000000002")
            }),
            Box::new(|value| value["details"]["currencyOut"]["currency"]["chainId"] = json!(42161)),
            Box::new(|value| {
                value["details"]["currencyOut"]["currency"]["address"] =
                    json!("0x0000000000000000000000000000000000000002")
            }),
            Box::new(|value| {
                value["details"]["recipient"] = json!("0x0000000000000000000000000000000000000002")
            }),
            Box::new(|value| {
                value["protocol"]["v2"]["orderData"]["inputs"][0]["payment"]["amount"] =
                    json!("99999999")
            }),
            Box::new(|value| {
                value["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"][1]["recipient"] =
                    json!("0x0000000000000000000000000000000000000002")
            }),
            Box::new(|value| {
                value["protocol"]["v2"]["orderData"]["output"]["payments"][0]["currency"] =
                    json!("0x0000000000000000000000000000000000000002")
            }),
            Box::new(|value| {
                value["protocol"]["v2"]["orderData"]["output"]["calls"] =
                    json!([{"to":"0x0000000000000000000000000000000000000002"}])
            }),
            Box::new(|value| {
                value["protocol"]["v2"]["orderData"]["fees"] = json!([{"amount":"1"}])
            }),
        ];
        for mutate in mutations {
            let mut value = response();
            mutate(&mut value);
            assert!(prepare(value).is_err());
        }
    }

    #[test]
    fn rejects_a_top_level_application_fee_even_when_order_fees_are_empty() {
        let mut value = response();
        value["fees"] = json!({
            "app": {
                "amount": "1",
                "minimumAmount": "1"
            }
        });
        assert!(prepare(value).is_err());
    }

    #[test]
    fn enforces_the_callers_output_floor_and_request_idempotency() {
        let mut below_floor = response();
        below_floor["details"]["currencyOut"]["minimumAmount"] = json!("96999999");
        below_floor["protocol"]["v2"]["orderData"]["output"]["payments"][0]["minimumAmount"] =
            json!("96999999");
        assert!(prepare(below_floor).is_err());

        let mut host = MockHost {
            now_ms: 1_000_000,
            ..MockHost::default()
        };
        host.push_json(200, response());
        host.sign_results.push_back(Ok(approval()));
        let _ = gasless_transaction_with_host(
            &mut host,
            "minnow-passkey".into(),
            WALLET.into(),
            "stable-id".into(),
            request(),
        );
        let mut changed = request();
        changed.destination.chain_id = 42161;
        let reused = gasless_transaction_with_host(
            &mut host,
            "minnow-passkey".into(),
            WALLET.into(),
            "stable-id".into(),
            changed,
        );
        assert!(matches!(reused, DispatchResponse::Error { code: -3, .. }));
        assert_eq!(host.requests.len(), 1);
    }

    #[test]
    fn validates_generic_request_fields_before_http_or_signing() {
        assert!(decimal_units("1.0000001", 6, "amount").is_err());
        assert_eq!(decimal_units("12.5", 6, "amount").unwrap(), "12500000");
        assert_eq!(decimal_units("12", 0, "amount").unwrap(), "12");
        assert_eq!(format_units(93_027_000, 8), "0.93027");

        let mut bad = request();
        bad.origin.currency = "USDC".into();
        assert!(bind_request(bad, WALLET).is_err());
        let mut native = request();
        native.origin.currency = "0x0000000000000000000000000000000000000000".into();
        assert!(bind_request(native, WALLET).is_err());
        let mut zero_recipient = request();
        zero_recipient.destination.recipient =
            Some("0x0000000000000000000000000000000000000000".into());
        assert!(bind_request(zero_recipient, WALLET).is_err());
        let mut bad = request();
        bad.origin.chain = "../base".into();
        assert!(bind_request(bad, WALLET).is_err());
        let mut bad = request();
        bad.destination.currency = "bad currency".into();
        assert!(bind_request(bad, WALLET).is_err());
        let mut bad = request();
        bad.destination.decimals = 39;
        assert!(bind_request(bad, WALLET).is_err());
    }

    #[test]
    fn ambiguous_submission_is_opaque_and_remains_reconcilable() {
        let mut host = MockHost {
            now_ms: 1_000_000,
            ..MockHost::default()
        };
        host.push_json(200, response());
        host.sign_results.push_back(Ok(signature()));
        host.http_results
            .push_back(Err("transport failed at ?signature=0xsecret".into()));
        let result = gasless_transaction_with_host(
            &mut host,
            "minnow-passkey".into(),
            WALLET.into(),
            "ambiguous".into(),
            request(),
        );
        assert_eq!(result, backend(SUBMISSION_UNKNOWN));
        let stored = host.store.get(&key("minnow-passkey", "ambiguous")).unwrap();
        let stored = String::from_utf8_lossy(stored);
        assert!(!stored.contains("secret"));
        assert!(!stored.contains("transport failed"));

        host.push_json(
            200,
            json!({"status":"success","originChainId":8453,"destinationChainId":10}),
        );
        let status = gasless_transaction_status_with_host(&mut host, "minnow-passkey", "ambiguous");
        let DispatchResponse::Read(bytes) = status else {
            panic!("ambiguous submission must be readable");
        };
        let public: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(public["status"], "success");
    }

    #[test]
    fn status_rejects_a_mismatched_relay_operation() {
        let mut state = prepare(response()).unwrap();
        state.id = "status".into();
        state.phase = "submitted".into();
        let projected = relay_status_projection(
            &state,
            &json!({"status":"success","originChainId":1,"destinationChainId":10}),
        );
        assert!(projected.is_none());
    }

    #[test]
    fn status_requires_both_chain_ids_before_reporting_a_terminal_outcome() {
        let mut state = prepare(response()).unwrap();
        state.id = "status".into();
        state.phase = "submitted".into();

        for status in ["success", "refund", "failure"] {
            assert!(
                relay_status_projection(&state, &json!({"status":status,"destinationChainId":10}))
                    .is_none()
            );
            assert!(
                relay_status_projection(&state, &json!({"status":status,"originChainId":8453}))
                    .is_none()
            );
        }
    }

    #[test]
    fn status_accepts_idless_waiting_but_rejects_any_supplied_mismatch() {
        let mut state = prepare(response()).unwrap();
        state.id = "status".into();
        state.phase = "submitted".into();

        assert!(
            relay_status_projection(
                &state,
                &json!({"status":"waiting","quoteCreatedAt":1_785_467_680_898_u64})
            )
            .is_some()
        );
        assert!(
            relay_status_projection(
                &state,
                &json!({"status":"waiting","originChainId":1,"destinationChainId":10})
            )
            .is_none()
        );
    }
}

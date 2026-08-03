#!/usr/bin/env bash
# Live quote-only validation against Relay's public API.
#
# Verifies that Relay's response shape matches what the gasless petal's
# validation logic expects, for both the generic transaction route and
# the legacy Hyperliquid deposit route. Does NOT submit permits or move
# funds — see AGENTS.md for authorization boundaries.
#
# Usage:
#   bash scripts/live-quote-check.sh
#
# Requires: curl, jq
set -euo pipefail

RELAY="https://api.relay.link"
WALLET="0x03508bb71268bba25ecacc8f620e01866650532c"
PERMIT_RECEIVER="0xccc88a9d1b4ed6b0eaba998850414b24f1c315be"
BASE_USDC="0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
OP_USDC="0x0b2c639c533813f4aa9d7837caf62653d097ff85"
HL_USDC="0x00000000000000000000000000000000"

pass=0
fail=0

check() {
  local label="$1" actual="$2" expected="$3"
  if [ "$actual" = "$expected" ]; then
    echo "  ✓ $label"
    pass=$((pass + 1))
  else
    echo "  ✗ $label (got: $actual, expected: $expected)"
    fail=$((fail + 1))
  fi
}

echo "=== Generic route: Base USDC → Optimism USDC ==="
generic=$(curl -s -X POST "$RELAY/quote/v2" \
  -H "Content-Type: application/json" \
  -d "{
    \"user\": \"$WALLET\",
    \"originChainId\": 8453,
    \"destinationChainId\": 10,
    \"originCurrency\": \"$BASE_USDC\",
    \"destinationCurrency\": \"$OP_USDC\",
    \"recipient\": \"$WALLET\",
    \"tradeType\": \"EXACT_INPUT\",
    \"amount\": \"100000000\",
    \"refundTo\": \"$WALLET\",
    \"usePermit\": true,
    \"forceSolverExecution\": true
  }")

check "exactly one step" \
  "$(echo "$generic" | jq -r '.steps | length')" "1"
check "step id is authorize1" \
  "$(echo "$generic" | jq -r '.steps[0].id')" "authorize1"
check "step kind is signature" \
  "$(echo "$generic" | jq -r '.steps[0].kind')" "signature"
check "exactly one item" \
  "$(echo "$generic" | jq -r '.steps[0].items | length')" "1"
check "primaryType is EIP-3009" \
  "$(echo "$generic" | jq -r '.steps[0].items[0].data.sign.primaryType')" "ReceiveWithAuthorization"
check "signatureKind is eip712" \
  "$(echo "$generic" | jq -r '.steps[0].items[0].data.sign.signatureKind')" "eip712"
check "permit receiver is pinned" \
  "$(echo "$generic" | jq -r '.steps[0].items[0].data.sign.value.to')" "$PERMIT_RECEIVER"
check "from is the wallet" \
  "$(echo "$generic" | jq -r '.steps[0].items[0].data.sign.value.from')" "$WALLET"
check "value matches input amount" \
  "$(echo "$generic" | jq -r '.steps[0].items[0].data.sign.value.value')" "100000000"
check "post endpoint is /execute/permits" \
  "$(echo "$generic" | jq -r '.steps[0].items[0].data.post.endpoint')" "/execute/permits"
check "post method is POST" \
  "$(echo "$generic" | jq -r '.steps[0].items[0].data.post.method')" "POST"
check "post body kind is eip3009" \
  "$(echo "$generic" | jq -r '.steps[0].items[0].data.post.body.kind')" "eip3009"
check "check method is GET" \
  "$(echo "$generic" | jq -r '.steps[0].items[0].check.method')" "GET"
check "no destination calls" \
  "$(echo "$generic" | jq -r '.protocol.v2.orderData.output.calls | length')" "0"
check "no order-level fees" \
  "$(echo "$generic" | jq -r '.protocol.v2.orderData.fees | length')" "0"
check "no app fee" \
  "$(echo "$generic" | jq -r '.fees.app.amount')" "0"
check "one input" \
  "$(echo "$generic" | jq -r '.protocol.v2.orderData.inputs | length')" "1"
check "two refunds" \
  "$(echo "$generic" | jq -r '.protocol.v2.orderData.inputs[0].refunds | length')" "2"
check "one output payment" \
  "$(echo "$generic" | jq -r '.protocol.v2.orderData.output.payments | length')" "1"

echo ""
echo "=== Legacy route: Base USDC → Hyperliquid ==="
hl=$(curl -s -X POST "$RELAY/quote/v2" \
  -H "Content-Type: application/json" \
  -d "{
    \"user\": \"$WALLET\",
    \"originChainId\": 8453,
    \"destinationChainId\": 1337,
    \"originCurrency\": \"$BASE_USDC\",
    \"destinationCurrency\": \"$HL_USDC\",
    \"recipient\": \"$WALLET\",
    \"tradeType\": \"EXACT_INPUT\",
    \"amount\": \"100000000\",
    \"refundTo\": \"$WALLET\",
    \"usePermit\": true,
    \"slippageTolerance\": \"50\"
  }")

check "exactly one step" \
  "$(echo "$hl" | jq -r '.steps | length')" "1"
check "step id is authorize1" \
  "$(echo "$hl" | jq -r '.steps[0].id')" "authorize1"
check "step kind is signature" \
  "$(echo "$hl" | jq -r '.steps[0].kind')" "signature"
check "permit receiver is pinned" \
  "$(echo "$hl" | jq -r '.steps[0].items[0].data.sign.value.to')" "$PERMIT_RECEIVER"
check "destination chain is 1337" \
  "$(echo "$hl" | jq -r '.details.currencyOut.currency.chainId')" "1337"
check "destination decimals are 8" \
  "$(echo "$hl" | jq -r '.details.currencyOut.currency.decimals')" "8"
check "two refund branches" \
  "$(echo "$hl" | jq -r '.protocol.v2.orderData.inputs[0].refunds | length')" "2"

echo ""
echo "=== Results: $pass passed, $fail failed ==="
exit $fail

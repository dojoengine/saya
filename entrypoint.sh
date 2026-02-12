#!/usr/bin/env bash
set -euo pipefail
set -x

: "${PRIVATE_KEY_KATANA0:?}"
: "${ADDRESS_KATANA0:?}"
: "${SETTLEMENT_RPC_URL:?}"
: "${SALT:?}"
: "${CHAIN_ID:?}"
: "${OUT_JSON:=/shared/out.json}"

curl -sS "$SETTLEMENT_RPC_URL" >/dev/null || true
asdf set sozo 1.8.6

DECLARE_OUT=$(sozo declare /app/contracts/piltover_mock.json --katana-account katana0 --rpc-url "$SETTLEMENT_RPC_URL" \
  --l1-gas 500000 --l1-gas-price 20000000000 \
  --l1-data-gas 50000 --l1-data-gas-price 1000000 \
  --l2-gas 20000000 --l2-gas-price 20000000000 \
  2>&1 | tee /dev/stderr)

CLASS_HASH=$(echo "$DECLARE_OUT" | grep -oE '0x[0-9a-f]{64}' | head -n1 || true)
test -n "$CLASS_HASH"

DEPLOY_OUT=$(sozo deploy "$CLASS_HASH" --katana-account katana0 --rpc-url "$SETTLEMENT_RPC_URL" 2>&1 | tee /dev/stderr)
FACT_REGISTRY_ADDRESS=$(echo "$DEPLOY_OUT" | grep -E 'Address' | awk '{print $3}' || true)
test -n "$FACT_REGISTRY_ADDRESS"

RUST_LOG=info saya core-contract \
  --private-key "$PRIVATE_KEY_KATANA0" \
  --account-address "$ADDRESS_KATANA0" \
  --settlement-chain-id custom \
  --settlement-rpc-url "$SETTLEMENT_RPC_URL" \
  declare

OUTPUT=$(RUST_LOG=info saya core-contract \
  --private-key "$PRIVATE_KEY_KATANA0" \
  --account-address "$ADDRESS_KATANA0" \
  --settlement-chain-id custom \
  --settlement-rpc-url "$SETTLEMENT_RPC_URL" \
  deploy --salt "$SALT" 2>&1 | tee /dev/stderr)

CORE_CONTRACT_ADDRESS=$(echo "$OUTPUT" | grep 'Core contract address' | awk '{print $NF}' || true)
BLOCK_NUMBER=$(echo "$OUTPUT" | grep 'Deployed on block' | awk '{print $NF}' || true)
test -n "$CORE_CONTRACT_ADDRESS"
test -n "$BLOCK_NUMBER"

saya core-contract \
  --private-key "$PRIVATE_KEY_KATANA0" \
  --account-address "$ADDRESS_KATANA0" \
  --settlement-chain-id custom \
  --settlement-rpc-url "$SETTLEMENT_RPC_URL" \
  setup-program \
  --chain-id custom \
  --core-contract-address "$CORE_CONTRACT_ADDRESS" \
  --fact-registry-address "$FACT_REGISTRY_ADDRESS"

mkdir -p "$(dirname "$OUT_JSON")"

printf '%s\n%s\n%s\n' "$CORE_CONTRACT_ADDRESS" "$BLOCK_NUMBER" "$FACT_REGISTRY_ADDRESS" > "$OUT_JSON"

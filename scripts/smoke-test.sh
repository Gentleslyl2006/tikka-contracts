#!/bin/bash
set -euo pipefail

if [ -f .env ]; then
  export $(grep -v '^\s*#' .env | xargs)
fi

: "${TESTNET_SECRET_KEY:?TESTNET_SECRET_KEY is required}"
NETWORK="testnet"

echo "=== Tikka Testnet Smoke Test ==="
echo ""

PUBLIC_KEY=$(stellar keys address <(echo "$TESTNET_SECRET_KEY") 2>/dev/null || \
  stellar keys address 2>/dev/null <<<"$TESTNET_SECRET_KEY" || true)
echo "Deployer public key: ${PUBLIC_KEY:-unknown}"
echo ""

WASM_DIR="target/wasm32-unknown-unknown/release"
FACTORY_WASM="$WASM_DIR/raffle_factory.wasm"
INSTANCE_WASM="$WASM_DIR/raffle_instance.wasm"

if [ ! -f "$FACTORY_WASM" ] || [ ! -f "$INSTANCE_WASM" ]; then
  echo "Building contracts..."
  cargo build --target wasm32-unknown-unknown --release
  echo ""
fi

echo "Funding deployer account..."
curl -sS "https://friendbot.stellar.org?addr=$PUBLIC_KEY" 2>/dev/null || true
echo ""
echo ""

echo "--- Step 1/6: Deploy Factory ---"
INSTANCE_HASH=$(stellar contract install \
  --wasm "$INSTANCE_WASM" \
  --source "$TESTNET_SECRET_KEY" \
  --network "$NETWORK")
echo "Instance WASM hash: $INSTANCE_HASH"

FACTORY_ID=$(stellar contract deploy \
  --wasm "$FACTORY_WASM" \
  --source "$TESTNET_SECRET_KEY" \
  --network "$NETWORK")
echo "Factory contract ID: $FACTORY_ID"

echo "Initializing factory..."
stellar contract invoke \
  --id "$FACTORY_ID" \
  --source "$TESTNET_SECRET_KEY" \
  --network "$NETWORK" \
  -- \
  init_factory \
  --admin "$PUBLIC_KEY" \
  --wasm_hash "$INSTANCE_HASH" \
  --protocol_fee_bp 0 \
  --treasury "$PUBLIC_KEY"
echo "Factory initialized successfully"
echo ""

echo "--- Step 2/6: Create Raffle ---"
NATIVE_ID=$(stellar util contract id native 2>/dev/null || \
  stellar utils contract id native 2>/dev/null || \
  echo "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2Q2JXX4B5ATQFBPK")
echo "Native asset contract ID: $NATIVE_ID"

METADATA_HASH="0101010101010101010101010101010101010101010101010101010101010101"

RAFFLE_CONFIG=$(cat <<EOF
{
  "description": "Smoke Test Raffle",
  "end_time": 0,
  "no_deadline": true,
  "max_tickets": 10,
  "max_tickets_per_tx": 10,
  "min_tickets": 1,
  "allow_multiple": true,
  "ticket_price": 10000000,
  "payment_token": "$NATIVE_ID",
  "prize_amount": 10000000,
  "prizes": [10000],
  "randomness_source": "Internal",
  "oracle_address": null,
  "protocol_fee_bp": 0,
  "treasury_address": null,
  "swap_router": null,
  "tikka_token": null,
  "metadata_hash": "$METADATA_HASH",
  "claim_lockup_seconds": 1,
  "swap_deadline_seconds": 0,
  "early_bird_ticket_percentage": 0,
  "early_bird_discount_bp": 0,
  "category": null
}
EOF
)

RAFFLE_ID=$(stellar contract invoke \
  --id "$FACTORY_ID" \
  --source "$TESTNET_SECRET_KEY" \
  --network "$NETWORK" \
  -- \
  create_raffle \
  --creator "$PUBLIC_KEY" \
  --config "$RAFFLE_CONFIG")
echo "Raffle instance ID: $RAFFLE_ID"
echo ""

echo "--- Step 3/6: Buy 1 Ticket ---"
echo "Depositing prize..."
stellar contract invoke \
  --id "$RAFFLE_ID" \
  --source "$TESTNET_SECRET_KEY" \
  --network "$NETWORK" \
  -- \
  deposit_prize
echo "Prize deposited, raffle is now Active"

echo "Buying 1 ticket..."
stellar contract invoke \
  --id "$RAFFLE_ID" \
  --source "$TESTNET_SECRET_KEY" \
  --network "$NETWORK" \
  -- \
  buy_tickets \
  --buyer "$PUBLIC_KEY" \
  --quantity 1
echo "Ticket purchased"
echo ""

echo "--- Step 4/6: Verify Raffle State ---"
RAFFLE_STATE=$(stellar contract invoke \
  --id "$RAFFLE_ID" \
  --source "$TESTNET_SECRET_KEY" \
  --network "$NETWORK" \
  -- \
  get_raffle)
echo "$RAFFLE_STATE"
echo ""

echo "--- Step 5/6: Finalize Raffle ---"
stellar contract invoke \
  --id "$RAFFLE_ID" \
  --source "$TESTNET_SECRET_KEY" \
  --network "$NETWORK" \
  -- \
  finalize_raffle
echo "Raffle finalized"
echo ""

echo "--- Step 6/6: Claim Prize ---"
stellar contract invoke \
  --id "$RAFFLE_ID" \
  --source "$TESTNET_SECRET_KEY" \
  --network "$NETWORK" \
  -- \
  claim_prize \
  --winner "$PUBLIC_KEY" \
  --tier_index 0
echo "Prize claimed"
echo ""

echo "=== Smoke Test PASSED ==="
echo "Factory: $FACTORY_ID"
echo "Raffle:  $RAFFLE_ID"

#!/usr/bin/env bash
# Deploy Agent Vault to NEAR testnet.
#
# Prerequisites:
#   - Rust + wasm32-unknown-unknown target (see README.md)
#   - `near` CLI (near-cli-rs >= 0.30)
#   - .env with OWNER_ACCOUNT, AGENT_ACCOUNT, VAULT_ACCOUNT (see .env.example)
#   - OWNER_ACCOUNT and AGENT_ACCOUNT already exist on testnet. Create one
#     with the testnet faucet if needed:
#       near account create-account sponsor-by-faucet-service <name>.testnet \
#         autogenerate-new-keypair save-to-keychain network-config testnet create
#
# Usage: ./scripts/deploy.sh
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -f .env ]; then
  # shellcheck disable=SC1091
  set -a; . ./.env; set +a
fi

OWNER_ACCOUNT="${OWNER_ACCOUNT:?set OWNER_ACCOUNT in .env}"
AGENT_ACCOUNT="${AGENT_ACCOUNT:?set AGENT_ACCOUNT in .env}"
VAULT_ACCOUNT="${VAULT_ACCOUNT:?set VAULT_ACCOUNT in .env}"
DAILY_LIMIT_NEAR="${DAILY_LIMIT_NEAR:-2}"
APPROVAL_THRESHOLD_NEAR="${APPROVAL_THRESHOLD_NEAR:-1}"
# Testnet faucet funds 10 NEAR per account. Budget: vault 5 (wasm storage),
# agent 0.5 (gas only), the rest stays with the owner for the deposit + gas.
VAULT_FUNDING_NEAR="${VAULT_FUNDING_NEAR:-5}"
AGENT_FUNDING_NEAR="${AGENT_FUNDING_NEAR:-0.5}"

to_yocto() {
  python3 -c "from decimal import Decimal; print(int(Decimal('$1') * 10**24))"
}
DAILY_LIMIT_YOCTO="$(to_yocto "$DAILY_LIMIT_NEAR")"
THRESHOLD_YOCTO="$(to_yocto "$APPROVAL_THRESHOLD_NEAR")"
# cargo-near (required by near-sdk 5) writes the wasm here:
WASM="contract/target/near/agent_vault.wasm"

echo "==> Building contract (cargo test first)"
( cd contract && cargo test --quiet )
echo "==> Building wasm (cargo near build)"
( cd contract && cargo near build non-reproducible-wasm )
[ -f "$WASM" ] || { echo "wasm not found at $WASM"; exit 1; }
ls -la "$WASM"

echo "==> Creating vault sub-account $VAULT_ACCOUNT (funded by $OWNER_ACCOUNT)"
near account create-account fund-myself "$VAULT_ACCOUNT" "$VAULT_FUNDING_NEAR NEAR" \
  autogenerate-new-keypair save-to-keychain \
  sign-as "$OWNER_ACCOUNT" network-config testnet sign-with-keychain send \
  || echo "(vault account may already exist — continuing)"

echo "==> Creating agent sub-account $AGENT_ACCOUNT (funded by $OWNER_ACCOUNT)"
near account create-account fund-myself "$AGENT_ACCOUNT" "$AGENT_FUNDING_NEAR NEAR" \
  autogenerate-new-keypair save-to-keychain \
  sign-as "$OWNER_ACCOUNT" network-config testnet sign-with-keychain send \
  || echo "(agent account may already exist — continuing)"

# Record the agent's full-access public key for the demo lockdown step
# (demo.sh deletes this key after adding the scoped function-call key).
mkdir -p neardev
python3 -c "import json;print(json.load(open('$HOME/.near-credentials/testnet/${AGENT_ACCOUNT}.json'))['public_key'])" \
  > neardev/agent-full-access-pubkey.txt 2>/dev/null \
  && echo "agent full-access key recorded in neardev/agent-full-access-pubkey.txt" \
  || echo "(could not record agent public key — demo.sh will print manual steps)"

echo "==> Deploying to $VAULT_ACCOUNT"
near contract deploy "$VAULT_ACCOUNT" use-file "$WASM" \
  with-init-call new \
  json-args "{\"owner\":\"$OWNER_ACCOUNT\",\"agent\":\"$AGENT_ACCOUNT\",\"daily_limit\":\"$DAILY_LIMIT_YOCTO\",\"approval_threshold\":\"$THRESHOLD_YOCTO\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  network-config testnet sign-with-keychain send

echo "==> Verifying deployment"
near contract call-function as-read-only "$VAULT_ACCOUNT" get_policy \
  json-args '{}' network-config testnet now

echo
echo "Deployed: $VAULT_ACCOUNT"
echo "Explorer: https://testnet.nearblocks.io/address/$VAULT_ACCOUNT"

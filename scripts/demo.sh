#!/usr/bin/env bash
# Agent Vault attack demo on NEAR testnet.
#
# Story: an AI agent holds money in the vault. The agent account keeps ONLY
# a function-call key scoped to `request_spend` on the vault — it never holds
# a full-access key. A prompt-injection attack tries to make it pay an
# attacker. The vault's policy stops it on-chain.
#
# Acts:
#   1. SETUP: owner allowlists api-provider.testnet and funds the vault (4 NEAR)
#   2. LOCKDOWN: owner gives the agent a function-call key (request_spend only,
#      0.25 NEAR gas allowance), then deletes the agent's full-access key.
#      From here the agent can ONLY call request_spend on this vault.
#   3. ATTACK: agent tries to pay attacker.testnet (not allowlisted) -> BLOCKED
#   4. ATTACK: agent tries to pay 50 NEAR (over the 2 NEAR daily limit) -> BLOCKED
#   5. OK:    agent pays 0.5 NEAR to api-provider.testnet -> EXECUTES
#   6. LARGE: agent requests 1.5 NEAR (>= 1 NEAR threshold) -> PENDING, not executed
#   7. OK:    owner approves the pending spend -> EXECUTES
#
# Prerequisites: a deployed vault (./scripts/deploy.sh) and .env set.
# Usage: ./scripts/demo.sh 2>&1 | tee demo.log
set -uo pipefail
cd "$(dirname "$0")/.."

if [ -f .env ]; then
  # shellcheck disable=SC1091
  set -a; . ./.env; set +a
fi

OWNER_ACCOUNT="${OWNER_ACCOUNT:?set OWNER_ACCOUNT in .env}"
AGENT_ACCOUNT="${AGENT_ACCOUNT:?set AGENT_ACCOUNT in .env}"
VAULT_ACCOUNT="${VAULT_ACCOUNT:?set VAULT_ACCOUNT in .env}"
ATTACKER="${ATTACKER_ACCOUNT:-attacker.testnet}"
RECIPIENT="${RECIPIENT_ACCOUNT:-api-provider.testnet}"

TX="near contract call-function as-transaction"
RO="near contract call-function as-read-only"

say() { echo; echo "===== $1 ====="; }
expect_fail() {
  # Runs a command that MUST fail; prints its error and continues.
  local desc="$1"; shift
  echo "--- $desc (expecting rejection) ---"
  if "$@" > /tmp/vault-demo-out.txt 2>&1; then
    echo "!! UNEXPECTED SUCCESS — the policy did NOT stop this:"
    tail -5 /tmp/vault-demo-out.txt
  else
    echo "BLOCKED as expected. Error:"
    grep -iE 'error|panic|rejected|not allowlisted|exceeds|Smart contract panicked' /tmp/vault-demo-out.txt | head -5
  fi
}

say "0. Current policy"
$RO "$VAULT_ACCOUNT" get_policy json-args '{}' network-config testnet now

say "1. SETUP — owner allowlists $RECIPIENT and funds the vault with 4 NEAR"
$TX "$VAULT_ACCOUNT" add_recipient \
  json-args "{\"account\":\"$RECIPIENT\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$OWNER_ACCOUNT" network-config testnet sign-with-keychain send
$TX "$VAULT_ACCOUNT" deposit \
  json-args '{}' \
  prepaid-gas '100.0 Tgas' attached-deposit '4 NEAR' \
  sign-as "$OWNER_ACCOUNT" network-config testnet sign-with-keychain send

say "2. LOCKDOWN — agent gets a scoped function-call key; full-access key deleted"
if [ -f neardev/lockdown-done.txt ]; then
  echo "(lockdown already done for this deployment — skipping)"
else
echo "--- adding function-call key: request_spend on $VAULT_ACCOUNT only, 0.25 NEAR allowance ---"
near account add-key "$AGENT_ACCOUNT" grant-function-call-access \
  --allowance '0.25 NEAR' \
  --contract-account-id "$VAULT_ACCOUNT" \
  --function-names 'request_spend' \
  autogenerate-new-keypair save-to-keychain \
  network-config testnet sign-with-keychain send
echo "--- deleting the agent's full-access key (owner signs as the agent account) ---"
PUBKEY_FILE="neardev/agent-full-access-pubkey.txt"
if [ -f "$PUBKEY_FILE" ]; then
  AGENT_FULL_KEY="$(cat "$PUBKEY_FILE")"
  echo "full-access key to delete: $AGENT_FULL_KEY"
  near account delete-keys "$AGENT_ACCOUNT" public-keys \
    "$AGENT_FULL_KEY" \
    network-config testnet sign-with-keychain send \
    || echo "(key deletion failed — check $PUBKEY_FILE and delete it manually: near account delete-keys $AGENT_ACCOUNT <key>)"
else
  echo "(no recorded full-access key at $PUBKEY_FILE — delete it manually:)"
  echo "  near account list-keys $AGENT_ACCOUNT network-config testnet now"
  echo "  near account delete-keys $AGENT_ACCOUNT <full-access-public-key>"
fi
echo "--- pointing the agent keychain file at the new scoped key ---"
echo "    (the CLI's legacy keychain only reads <account>.json, not the per-key files)"
NEW_KEY_FILE="$(ls -t "$HOME/.near-credentials/testnet/${AGENT_ACCOUNT}/ed25519_"*.json 2>/dev/null | head -1)"
if [ -n "$NEW_KEY_FILE" ]; then
  cp "$NEW_KEY_FILE" "$HOME/.near-credentials/testnet/${AGENT_ACCOUNT}.json"
  echo "agent keychain now -> $(python3 -c "import json;print(json.load(open('$HOME/.near-credentials/testnet/${AGENT_ACCOUNT}.json'))['public_key'])")"
else
  echo "(no per-key file found — copy the new scoped key to ~/.near-credentials/testnet/${AGENT_ACCOUNT}.json manually)"
fi
echo "--- remaining keys on $AGENT_ACCOUNT (should be the scoped key only) ---"
near account list-keys "$AGENT_ACCOUNT" network-config testnet now
touch neardev/lockdown-done.txt
fi

say "3. ATTACK — prompt-injected agent pays $ATTACKER (not allowlisted)"
expect_fail "pay attacker" $TX "$VAULT_ACCOUNT" request_spend \
  json-args "{\"to\":\"$ATTACKER\",\"amount\":\"500000000000000000000000\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$AGENT_ACCOUNT" network-config testnet sign-with-keychain send

say "4. ATTACK — agent pays 50 NEAR to $RECIPIENT (over the 2 NEAR daily limit)"
expect_fail "overspend" $TX "$VAULT_ACCOUNT" request_spend \
  json-args "{\"to\":\"$RECIPIENT\",\"amount\":\"50000000000000000000000000\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$AGENT_ACCOUNT" network-config testnet sign-with-keychain send

say "5. OK — agent pays 0.5 NEAR to $RECIPIENT (within policy)"
$TX "$VAULT_ACCOUNT" request_spend \
  json-args "{\"to\":\"$RECIPIENT\",\"amount\":\"500000000000000000000000\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$AGENT_ACCOUNT" network-config testnet sign-with-keychain send

say "6. LARGE — agent requests 1.5 NEAR (>= 1 NEAR threshold): goes PENDING"
$TX "$VAULT_ACCOUNT" request_spend \
  json-args "{\"to\":\"$RECIPIENT\",\"amount\":\"1500000000000000000000000\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$AGENT_ACCOUNT" network-config testnet sign-with-keychain send
echo "--- pending spends ---"
$RO "$VAULT_ACCOUNT" get_pending json-args '{}' network-config testnet now

say "7. OK — owner approves pending spend #1"
$TX "$VAULT_ACCOUNT" approve_spend \
  json-args '{"spend_id":1}' \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$OWNER_ACCOUNT" network-config testnet sign-with-keychain send

say "8. Final policy"
$RO "$VAULT_ACCOUNT" get_policy json-args '{}' network-config testnet now

echo
echo "Done. Vault: $VAULT_ACCOUNT"
echo "Explorer: https://testnet.nearblocks.io/address/$VAULT_ACCOUNT"

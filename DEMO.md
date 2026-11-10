# Agent Vault — demo (NEAR testnet, 10 Nov 2026)

Live contract: **`vault2.agentvault-owner.testnet`**
Explorer: https://testnet.nearblocks.io/address/vault2.agentvault-owner.testnet

Policy for this demo: daily limit **2 NEAR**, approval threshold **1 NEAR**,
funded with **4 NEAR**. Owner `agentvault-owner.testnet`,
agent `agent2.agentvault-owner.testnet`, allowlist `api-provider.testnet`.

Every transaction below was verified on-chain via the NEAR testnet RPC
(`tx` status) on 10 Nov 2026. Links use the NEAR testnet explorer.

## The story

An AI agent needs to pay for an API, but its owner does not trust it with
an open wallet. The agent's key can only call `request_spend` on this
vault — and the vault checks every spend against the owner's policy before
any NEAR moves.

## Setup

| Step | Transaction |
|---|---|
| Deploy vault2 | [EESf9utde6SemJXfDKMR6ZMJzY3D2UUnDgDsDX5RpWnC](https://explorer.testnet.near.org/transactions/EESf9utde6SemJXfDKMR6ZMJzY3D2UUnDgDsDX5RpWnC) |
| Allowlist `api-provider.testnet` | [BgKT4WgmhxtQMsfNtBNVVNGXZwa7QWPx5z4yZhAF8pjz](https://explorer.testnet.near.org/transactions/BgKT4WgmhxtQMsfNtBNVVNGXZwa7QWPx5z4yZhAF8pjz) |
| Deposit 4 NEAR | [HotYs92T5eyLtHHEwStXjnf8BmjxSmrBX9VUHArgEm4o](https://explorer.testnet.near.org/transactions/HotYs92T5eyLtHHEwStXjnf8BmjxSmrBX9VUHArgEm4o) |

**Lockdown.** The agent account `agent2` holds exactly one key: a
function-call key scoped to `request_spend` on this vault only, with a
0.25 NEAR gas allowance. Its full-access key was deleted
(`AXQCKwoy5hw9UAgCM5zWoUsQ8WZYAG7UBTeWGbxwYAG4`). The agent cannot
transfer NEAR, add keys, or call any other contract — it can only ask the
vault to spend.

## Attack 1 — pay an address that is not allowlisted: BLOCKED

The agent tries to pay `attacker.testnet` 1 NEAR.

- Tx: [xQpiiZWNxYpBfFAgF32AJ66iwD6cikUMLScSRdbpx3Q](https://explorer.testnet.near.org/transactions/xQpiiZWNxYpBfFAgF32AJ66iwD6cikUMLScSRdbpx3Q)
- Result: **transaction failed** — `Smart contract panicked: recipient not
  allowlisted: attacker.testnet`. No funds moved.

## Attack 2 — pay 50 NEAR, over the 2 NEAR daily limit: BLOCKED

The agent tries to pay `api-provider.testnet` 50 NEAR.

- Tx: [7VjTQYA9wqXLx1gwAmPD1mU2GpT3hFNPzrFpZFc3RsAw](https://explorer.testnet.near.org/transactions/7VjTQYA9wqXLx1gwAmPD1mU2GpT3hFNPzrFpZFc3RsAw)
- Result: **transaction failed** — `Smart contract panicked: amount exceeds
  daily limit`. No funds moved.

## Good spend — 0.5 NEAR to an allowlisted recipient: EXECUTED

- Tx: [BXJ9hS8UQAd2cNRuPQsqi22JFAJtG79FQMoPrnCaRMxk](https://explorer.testnet.near.org/transactions/BXJ9hS8UQAd2cNRuPQsqi22JFAJtG79FQMoPrnCaRMxk)
- Result: **success** (`spend_executed` event). 0.5 NEAR moved,
  `spent_today` 0.5 NEAR.

## Large spend — 1.5 NEAR (above the 1 NEAR threshold): PENDING, then APPROVED

The agent requests 1.5 NEAR. It does **not** execute — it becomes pending
spend #1.

- Request tx: [AjptgSFpTdcQwf3ouQQpc9qMLK2R4CuXmeAv5vig8bKT](https://explorer.testnet.near.org/transactions/AjptgSFpTdcQwf3ouQQpc9qMLK2R4CuXmeAv5vig8bKT) (`spend_requested`)
- Owner approval tx: [9qS2DdYBULZ3cC8DJ4xrnkjU74i335qMofXrtYH28j87](https://explorer.testnet.near.org/transactions/9qS2DdYBULZ3cC8DJ4xrnkjU74i335qMofXrtYH28j87) (`spend_approved` + `spend_executed`)
- Result: 1.5 NEAR moved after the owner's explicit approval.

## Final state (verified via `get_policy`)

```json
{
  "owner": "agentvault-owner.testnet",
  "agent": "agent2.agentvault-owner.testnet",
  "allowlist": ["api-provider.testnet"],
  "daily_limit": "2000000000000000000000000",
  "approval_threshold": "1000000000000000000000000",
  "balance": "6998816462279454500000000",
  "spent_today": "2000000000000000000000000",
  "total_spent": "2000000000000000000000000",
  "pending_count": 0
}
```

Vault balance ~6.9988 NEAR (4 deposited + ~5 funded − 2 spent; the vault
account also holds its creation balance). `api-provider.testnet` holds
12 NEAR (10 from the testnet faucet + 2 received from the vault) — the
money actually moved.

## Reproduce it yourself

```sh
cp .env.example .env   # fill in your own testnet accounts
./scripts/deploy.sh    # build, create vault account, deploy, init
./scripts/demo.sh      # runs the full story above against your vault
```

## Note

The first demo vault (`vault.agentvault-owner.testnet`) recorded 2 NEAR of
spends to a recipient that did not exist yet on testnet; those transfers
failed with `AccountDoesNotExist` and the NEAR was refunded, but the spends
stayed recorded. Recipients must be existing NEAR accounts — verify before
allowlisting. The vault2 demo above used the now-existing recipient, so
every transfer landed.

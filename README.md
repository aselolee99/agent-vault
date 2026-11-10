# Agent Vault

> **Give AI money without giving it the keys to everything.**

Agent Vault is a policy-enforced vault for an AI agent's funds on NEAR. The
owner funds the vault and sets the policy. The agent can only spend through
the vault, and every spend is checked on-chain before any NEAR moves:

1. the recipient must be on the **allowlist**,
2. the spend must fit the **remaining daily allowance** (resets every UTC day),
3. spends at or above the **approval threshold** do not execute — they become
   **pending** and need an explicit `approve_spend` from the owner.

If a prompt-injected agent tries to pay an attacker, the contract rejects the
spend. The agent never holds a full-access key to the money.

Built for **NEAR HACKATHON 2026**, track **06 — AI Security & Defense**
(`security`). Testnet-only deployment; no mainnet funds are involved.

## The problem

AI agents need money to do useful work — pay for APIs, compute, data, other
agents. Today that means handing an agent a wallet key. A prompt injection,
a hallucinated address, or a runaway loop can then drain the whole balance in
seconds. There is no undo button on-chain.

Agent Vault fixes this at the contract level: the agent's key can only *ask*
the vault to spend, and the vault enforces the owner's policy before any
transfer happens. The worst case for a compromised agent is one small spend
inside the daily cap to an already-approved recipient.

## How it works

```
owner ── deposit ──▶ ┌──────────────┐
                    │  Agent Vault │── spend ──▶ allowlisted recipient
agent ── request_spend ──▶ │ (NEAR contract) │
                    │  policy check│
owner ── approve_spend ──▶ └──────────────┘  (large spends only)
```

- **Owner** (`owner`): funds the vault, manages the allowlist and limits,
  approves or rejects large spends. Holds the only full-access key.
- **Agent** (`agent`): calls only `request_spend`. In production it holds a
  **function-call access key scoped to `request_spend` on this contract**
  (see [Function-call key limits](#function-call-key-limits) below).
- **Policy**: `daily_limit` (max per UTC day), `approval_threshold`
  (spends at or above this go pending), `allowlist` (recipients).

Contract methods:

| Method | Who | What |
| --- | --- | --- |
| `new(owner, agent, daily_limit, approval_threshold)` | init | Create the vault. Requires `0 < approval_threshold <= daily_limit`. |
| `deposit()` | anyone, payable | Fund the vault. Attached NEAR stays in the contract. |
| `request_spend(to, amount)` | agent | Policy-checked spend. Executes, goes pending, or panics. |
| `approve_spend(spend_id)` | owner | Execute a pending spend (policy re-checked). |
| `reject_spend(spend_id)` | owner | Delete a pending spend; funds stay in the vault. |
| `add_recipient(account)` / `remove_recipient(account)` | owner | Manage the allowlist. |
| `set_daily_limit(daily_limit)` | owner | Change the daily limit (never below the threshold). |
| `get_policy()` / `get_balance()` / `get_spent_today()` / `get_pending()` / `get_pending_spend(id)` | view | Read state. |

Amounts are yoctoNEAR (`1 NEAR = 10^24`). The contract emits
[NEP-297](https://nomicon.io/Standards/EventsFormat) events with
`standard: "agent_vault"` for `deposit`, `spend_executed`,
`spend_requested`, `spend_approved`, `spend_rejected`, `recipient_added`,
`recipient_removed`, `daily_limit_changed`.

## Testnet deployment (live, verified 10 Nov 2026)

- **Contract:** `vault2.agentvault-owner.testnet`
- **Network:** NEAR testnet
- **Explorer:** https://testnet.nearblocks.io/address/vault2.agentvault-owner.testnet
- **Deploy tx:** https://explorer.testnet.near.org/transactions/EESf9utde6SemJXfDKMR6ZMJzY3D2UUnDgDsDX5RpWnC
- **Owner:** `agentvault-owner.testnet` — **Agent:** `agent2.agentvault-owner.testnet`
- **Demo policy:** daily limit 2 NEAR, approval threshold 1 NEAR, funded with
  4 NEAR. Allowlist: `api-provider.testnet`.
- **Demo result (see [DEMO.md](DEMO.md)):** 2 attacks blocked on-chain, 0.5 NEAR
  spent, 1.5 NEAR requested → pending → owner-approved. Final state:
  balance ~6.9988 NEAR, `spent_today` 2 NEAR, `total_spent` 2 NEAR, 0 pending.

**Known limitation:** a `transfer` to a *non-existent* NEAR account fails
(`AccountDoesNotExist`) and the funds are refunded — but the vault still
records the spend. So the owner must verify a recipient account exists
before allowlisting it. (Found during the first demo run: the first vault
recorded 2 NEAR of spends to a recipient that did not exist yet; the NEAR
was refunded and the balance was unchanged.)

## Quickstart

Prerequisites: Rust stable + `wasm32-unknown-unknown` target, `near` CLI
(near-cli-rs ≥ 0.30). Node is not required.

```sh
# 1. Build and test (cargo-near is required by near-sdk 5)
cd contract
cargo test
cargo near build

# 2. Configure (copy .env.example to .env and fill in your testnet accounts)
cp ../.env.example ../.env
# OWNER_ACCOUNT, AGENT_ACCOUNT, VAULT_ACCOUNT (a sub-account of the owner)

# 3. Deploy to testnet (creates the vault account, deploys, initializes)
../scripts/deploy.sh

# 4. Run the attack demo (see DEMO.md for the story and expected output)
../scripts/demo.sh
```

## Try it — do X, see Y

All commands run against the deployed testnet contract. Replace
`VAULT` with the deployed contract account (see above). Amounts are
yoctoNEAR strings. The `near` CLI signs with the keychain account named in
`sign-as`.

**Step 1 — Read the policy.**

```sh
near contract call-function as-read-only VAULT get_policy \
  json-args '{}' network-config testnet now
```

> See: `owner`, `agent`, `daily_limit: "2000000000000000000000000"`
> (2 NEAR), `approval_threshold: "1000000000000000000000000"` (1 NEAR),
> `allowlist`, `spent_today`, `balance`.

**Step 2 — Attack: the agent pays an address that is not allowlisted.**

```sh
near contract call-function as-transaction VAULT request_spend \
  json-args '{"to":"attacker.testnet","amount":"1000000000000000000000000"}' \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as AGENT network-config testnet sign-with-keychain send
```

> See: the transaction **fails** — `Smart contract panicked: recipient not
> allowlisted: attacker.testnet`. No funds move.

**Step 3 — Attack: the agent pays 50 NEAR, over the 2 NEAR daily limit.**

```sh
near contract call-function as-transaction VAULT request_spend \
  json-args '{"to":"api-provider.testnet","amount":"50000000000000000000000000"}' \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as AGENT network-config testnet sign-with-keychain send
```

> See: the transaction **fails** — `Smart contract panicked: amount exceeds
> daily limit`.

**Step 4 — The agent pays 0.5 NEAR to an allowlisted recipient. It works.**

```sh
# Owner allowlists the recipient and funds the vault (once):
near contract call-function as-transaction VAULT add_recipient \
  json-args '{"account":"api-provider.testnet"}' \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as OWNER network-config testnet sign-with-keychain send

near contract call-function as-transaction VAULT deposit \
  json-args '{}' \
  prepaid-gas '100.0 Tgas' attached-deposit '4 NEAR' \
  sign-as OWNER network-config testnet sign-with-keychain send

# Agent spends:
near contract call-function as-transaction VAULT request_spend \
  json-args '{"to":"api-provider.testnet","amount":"500000000000000000000000"}' \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as AGENT network-config testnet sign-with-keychain send
```

> See: the transaction **succeeds**. `get_spent_today` now returns
> `"500000000000000000000000"` (0.5 NEAR).

**Step 5 — The agent requests 1.5 NEAR (above the 1 NEAR threshold).**

```sh
near contract call-function as-transaction VAULT request_spend \
  json-args '{"to":"api-provider.testnet","amount":"1500000000000000000000000"}' \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as AGENT network-config testnet sign-with-keychain send
```

> See: the transaction succeeds, but the return value is
> `{"executed": false, "spend_id": 1, ...}` — the money did **not** move.
> `get_pending` lists spend #1. `get_spent_today` is unchanged.

**Step 6 — The owner approves the pending spend.**

```sh
near contract call-function as-transaction VAULT approve_spend \
  json-args '{"spend_id":1}' \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as OWNER network-config testnet sign-with-keychain send
```

> See: the transaction **succeeds** and 1.5 NEAR moves to
> `api-provider.testnet`. `get_pending` is empty, `spent_today` is 2 NEAR.

**Step 7 — The daily allowance resets on a new UTC day.** (No waiting
needed: `get_spent_today` returns 0 once the UTC day rolls over; the unit
test `daily_allowance_resets_on_new_day` covers this deterministically.)

## Function-call key limits

The track asks for this explicitly, so here it is. The agent's key is a
**function-call access key** added to the agent account like this:

```sh
near account add-key AGENT grant-function-call-access \
  --allowance '0.25 NEAR' \
  --contract-account-id VAULT \
  --function-names 'request_spend' \
  autogenerate-new-keypair save-to-keychain \
  sign-as AGENT network-config testnet sign-with-keychain send
```

What that key **can** do:

- Call exactly one method, `request_spend`, on exactly one contract
  (the vault). Nothing else. It cannot call `approve_spend`, `add_recipient`,
  or any other method, and it cannot call any other contract.

What it **cannot** do:

- Transfer NEAR directly, create accounts, add or delete keys, or stake.
- Spend more than its gas allowance (0.25 NEAR here) on fees; when the
  allowance runs out the key stops working until the owner tops it up.
- Move money by itself: even a successful `request_spend` only moves funds
  if the vault's policy allows it. The contract, not the key, is the
  security boundary.

Caveats (from the NEAR docs): an account that holds **only** function-call
keys cannot transfer NEAR or add keys, and can lock itself out if the gas
allowance runs out. That is why the agent account in this project keeps its
full-access key with the **owner** — the agent process is only ever given the
scoped function-call key. If the agent's key is compromised, the owner
deletes it (`near account delete-keys`) and the attacker still cannot touch
the funds: the vault only releases money under policy.

## Architecture

```
agent-vault/
├── contract/            # Rust smart contract (near-sdk 5)
│   ├── Cargo.toml
│   └── src/lib.rs       # Vault logic + unit tests
├── scripts/
│   ├── deploy.sh        # Build, create vault account, deploy, init
│   └── demo.sh          # Attack demo: 2 blocked attacks, 2 good spends
├── README.md            # This file
├── DEMO.md              # Demo story + real testnet transaction hashes
├── LICENSE              # MIT
└── .env.example         # Placeholder config (copy to .env, never commit it)
```

Design choices:

- **One contract, no dependencies beyond `near-sdk`.** No oracle, no
  solver, no off-chain service: the policy is enforced entirely on-chain,
  so the demo cannot fail on an external service.
- **Allowlist and pending spends live in plain `Vec`s in contract state.**
  The lists are tiny (a handful of recipients, a few pending spends), so
  linear scans keep the code auditable and avoid collection-API risk.
- **UTC-day allowance.** `spent_today` resets when the block timestamp
  crosses into a new UTC day; views report 0 on a new day even before the
  next state-changing call.
- **Approval re-checks the policy.** A pending spend is checked again at
  approval time (recipient still allowlisted, still fits the day's
  allowance), so the owner cannot be tricked into approving a spend the
  policy no longer allows. The owner can also `reject_spend` to delete a
  pending spend.
- **Threshold invariant.** `approval_threshold <= daily_limit` is enforced
  at init and in `set_daily_limit`, so a pending spend can always in
  principle be approved on a fresh day.

## Tests

```sh
cd contract && cargo test
```

18 unit tests cover: allowed spend succeeds; non-allowlisted recipient
rejected; spend larger than the whole daily limit rejected; over-cap
rejected; at/above-threshold spends go pending without moving funds; owner
approval executes; non-owner cannot approve / add recipients / change limits;
non-agent cannot request spends; daily allowance resets on a new day;
removed recipient can no longer receive or be approved; limit changes take
effect; approval re-checks the cap; owner can reject a pending spend;
deposit is open to anyone; init rejects a threshold above the daily limit.

## Tech used

- NEAR Protocol (testnet), Rust, near-sdk 5, `near` CLI (near-cli-rs 0.31)
- No other chain, no bridge, no oracle. NEAR is the whole stack: named
  accounts, function-call access keys, and contract-enforced policy are the
  features this project is built on.

## Hackathon

- Event: NEAR HACKATHON 2026, 1–14 Nov 2026 (submission closes 14 Nov 2026,
  23:59 UTC)
- Track: **06 — AI Security & Defense** (`security`). The project also fits
  the "Agent Commerce" build channel, but it is submitted under `security`.
- Team size: 1 (solo builder)
- Prize payout account (mainnet, for the submission form): `asell.near`
- This repository is a new project built during the hackathon window; it is
  not a port of an existing product. The Fomo Agent project (a separate BSC
  mint DApp by the same builder) is **not** part of this submission.

## License

MIT — see [LICENSE](LICENSE).

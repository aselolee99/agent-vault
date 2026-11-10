//! Agent Vault — a policy-enforced vault for an AI agent's funds on NEAR.
//!
//! The owner funds the vault and sets the policy. The agent may only spend
//! through `request_spend`, and every spend is checked on-chain:
//!   1. the recipient must be on the allowlist,
//!   2. the spend must fit the remaining daily allowance,
//!   3. spends at or above the approval threshold do NOT execute — they
//!      become pending and need an explicit `approve_spend` from the owner.
//!
//! The agent never holds a full-access key to the funds: it only ever holds
//! a function-call key scoped to `request_spend` on this contract (see
//! README.md for the key limits). The owner keeps the only full-access key.

use near_sdk::json_types::U128;
use near_sdk::{env, near, AccountId, NearToken, Promise};

/// Nanoseconds in one UTC day. NEAR block timestamps are nanoseconds.
const NS_PER_DAY: u64 = 86_400_000_000_000;

/// A spend that is waiting for the owner's approval.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSpend {
    pub id: u64,
    pub to: AccountId,
    pub amount: U128,
    /// UTC day on which the spend was requested.
    pub requested_day: u64,
}

/// What an agent `request_spend` call returns.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpendOutcome {
    /// True when the transfer was executed immediately.
    pub executed: bool,
    /// Set when the spend went to pending approval instead of executing.
    pub spend_id: Option<u64>,
    pub to: AccountId,
    pub amount: U128,
}

/// The vault's current policy and state, as seen by a view call.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub owner: AccountId,
    pub agent: AccountId,
    pub daily_limit: U128,
    pub approval_threshold: U128,
    pub allowlist: Vec<AccountId>,
    /// NEAR spent so far on the current UTC day (0 on a new day).
    pub spent_today: U128,
    pub current_day: u64,
    pub balance: U128,
    pub total_spent: U128,
    pub pending_count: u64,
}

#[near(contract_state)]
pub struct AgentVault {
    owner: AccountId,
    agent: AccountId,
    daily_limit: u128,
    approval_threshold: u128,
    allowlist: Vec<AccountId>,
    spent_today: u128,
    current_day: u64,
    pending: Vec<PendingSpend>,
    next_spend_id: u64,
    total_spent: u128,
}

impl Default for AgentVault {
    /// Only used when the contract state has never been written (i.e.
    /// before `new` runs). Real state always comes from `new`.
    fn default() -> Self {
        Self {
            owner: "owner.testnet".parse().unwrap(),
            agent: "agent.testnet".parse().unwrap(),
            daily_limit: 0,
            approval_threshold: 0,
            allowlist: Vec::new(),
            spent_today: 0,
            current_day: 0,
            pending: Vec::new(),
            next_spend_id: 1,
            total_spent: 0,
        }
    }
}

impl AgentVault {
    /// Current UTC day number, derived from the block timestamp.
    fn today() -> u64 {
        env::block_timestamp() / NS_PER_DAY
    }

    /// Reset the daily spend counter when a new UTC day starts.
    fn rollover_day(&mut self) {
        let day = Self::today();
        if day != self.current_day {
            self.current_day = day;
            self.spent_today = 0;
        }
    }

    fn assert_owner(&self) {
        assert_eq!(
            env::predecessor_account_id(),
            self.owner,
            "only the owner can call this method"
        );
    }

    fn assert_agent(&self) {
        assert_eq!(
            env::predecessor_account_id(),
            self.agent,
            "only the agent can call this method"
        );
    }

    /// Emit a NEP-297 standard event.
    fn emit(event: &str, data: near_sdk::serde_json::Value) {
        env::log_str(&format!(
            "EVENT_JSON:{{\"standard\":\"agent_vault\",\"version\":\"1.0.0\",\"event\":\"{}\",\"data\":{}}}",
            event,
            near_sdk::serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string())
        ));
    }

    /// Shared checks for executing a spend now. Panics on any violation.
    /// Caller must have already rolled the day over.
    fn assert_can_execute(&self, to: &AccountId, amount: u128) {
        assert!(
            self.allowlist.contains(to),
            "recipient not allowlisted: {}",
            to
        );
        assert!(
            self.spent_today.checked_add(amount).unwrap() <= self.daily_limit,
            "spend exceeds daily limit"
        );
        assert!(
            env::account_balance().as_yoctonear() >= amount,
            "vault balance too low for this spend"
        );
    }

    fn execute_spend(&mut self, to: AccountId, amount: u128) {
        self.spent_today += amount;
        self.total_spent += amount;
        Promise::new(to.clone())
            .transfer(NearToken::from_yoctonear(amount))
            .detach();
        Self::emit(
            "spend_executed",
            near_sdk::serde_json::json!({
                "to": to.to_string(),
                "amount": amount.to_string(),
                "spent_today": self.spent_today.to_string(),
            }),
        );
    }
}

#[near]
impl AgentVault {
    /// Initialize the vault. `daily_limit` is the most the agent may spend
    /// per UTC day; `approval_threshold` is the smallest spend that needs
    /// the owner's explicit approval instead of executing immediately.
    /// Must hold: 0 < approval_threshold <= daily_limit.
    #[init]
    pub fn new(
        owner: AccountId,
        agent: AccountId,
        daily_limit: U128,
        approval_threshold: U128,
    ) -> Self {
        assert!(daily_limit.0 > 0, "daily_limit must be greater than 0");
        assert!(
            approval_threshold.0 > 0,
            "approval_threshold must be greater than 0"
        );
        assert!(
            approval_threshold.0 <= daily_limit.0,
            "approval_threshold must not exceed daily_limit"
        );
        Self {
            owner,
            agent,
            daily_limit: daily_limit.0,
            approval_threshold: approval_threshold.0,
            allowlist: Vec::new(),
            spent_today: 0,
            current_day: Self::today(),
            pending: Vec::new(),
            next_spend_id: 1,
            total_spent: 0,
        }
    }

    /// Fund the vault. Anyone may deposit; the attached NEAR stays in the
    /// contract balance and can only leave through a policy-checked spend.
    #[payable]
    pub fn deposit(&mut self) -> U128 {
        let amount = env::attached_deposit().as_yoctonear();
        Self::emit(
            "deposit",
            near_sdk::serde_json::json!({
                "from": env::predecessor_account_id().to_string(),
                "amount": amount.to_string(),
            }),
        );
        U128(env::account_balance().as_yoctonear())
    }

    /// The agent requests a spend. The agent signs this with a function-call
    /// access key scoped to this contract — it can never move funds itself.
    ///
    /// - Recipient not allowlisted -> rejected (panic).
    /// - Amount larger than the whole daily limit -> rejected (panic), it
    ///   could never execute on any day.
    /// - Amount >= approval_threshold -> recorded as pending, NOT executed.
    ///   The owner must call `approve_spend`.
    /// - Otherwise, if it fits the remaining daily allowance, it executes.
    pub fn request_spend(&mut self, to: AccountId, amount: U128) -> SpendOutcome {
        self.assert_agent();
        let amount = amount.0;
        assert!(amount > 0, "amount must be greater than 0");
        assert!(
            self.allowlist.contains(&to),
            "recipient not allowlisted: {}",
            to
        );
        assert!(
            amount <= self.daily_limit,
            "amount exceeds daily limit"
        );
        self.rollover_day();

        if amount >= self.approval_threshold {
            let id = self.next_spend_id;
            self.next_spend_id += 1;
            self.pending.push(PendingSpend {
                id,
                to: to.clone(),
                amount: U128(amount),
                requested_day: self.current_day,
            });
            Self::emit(
                "spend_requested",
                near_sdk::serde_json::json!({
                    "spend_id": id,
                    "to": to.to_string(),
                    "amount": amount.to_string(),
                }),
            );
            return SpendOutcome {
                executed: false,
                spend_id: Some(id),
                to,
                amount: U128(amount),
            };
        }

        self.assert_can_execute(&to, amount);
        self.execute_spend(to.clone(), amount);
        SpendOutcome {
            executed: true,
            spend_id: None,
            to,
            amount: U128(amount),
        }
    }

    /// The owner approves a pending spend, executing it. The policy is
    /// re-checked at approval time: the recipient must still be allowlisted
    /// and the spend must still fit the remaining daily allowance.
    pub fn approve_spend(&mut self, spend_id: u64) -> U128 {
        self.assert_owner();
        let pos = self
            .pending
            .iter()
            .position(|p| p.id == spend_id)
            .expect("pending spend not found");
        let spend = self.pending[pos].clone();
        self.rollover_day();
        self.assert_can_execute(&spend.to, spend.amount.0);
        self.pending.remove(pos);
        self.execute_spend(spend.to, spend.amount.0);
        Self::emit(
            "spend_approved",
            near_sdk::serde_json::json!({ "spend_id": spend_id }),
        );
        spend.amount
    }

    /// The owner rejects (deletes) a pending spend. The funds stay in the vault.
    pub fn reject_spend(&mut self, spend_id: u64) {
        self.assert_owner();
        let pos = self
            .pending
            .iter()
            .position(|p| p.id == spend_id)
            .expect("pending spend not found");
        let spend = self.pending.remove(pos);
        Self::emit(
            "spend_rejected",
            near_sdk::serde_json::json!({
                "spend_id": spend_id,
                "to": spend.to.to_string(),
                "amount": spend.amount.0.to_string(),
            }),
        );
    }

    /// Owner only: allow a recipient to receive agent spends.
    pub fn add_recipient(&mut self, account: AccountId) {
        self.assert_owner();
        if !self.allowlist.contains(&account) {
            self.allowlist.push(account.clone());
        }
        Self::emit(
            "recipient_added",
            near_sdk::serde_json::json!({ "account": account.to_string() }),
        );
    }

    /// Owner only: remove a recipient. Pending spends to this recipient can
    /// no longer be approved.
    pub fn remove_recipient(&mut self, account: AccountId) {
        self.assert_owner();
        self.allowlist.retain(|a| a != &account);
        Self::emit(
            "recipient_removed",
            near_sdk::serde_json::json!({ "account": account.to_string() }),
        );
    }

    /// Owner only: change the daily spending limit (yoctoNEAR).
    pub fn set_daily_limit(&mut self, daily_limit: U128) {
        self.assert_owner();
        assert!(daily_limit.0 > 0, "daily_limit must be greater than 0");
        assert!(
            self.approval_threshold <= daily_limit.0,
            "daily_limit must not go below approval_threshold"
        );
        self.daily_limit = daily_limit.0;
        Self::emit(
            "daily_limit_changed",
            near_sdk::serde_json::json!({ "daily_limit": daily_limit.0.to_string() }),
        );
    }

    // ---- Views ----

    pub fn get_policy(&self) -> Policy {
        Policy {
            owner: self.owner.clone(),
            agent: self.agent.clone(),
            daily_limit: U128(self.daily_limit),
            approval_threshold: U128(self.approval_threshold),
            allowlist: self.allowlist.clone(),
            spent_today: U128(self.spent_today),
            current_day: self.current_day,
            balance: U128(env::account_balance().as_yoctonear()),
            total_spent: U128(self.total_spent),
            pending_count: self.pending.len() as u64,
        }
    }

    pub fn get_balance(&self) -> U128 {
        U128(env::account_balance().as_yoctonear())
    }

    /// NEAR spent so far on the current UTC day (0 if a new day started).
    pub fn get_spent_today(&self) -> U128 {
        if Self::today() == self.current_day {
            U128(self.spent_today)
        } else {
            U128(0)
        }
    }

    pub fn get_pending(&self) -> Vec<PendingSpend> {
        self.pending.clone()
    }

    pub fn get_pending_spend(&self, spend_id: u64) -> Option<PendingSpend> {
        self.pending.iter().find(|p| p.id == spend_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::testing_env;

    /// 1 NEAR in yoctoNEAR.
    const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

    fn owner() -> AccountId {
        accounts(0)
    }
    fn agent() -> AccountId {
        accounts(1)
    }
    fn bob() -> AccountId {
        accounts(2)
    }
    fn attacker() -> AccountId {
        accounts(3)
    }

    fn ctx(predecessor: AccountId, balance: u128, timestamp_ns: u64) {
        let mut b = VMContextBuilder::new();
        b.predecessor_account_id(predecessor);
        b.account_balance(NearToken::from_yoctonear(balance));
        b.block_timestamp(timestamp_ns);
        testing_env!(b.build());
    }

    /// Fresh vault: daily limit 10 NEAR, approval threshold 5 NEAR,
    /// allowlist = [bob], funded with 20 NEAR, on day 0.
    fn vault() -> AgentVault {
        ctx(owner(), 20 * ONE_NEAR, 0);
        let mut v = AgentVault::new(owner(), agent(), U128(10 * ONE_NEAR), U128(5 * ONE_NEAR));
        v.allowlist.push(bob());
        v
    }

    /// Re-enter the test env as `predecessor` on the given day.
    fn as_user(predecessor: AccountId, day: u64) {
        ctx(predecessor, 20 * ONE_NEAR, day * NS_PER_DAY);
    }

    #[test]
    fn allowed_spend_succeeds() {
        let mut v = vault();
        as_user(agent(), 0);
        let out = v.request_spend(bob(), U128(ONE_NEAR));
        assert!(out.executed);
        assert_eq!(out.spend_id, None);
        assert_eq!(v.spent_today, ONE_NEAR);
        assert_eq!(v.total_spent, ONE_NEAR);
        assert!(v.pending.is_empty());
    }

    #[test]
    #[should_panic(expected = "recipient not allowlisted")]
    fn non_allowlisted_recipient_rejected() {
        let mut v = vault();
        as_user(agent(), 0);
        v.request_spend(attacker(), U128(ONE_NEAR));
    }

    #[test]
    #[should_panic(expected = "amount exceeds daily limit")]
    fn spend_larger_than_whole_daily_limit_rejected() {
        let mut v = vault();
        as_user(agent(), 0);
        v.request_spend(bob(), U128(11 * ONE_NEAR));
    }

    #[test]
    #[should_panic(expected = "spend exceeds daily limit")]
    fn over_daily_cap_rejected_small_spends() {
        let mut v = vault();
        as_user(agent(), 0);
        v.request_spend(bob(), U128(4 * ONE_NEAR));
        v.request_spend(bob(), U128(4 * ONE_NEAR));
        // 4 + 4 + 4 = 12 > 10
        v.request_spend(bob(), U128(4 * ONE_NEAR));
    }

    #[test]
    fn spend_at_threshold_goes_pending_not_executed() {
        let mut v = vault();
        as_user(agent(), 0);
        let out = v.request_spend(bob(), U128(5 * ONE_NEAR));
        assert!(!out.executed);
        assert_eq!(out.spend_id, Some(1));
        assert_eq!(v.spent_today, 0, "pending spend must not move funds");
        assert_eq!(v.pending.len(), 1);
        assert_eq!(v.pending[0].to, bob());
    }

    #[test]
    fn spend_above_threshold_goes_pending() {
        let mut v = vault();
        as_user(agent(), 0);
        let out = v.request_spend(bob(), U128(8 * ONE_NEAR));
        assert!(!out.executed);
        assert_eq!(out.spend_id, Some(1));
        assert_eq!(v.spent_today, 0);
    }

    #[test]
    fn owner_approval_executes_pending_spend() {
        let mut v = vault();
        as_user(agent(), 0);
        let out = v.request_spend(bob(), U128(8 * ONE_NEAR));
        assert!(!out.executed);
        as_user(owner(), 0);
        let amount = v.approve_spend(1);
        assert_eq!(amount.0, 8 * ONE_NEAR);
        assert_eq!(v.spent_today, 8 * ONE_NEAR);
        assert!(v.pending.is_empty());
    }

    #[test]
    #[should_panic(expected = "only the owner can call this method")]
    fn non_owner_cannot_approve() {
        let mut v = vault();
        as_user(agent(), 0);
        v.request_spend(bob(), U128(8 * ONE_NEAR));
        // the agent tries to approve its own pending spend
        v.approve_spend(1);
    }

    #[test]
    #[should_panic(expected = "only the owner can call this method")]
    fn non_owner_cannot_add_recipient() {
        let mut v = vault();
        as_user(agent(), 0);
        v.add_recipient(attacker());
    }

    #[test]
    #[should_panic(expected = "only the owner can call this method")]
    fn non_owner_cannot_set_daily_limit() {
        let mut v = vault();
        as_user(agent(), 0);
        v.set_daily_limit(U128(ONE_NEAR));
    }

    #[test]
    #[should_panic(expected = "only the agent can call this method")]
    fn non_agent_cannot_request_spend() {
        let mut v = vault();
        as_user(owner(), 0);
        v.request_spend(bob(), U128(ONE_NEAR));
    }

    #[test]
    fn daily_allowance_resets_on_new_day() {
        let mut v = vault();
        as_user(agent(), 0);
        v.request_spend(bob(), U128(4 * ONE_NEAR));
        v.request_spend(bob(), U128(4 * ONE_NEAR));
        assert_eq!(v.get_spent_today().0, 8 * ONE_NEAR);
        // Next UTC day: allowance is fresh.
        as_user(agent(), 1);
        assert_eq!(v.get_spent_today().0, 0);
        let out = v.request_spend(bob(), U128(4 * ONE_NEAR));
        assert!(out.executed);
        assert_eq!(v.spent_today, 4 * ONE_NEAR);
    }

    #[test]
    fn removed_recipient_cannot_receive_or_be_approved() {
        let mut v = vault();
        as_user(agent(), 0);
        let out = v.request_spend(bob(), U128(8 * ONE_NEAR));
        assert!(!out.executed);
        as_user(owner(), 0);
        v.remove_recipient(bob());
        // Approval must now fail because the recipient is no longer allowlisted.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            v.approve_spend(1);
        }));
        assert!(result.is_err());
        // And fresh spends to bob are rejected too.
        as_user(agent(), 0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            v.request_spend(bob(), U128(ONE_NEAR));
        }));
        assert!(result.is_err());
    }

    #[test]
    fn set_daily_limit_changes_the_cap() {
        let mut v = vault();
        as_user(owner(), 0);
        // New limit 6 NEAR (must stay >= the 5 NEAR approval threshold).
        v.set_daily_limit(U128(6 * ONE_NEAR));
        as_user(agent(), 0);
        v.request_spend(bob(), U128(4 * ONE_NEAR)); // executes, spent = 4
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            v.request_spend(bob(), U128(4 * ONE_NEAR)); // 4 + 4 = 8 > 6
        }));
        assert!(result.is_err(), "4 NEAR must exceed the new 6 NEAR cap");
        let out = v.request_spend(bob(), U128(ONE_NEAR)); // 4 + 1 = 5 <= 6
        assert!(out.executed);
    }

    #[test]
    fn approval_rechecks_daily_cap() {
        let mut v = vault();
        as_user(agent(), 0);
        v.request_spend(bob(), U128(8 * ONE_NEAR)); // pending, id 1
        v.request_spend(bob(), U128(4 * ONE_NEAR)); // executes, spent = 4
        as_user(owner(), 0);
        // 4 already spent + 8 pending = 12 > 10 cap, so approval must fail
        // and the pending spend must stay pending.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            v.approve_spend(1);
        }));
        assert!(result.is_err());
        assert_eq!(v.pending.len(), 1);
    }

    #[test]
    fn owner_can_reject_pending_spend() {
        let mut v = vault();
        as_user(agent(), 0);
        v.request_spend(bob(), U128(8 * ONE_NEAR));
        as_user(owner(), 0);
        v.reject_spend(1);
        assert!(v.pending.is_empty());
        assert_eq!(v.spent_today, 0);
    }

    #[test]
    fn deposit_is_open_to_anyone() {
        let mut v = vault();
        as_user(attacker(), 0);
        let balance = v.deposit();
        assert_eq!(balance.0, 20 * ONE_NEAR);
    }

    #[test]
    #[should_panic(expected = "approval_threshold must not exceed daily_limit")]
    fn init_rejects_threshold_above_daily_limit() {
        ctx(owner(), 0, 0);
        AgentVault::new(owner(), agent(), U128(ONE_NEAR), U128(2 * ONE_NEAR));
    }
}

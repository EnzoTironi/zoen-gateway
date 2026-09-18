//! Prepaid integer micro-USD ledger. Five entries only: grant, topup, reserve, settle, release.

use std::collections::{HashMap, HashSet};

use parking_lot::Mutex;

use crate::CatalogError;

/// $1.00 in micro-USD — Treg signup credit.
pub const SIGNUP_GRANT_MICRO: i64 = 1_000_000;

/// In-memory ledger for local/mock deployments.
#[derive(Debug, Default)]
pub struct MemoryLedger {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    balance: HashMap<String, i64>,
    holds: HashMap<String, Hold>,
    granted: HashSet<String>,
}

#[derive(Clone, Debug)]
struct Hold {
    subject: String,
    reserved: i64,
}

impl MemoryLedger {
    /// Empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure the subject has received the one-time signup grant.
    ///
    /// # Errors
    ///
    /// Never today; `Result` keeps the grant/reserve/settle/release shape.
    pub fn ensure_signup_grant(&self, subject: &str) -> Result<i64, CatalogError> {
        let mut g = self.inner.lock();
        if g.granted.insert(subject.to_owned()) {
            *g.balance.entry(subject.to_owned()).or_insert(0) += SIGNUP_GRANT_MICRO;
        }
        let out = *g.balance.get(subject).unwrap_or(&0);
        drop(g);
        Ok(out)
    }

    /// Current balance (does not grant).
    #[must_use]
    pub fn balance(&self, subject: &str) -> i64 {
        self.inner.lock().balance.get(subject).copied().unwrap_or(0)
    }

    /// Credit (grant or mock top-up).
    ///
    /// # Errors
    ///
    /// Non-positive `micro`.
    pub fn grant(&self, subject: &str, micro: i64) -> Result<i64, CatalogError> {
        if micro <= 0 {
            return Err(CatalogError::Invalid("grant must be positive"));
        }
        let mut g = self.inner.lock();
        let slot = g.balance.entry(subject.to_owned()).or_insert(0);
        *slot = slot.saturating_add(micro);
        let out = *slot;
        drop(g);
        Ok(out)
    }

    /// Reserve `micro` for `call_id`. Fail with payment required if short.
    ///
    /// # Errors
    ///
    /// Negative reserve, duplicate `call_id`, or insufficient balance.
    pub fn reserve(&self, subject: &str, call_id: &str, micro: i64) -> Result<(), CatalogError> {
        if micro < 0 {
            return Err(CatalogError::Invalid("reserve cannot be negative"));
        }
        if micro == 0 {
            return Ok(());
        }
        let mut g = self.inner.lock();
        if g.holds.contains_key(call_id) {
            return Err(CatalogError::Invalid("call already reserved"));
        }
        let bal = g.balance.entry(subject.to_owned()).or_insert(0);
        if *bal < micro {
            return Err(CatalogError::PaymentRequired {
                balance_micro: *bal,
                estimated_cost_micro: micro,
            });
        }
        *bal -= micro;
        g.holds.insert(
            call_id.to_owned(),
            Hold {
                subject: subject.to_owned(),
                reserved: micro,
            },
        );
        drop(g);
        Ok(())
    }

    /// Consume a hold. `actual` may be less than reserved (refund the rest).
    ///
    /// # Errors
    ///
    /// Never today; missing holds are no-ops so settle/release stay idempotent.
    pub fn settle(&self, call_id: &str, actual: i64) -> Result<(), CatalogError> {
        let mut g = self.inner.lock();
        let Some(hold) = g.holds.remove(call_id) else {
            return Ok(());
        };
        let refund = hold.reserved.saturating_sub(actual.max(0));
        if refund > 0 {
            *g.balance.entry(hold.subject).or_insert(0) += refund;
        }
        drop(g);
        Ok(())
    }

    /// Release a hold without charging.
    ///
    /// # Errors
    ///
    /// Same as [`Self::settle`].
    pub fn release(&self, call_id: &str) -> Result<(), CatalogError> {
        self.settle(call_id, 0)
    }
}

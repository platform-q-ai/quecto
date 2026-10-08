//! The models a provider refused for the account in use (#2435), held by
//! the catalogue snapshot store: a refusal is held until it expires (its
//! hold, configurable, defaults to an hour) or the provider next serves the
//! model, so a transient refusal (a rollout's 404) never bans a model for
//! good.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::domain::catalogue::value_objects::catalogue::ModelRef;

/// One held refusal: the provider's reason and when the hold ends.
#[derive(Debug, Clone)]
struct Refusal {
    reason: String,
    until: Instant,
}

/// The refusals one snapshot store holds, shared by its clones.
#[derive(Debug, Clone, Default)]
pub(super) struct RefusalLedger {
    held: Arc<Mutex<HashMap<ModelRef, Refusal>>>,
}

impl RefusalLedger {
    /// Hold a refusal of `reference` for `held_for` from `now`. True when it
    /// is new: one already held keeps its first reason and hold; one whose
    /// hold has ended is replaced.
    pub(super) fn record(
        &self,
        reference: &ModelRef,
        reason: &str,
        now: Instant,
        held_for: Duration,
    ) -> bool {
        let mut held = self.lock();
        if held
            .get(reference)
            .is_some_and(|refusal| now < refusal.until)
        {
            return false;
        }
        held.insert(
            reference.clone(),
            Refusal {
                reason: reason.to_string(),
                // A hold too long to represent is held for as long as the
                // clock goes: never shorter than asked.
                until: now
                    .checked_add(held_for)
                    .unwrap_or(now + Duration::from_secs(u32::MAX.into())),
            },
        );
        true
    }

    /// Release the refusal of `reference`, if one is held: the provider has
    /// served the model. True when one was held.
    pub(super) fn clear(&self, reference: &ModelRef) -> bool {
        self.lock().remove(reference).is_some()
    }

    /// The reason `reference` is refused at `now`, if a hold covers it.
    pub(super) fn reason_at(&self, reference: &ModelRef, now: Instant) -> Option<String> {
        self.lock()
            .get(reference)
            .filter(|refusal| now < refusal.until)
            .map(|refusal| refusal.reason.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ModelRef, Refusal>> {
        self.held
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

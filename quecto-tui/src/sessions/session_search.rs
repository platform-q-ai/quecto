//! The search box's flight control (#2010): which metadata search is in the
//! air, which answer may still be shown, and when the next one goes out. Pure
//! state — no I/O, no scope or match policy (the harness owns both).
//!
//! Single flight, latest wins: at most one search is outstanding per picker;
//! an edit made while one is outstanding is sent when that one settles, so a
//! burst of keystrokes costs the harness two searches, not one per key, with
//! no timer. Every edit — of the query or of the scope — advances the
//! generation, and only an answer that carries the latest generation under
//! the id that was sent is shown. Everything else is discarded.

/// What to do with an answer that arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// Not the search in flight (another tab's, a closed picker's): ignore it.
    Foreign,
    /// The search in flight, overtaken by an edit: discard, and send the
    /// latest if `resend`.
    Stale { resend: bool },
    /// The search in flight and still the latest: show it.
    Fresh,
}

#[derive(Debug, Default)]
pub struct SearchFlight {
    generation: u64,
    in_flight: Option<(String, u64)>,
    queued: bool,
}

impl SearchFlight {
    /// The query or scope was edited and wants a search: every earlier answer
    /// is stale from now on. `Some(generation)` when it should be sent now.
    pub fn edited(&mut self) -> Option<u64> {
        self.generation += 1;
        self.queued = self.in_flight.is_some();
        (!self.queued).then_some(self.generation)
    }

    /// The edit wants no search (the box was cleared, the picker closed or
    /// listed again): nothing in flight may be shown, nothing is queued.
    pub fn superseded(&mut self) {
        self.generation += 1;
        self.queued = false;
    }

    /// The picker opened or closed: whatever is in the air is nobody's any
    /// more (its answer will be foreign), and the next edit goes straight out.
    pub fn abandon(&mut self) {
        self.generation += 1;
        self.in_flight = None;
        self.queued = false;
    }

    /// The generation the search under `id` was sent with, if it is in flight.
    pub fn sent_generation(&self, id: Option<&str>) -> Option<u64> {
        let (_, generation) = self.in_flight.as_ref().filter(|_| self.owns(id))?;
        Some(*generation)
    }

    /// The search of `generation` went out under `id`.
    pub fn sent(&mut self, id: String, generation: u64) {
        debug_assert!(self.in_flight.is_none(), "single flight");
        debug_assert_eq!(generation, self.generation, "only the latest is sent");
        self.in_flight = Some((id, generation));
    }

    /// The send failed: nothing is in flight after all.
    pub fn unsent(&mut self) {
        self.in_flight = None;
        self.queued = false;
    }

    /// Whether `id` is the search in flight.
    pub fn owns(&self, id: Option<&str>) -> bool {
        matches!((&self.in_flight, id), (Some((sent, _)), Some(id)) if sent == id)
    }

    /// An answer (or a failure) for `id` arrived, echoing `generation`.
    pub fn settle(&mut self, id: Option<&str>, generation: Option<u64>) -> Settled {
        if !self.owns(id) {
            return Settled::Foreign;
        }
        let (_, sent) = self.in_flight.take().expect("owned");
        // Affirmative: the echo is the generation sent AND still the latest.
        if generation == Some(sent) && sent == self.generation {
            debug_assert!(!self.queued, "an edit would have advanced the generation");
            Settled::Fresh
        } else {
            Settled::Stale {
                resend: std::mem::take(&mut self.queued),
            }
        }
    }

    /// The generation to send for a queued edit (after a stale settle).
    pub fn latest(&self) -> u64 {
        self.generation
    }

    pub fn is_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }
}

#[cfg(test)]
#[path = "session_search_tests.rs"]
mod tests;

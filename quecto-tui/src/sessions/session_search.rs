//! The search box's flight control (#2010): which metadata search is in the
//! air, which answer may still be shown, what the rows on screen are worth
//! for the text in the box, and when the next search goes out. Pure state —
//! no I/O, no clock of its own, no scope or match policy (the harness owns
//! both).
//!
//! Single flight, latest wins: at most one search is outstanding per picker;
//! an edit made while one is outstanding is sent when that one settles. A
//! burst typed FASTER than one round trip therefore costs two searches, not
//! one per key; typing slower than a round trip costs one search per key.
//! Every edit — of the query or of the scope — advances the generation. Only
//! the answer that carries the latest generation under the id that was sent
//! SETTLES the box; an overtaken answer is still *progress* — newer than the
//! rows on screen — and may be shown while the box stays unsettled, unless
//! the box was cleared or the scope changed since it was sent. A search that
//! is not answered within [`ANSWER_TIMEOUT`] is re-issued once for the latest
//! text and then given up, so a lost answer never wedges the box.
use tokio::time::Instant;

/// How long an answer may take before the flight is given up (R1-T4).
pub const ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What the rows on screen are worth for the text in the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlightState {
    /// No search is wanted, or the latest one was answered: the rows are the
    /// answer for the text in the box.
    Settled,
    /// The latest text is being asked: the rows belong to older text.
    Searching,
    /// The latest text is unanswered and nothing is asking (a failed send, a
    /// lost connection, a search given up): the next edit asks again.
    Stalled,
}

/// What became of a search that was not answered in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overdue {
    /// Given up once: send the latest text again, as this generation.
    Retry(u64),
    /// The retry was not answered either: stalled until the next edit.
    GaveUp,
    /// Nobody wants its answer any more (the box was cleared): just dropped.
    Moot,
}

/// What to do with an answer that arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// Not the exact search in flight (another client's, a closed picker's): ignore it.
    Foreign,
    /// The search in flight, overtaken by an edit: show it only as
    /// `progress`, never as the settled answer, and send the latest if `resend`.
    Stale { resend: bool, progress: bool },
    /// The search in flight and still the latest: show it, settled.
    Fresh,
}

#[derive(Debug)]
struct InFlight {
    id: String,
    generation: u64,
    deadline: Instant,
}

#[derive(Debug, Default)]
pub struct SearchFlight {
    generation: u64,
    in_flight: Option<InFlight>,
    queued: bool,
    /// The latest edit wants a search (not a listing, not a closed picker).
    wanted: bool,
    /// The latest edit's own answer arrived.
    answered: bool,
    /// The flight in the air is the one retry of an unanswered search.
    retried: bool,
    /// A search sent at or before this generation is never shown: the box
    /// was cleared, the scope changed or the picker closed since.
    floor: u64,
}

impl SearchFlight {
    /// The query was edited and wants a search: every earlier answer is
    /// overtaken from now on. `Some(generation)` when it should be sent now.
    pub fn edited(&mut self) -> Option<u64> {
        self.generation += 1;
        (self.wanted, self.answered, self.retried) = (true, false, false);
        self.queued = self.in_flight.is_some();
        (!self.queued).then_some(self.generation)
    }

    /// The scope changed under a query: as [`Self::edited`], and nothing
    /// asked of the old scope may be shown, not even as progress.
    pub fn scope_changed(&mut self) -> Option<u64> {
        self.floor = self.generation;
        self.edited()
    }

    /// The edit wants no search (the box was cleared, the scope listed
    /// again): nothing in flight may be shown, nothing is queued.
    pub fn superseded(&mut self) {
        self.generation += 1;
        self.floor = self.generation;
        (self.wanted, self.queued) = (false, false);
    }

    /// The picker opened or closed: whatever is in the air is nobody's any
    /// more (its answer will be foreign), and the next edit goes straight out.
    pub fn abandon(&mut self) {
        self.superseded();
        self.in_flight = None;
    }

    /// The connection was lost with the picker open: the flight died with it,
    /// and the text in the box stays unanswered until the next edit.
    pub fn interrupted(&mut self) {
        let wanted = self.wanted;
        self.abandon();
        (self.wanted, self.answered) = (wanted, false);
    }

    /// The generation the search under `id` was sent with, if it is in flight.
    pub fn sent_generation(&self, id: Option<&str>) -> Option<u64> {
        let flight = self.in_flight.as_ref().filter(|_| self.owns(id))?;
        Some(flight.generation)
    }

    /// The search of `generation` went out under `id` at `now`.
    pub fn sent(&mut self, id: String, generation: u64, now: Instant) {
        debug_assert!(self.in_flight.is_none(), "single flight");
        debug_assert_eq!(generation, self.generation, "only the latest is sent");
        self.in_flight = Some(InFlight {
            id,
            generation,
            deadline: now + ANSWER_TIMEOUT,
        });
    }

    /// The send failed: nothing is in flight after all.
    pub fn unsent(&mut self) {
        self.in_flight = None;
        self.queued = false;
    }

    /// Whether `id` is the search in flight.
    pub fn owns(&self, id: Option<&str>) -> bool {
        matches!((&self.in_flight, id), (Some(flight), Some(id)) if flight.id == id)
    }

    /// An answer (or a failure) for `id` arrived, echoing `generation`.
    pub fn settle(&mut self, id: Option<&str>, generation: Option<u64>) -> Settled {
        if !self.owns(id) {
            return Settled::Foreign;
        }
        let sent = self.in_flight.take().expect("owned").generation;
        self.retried = false;
        // Affirmative: the echo is the generation sent AND still the latest.
        let echoed = generation == Some(sent);
        if echoed && sent == self.generation && self.wanted {
            debug_assert!(!self.queued, "an edit would have advanced the generation");
            self.answered = true;
            Settled::Fresh
        } else {
            Settled::Stale {
                resend: std::mem::take(&mut self.queued),
                progress: echoed && sent > self.floor,
            }
        }
    }

    /// The latest edit's answer was a failure: the text stays unanswered.
    pub fn unanswered(&mut self) {
        self.answered = false;
    }

    /// When the search in flight is overdue, if one is in flight.
    pub fn deadline(&self) -> Option<Instant> {
        self.in_flight.as_ref().map(|flight| flight.deadline)
    }

    /// Give up a search that is overdue at `now`: its answer, should it ever
    /// come, is foreign. `None` while nothing is overdue.
    pub fn overdue(&mut self, now: Instant) -> Option<Overdue> {
        self.in_flight.take_if(|flight| flight.deadline <= now)?;
        self.queued = false;
        Some(match (self.wanted, self.retried) {
            (false, _) => Overdue::Moot,
            (true, false) => {
                self.retried = true;
                Overdue::Retry(self.generation)
            }
            (true, true) => {
                self.retried = false;
                Overdue::GaveUp
            }
        })
    }

    /// The generation to send for a queued edit (after a stale settle).
    pub fn latest(&self) -> u64 {
        self.generation
    }

    pub fn is_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    pub fn state(&self) -> FlightState {
        match (self.wanted && !self.answered, self.is_in_flight()) {
            (false, _) => FlightState::Settled,
            (true, true) => FlightState::Searching,
            (true, false) => FlightState::Stalled,
        }
    }
}

#[cfg(test)]
#[path = "session_search_tests.rs"]
mod tests;

//! How long a broker started on demand lives (#2615, ADR-0032): it stops
//! itself once it has held nothing for a whole idle period, so no stale
//! broker keeps serving a stale policy; the next session starts it again.
//! An always-on broker (the systemd unit, a foreground `run`) never stops
//! on its own.
//!
//! "Holds nothing" is the caller's observation (no connection, no root, no
//! permit, queued wait or lease, uncertain ones included); this value only
//! times it, on the authority's own monotonic clock.

/// The idle period of a broker started on demand: one hour.
pub const AUTHORITY_IDLE_STOP_MS: u64 = 60 * 60 * 1000;

/// What the idle timer says after one observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleVerdict {
    /// The broker holds something (or never stops): keep serving.
    Holding,
    /// Idle since an earlier observation; stop at this instant if it stays so.
    IdleUntil(u64),
    /// Idle for the whole period: stop now.
    Stop,
}

/// Times how long the broker has held nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleWatch {
    period_ms: u64,
    idle_since: Option<u64>,
}

impl IdleWatch {
    /// A watch that stops after `period_ms` of holding nothing. A zero
    /// period would stop a broker in the instant it starts, before its
    /// first client could connect, so it is refused.
    pub fn new(period_ms: u64) -> Option<Self> {
        match period_ms {
            0 => None,
            _ => Some(Self {
                period_ms,
                idle_since: None,
            }),
        }
    }

    /// Observe whether the broker holds anything at `now`. Holding anything
    /// forgets the idle start, so the next idle spell runs a whole period;
    /// `Stop` is only ever answered for an observation of holding nothing.
    pub fn observe(&mut self, holds_anything: bool, now: u64) -> IdleVerdict {
        match holds_anything {
            true => {
                self.idle_since = None;
                IdleVerdict::Holding
            }
            false => {
                let since = *self.idle_since.get_or_insert(now);
                let deadline = since.saturating_add(self.period_ms);
                debug_assert!(deadline > since || since == u64::MAX, "a positive period");
                match now >= deadline {
                    true => IdleVerdict::Stop,
                    false => IdleVerdict::IdleUntil(deadline),
                }
            }
        }
    }
}

/// Whether a broker stops itself when idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityLifetime {
    /// Serves until it is stopped (the systemd unit, a foreground `run`).
    AlwaysOn,
    /// Started on demand: stops itself once idle for the watch's period.
    StopWhenIdle(IdleWatch),
}

impl AuthorityLifetime {
    /// Observe whether the broker holds anything at `now`.
    pub fn observe(&mut self, holds_anything: bool, now: u64) -> IdleVerdict {
        match self {
            Self::AlwaysOn => IdleVerdict::Holding,
            Self::StopWhenIdle(watch) => watch.observe(holds_anything, now),
        }
    }
}

#[cfg(test)]
#[path = "authority_idle_tests.rs"]
mod tests;

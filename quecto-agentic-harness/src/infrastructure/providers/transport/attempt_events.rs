//! The stream events attempt diagnostics know (#2158): an event outside a
//! wire's [`Vocabulary`] counts as unknown, so a vendor's new or renamed
//! event shows up in the diagnostics instead of hiding among normal
//! traffic. Each wire names only its own events (#2597): the lists live
//! beside the vendors' dialects.
use crate::domain::inference::value_objects::attempt_diagnostics::TerminalEvent;

/// The events one wire names.
pub(in crate::infrastructure::providers) struct Vocabulary {
    /// Every non-terminal event the wire's stream handler consumes, or that
    /// the wire sends as ordinary traffic.
    pub known: &'static [&'static str],
    /// The wire's terminal events, each with what it records.
    pub terminal: &'static [(&'static str, TerminalEvent)],
}

impl Vocabulary {
    /// The type an attempt's event counts file an event named `named` under
    /// (#2433): its name when the wire knows it, and `(unknown)` otherwise,
    /// so a record never holds a name a provider made up.
    pub(in crate::infrastructure::providers) fn kind(&self, named: &str) -> &'static str {
        self.name(named).unwrap_or(UNKNOWN_KIND)
    }

    /// The wire's own spelling of `named`, known or terminal, if it has one.
    pub(in crate::infrastructure::providers) fn name(&self, named: &str) -> Option<&'static str> {
        self.known
            .iter()
            .copied()
            .chain(self.terminal.iter().map(|(name, _)| *name))
            .find(|known| *known == named)
    }

    /// What the terminal event `named` records, if `named` is one.
    pub(in crate::infrastructure::providers) fn terminal(
        &self,
        named: &str,
    ) -> Option<TerminalEvent> {
        self.terminal
            .iter()
            .find(|(name, _)| *name == named)
            .map(|(_, event)| *event)
    }
}

/// The type of an event the harness does not know.
pub(in crate::infrastructure::providers) const UNKNOWN_KIND: &str = "(unknown)";

#[cfg(test)]
#[path = "attempt_events_tests.rs"]
mod tests;

//! The closed vocabularies of a task; any other spelling is refused.

/// An enum with its exact spellings: `parse` admits only those.
macro_rules! vocabulary {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            pub fn parse(text: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|word| word.as_str() == text)
            }
        }
    };
}

vocabulary!(TaskKind { Epic => "epic", Task => "task", Chore => "chore" });

vocabulary!(TaskStatus {
    Draft => "draft",
    Ready => "ready",
    Claimed => "claimed",
    InProgress => "in_progress",
    Review => "review",
    Blocked => "blocked",
    Done => "done",
    Archived => "archived",
});

impl TaskStatus {
    /// Claimed and in-progress tasks always have a holder.
    pub fn requires_claim(self) -> bool {
        matches!(self, Self::Claimed | Self::InProgress)
    }

    /// A task under review or blocked may keep its holder's claim.
    pub fn may_hold_claim(self) -> bool {
        self.requires_claim() || matches!(self, Self::Review | Self::Blocked)
    }
}

vocabulary!(
    /// Where one item of a task's own board stands.
    ItemState { Todo => "todo", Doing => "doing", Review => "review", Done => "done", Blocked => "blocked" }
);

vocabulary!(
    /// How hard a role's model reasons: the board's own scale.
    Effort { Low => "low", Medium => "medium", High => "high", Max => "max" }
);

vocabulary!(
    /// How a run ended; `lost` is a run whose host died.
    RunOutcome { Succeeded => "succeeded", Failed => "failed", Stopped => "stopped", Lost => "lost" }
);

vocabulary!(Severity { Low => "low", Medium => "medium", High => "high", Critical => "critical" });

vocabulary!(
    /// What became of a review finding.
    Verdict { Open => "open", Fixed => "fixed", Invalid => "invalid", WontFix => "wont_fix" }
);

#[cfg(test)]
#[path = "vocabulary_tests.rs"]
mod tests;

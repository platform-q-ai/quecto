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

    /// A dependency in this status no longer holds its dependents back:
    /// done, or archived (finished or cancelled, either way over).
    pub fn releases_dependents(self) -> bool {
        matches!(self, Self::Done | Self::Archived)
    }
}

vocabulary!(
    /// Where a version of a task's plan stands: a draft is edited, an
    /// accepted one is the task's plan (never edited again), a superseded
    /// one was replaced by a newer acceptance, and a discarded draft is kept
    /// for history.
    PlanState { Draft => "draft", Accepted => "accepted", Superseded => "superseded", Discarded => "discarded" }
);

vocabulary!(
    /// What a review round concluded of the change at its commit.
    ReviewVerdict { Approved => "approved", ChangesRequested => "changes_requested" }
);

vocabulary!(
    /// Where one item of a run's board stands.
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

vocabulary!(
    /// How a done or archived task was resolved (the host's words for a
    /// closed issue, and `superseded`).
    Resolution { Completed => "completed", NotPlanned => "not_planned", Duplicate => "duplicate", Superseded => "superseded" }
);

vocabulary!(
    /// Where a discussion stands: open until someone closes it.
    DiscussionState { Open => "open", Closed => "closed" }
);

vocabulary!(
    /// What a full discussion has no more room for.
    DiscussionLimit { Messages => "messages", Lines => "lines", Text => "text" }
);

vocabulary!(Severity { Low => "low", Medium => "medium", High => "high", Critical => "critical" });

vocabulary!(
    /// What became of a review finding.
    Verdict { Open => "open", Fixed => "fixed", Invalid => "invalid", WontFix => "wont_fix" }
);

#[cfg(test)]
#[path = "vocabulary_tests.rs"]
mod tests;

//! Submitted-input history for the text-input system.
//!
//! Owns the history buffer, navigation index, and in-progress draft snapshot
//! used while the user walks Up/Down through prior submits. Callers must not
//! maintain parallel history — all mutation goes through [`InputHistory`].

/// Maximum retained submitted entries (oldest evicted first).
const MAX_HISTORY: usize = 500;

/// Result of a history Down step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HistoryDown {
    /// Load this prior entry into the draft.
    Entry(String),
    /// Left history navigation; restore the in-progress draft saved on first Up.
    RestoreDraft(String),
}

/// Submitted-input history with Up/Down navigation and draft save/restore.
///
/// Invariants (parity contract #1277):
/// - Empty pushes are ignored.
/// - Consecutive duplicate of the last entry is skipped.
/// - Length is capped at [`MAX_HISTORY`]; overflow drops index 0.
/// - First Up from live editing saves the current draft; Down past newest
///   restores that draft and leaves navigation.
#[derive(Debug, Default)]
pub(super) struct InputHistory {
    entries: Vec<String>,
    /// Position while navigating (`-1` = editing live draft, not in history).
    index: isize,
    /// Draft text captured on the first Up from live editing.
    saved_draft: String,
}

impl InputHistory {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: -1,
            saved_draft: String::new(),
        }
    }

    /// Record a submitted line. Ignores empty; skips duplicate of last entry;
    /// caps at [`MAX_HISTORY`]; resets navigation index.
    pub(super) fn push(&mut self, text: &str) {
        if text.is_empty() {
            self.index = -1;
            return;
        }
        if self.entries.last().map(|s| s.as_str()) != Some(text) {
            self.entries.push(text.to_string());
            if self.entries.len() > MAX_HISTORY {
                self.entries.remove(0);
            }
        }
        self.index = -1;
    }

    /// Move toward older entries. On first Up from live editing, `current_draft`
    /// is saved for later Down-restore. Returns the entry to load, if any.
    pub(super) fn navigate_up(&mut self, current_draft: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        if self.index < 0 {
            self.saved_draft = current_draft.to_string();
            self.index = self.entries.len() as isize - 1;
        } else if self.index > 0 {
            self.index -= 1;
        } else {
            return None;
        }
        Some(self.entries[self.index as usize].clone())
    }

    /// Move toward newer entries. Past newest restores the saved draft.
    pub(super) fn navigate_down(&mut self) -> Option<HistoryDown> {
        if self.index < 0 {
            return None;
        }
        if (self.index as usize) < self.entries.len() - 1 {
            self.index += 1;
            Some(HistoryDown::Entry(
                self.entries[self.index as usize].clone(),
            ))
        } else {
            self.index = -1;
            Some(HistoryDown::RestoreDraft(self.saved_draft.clone()))
        }
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;

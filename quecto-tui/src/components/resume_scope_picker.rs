//! Scope-aware `/resume` picker presentation state (#2001 D5).
//!
//! Pure UI decision state: Local|Global scope control, focus ring, metadata
//! query, cross-folder / missing-home disposition dialogs. No filesystem or
//! agent I/O — shell maps rows in and dispositions out.

use crate::shell::keys::Key;

/// Which catalogue scope the picker is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerScopeMode {
    Local,
    Global,
}

impl PickerScopeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Global => "global",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Local => Self::Global,
            Self::Global => Self::Local,
        }
    }
}

/// Focus targets inside the picker (Tab / Shift+Tab ring).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerFocus {
    ScopeControl,
    Query,
    Results,
    DispositionDialog,
}

impl PickerFocus {
    fn next(self, dialog_open: bool) -> Self {
        match self {
            Self::ScopeControl => Self::Query,
            Self::Query => Self::Results,
            Self::Results if dialog_open => Self::DispositionDialog,
            Self::Results => Self::ScopeControl,
            Self::DispositionDialog => Self::ScopeControl,
        }
    }

    fn prev(self, dialog_open: bool) -> Self {
        match self {
            Self::ScopeControl if dialog_open => Self::DispositionDialog,
            Self::ScopeControl => Self::Results,
            Self::Query => Self::ScopeControl,
            Self::Results => Self::Query,
            Self::DispositionDialog => Self::Results,
        }
    }
}

/// UI-facing disposition choices (wire names align with domain ResumeDisposition).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDispositionUi {
    SameScope,
    OpenOriginal,
    ForkCurrent,
    Locate,
    Cancel,
}

impl ResumeDispositionUi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameScope => "same_scope",
            Self::OpenOriginal => "open_original",
            Self::ForkCurrent => "fork_current",
            Self::Locate => "locate",
            Self::Cancel => "cancel",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SameScope => "Resume here",
            Self::OpenOriginal => "Open original",
            Self::ForkCurrent => "Fork here",
            Self::Locate => "Locate / reassociate",
            Self::Cancel => "Cancel",
        }
    }
}

/// One metadata row for the picker (title/key/repo/path — never body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePickerRow {
    pub key: String,
    pub title: String,
    pub repository_label: Option<String>,
    pub execution_path: Option<String>,
    pub is_legacy_unscoped: bool,
    /// True when selected home differs from current execution scope.
    pub cross_folder: bool,
    /// True when recorded home path is missing/unreachable.
    pub home_missing: bool,
}

impl ResumePickerRow {
    pub fn metadata_haystack(&self) -> String {
        let mut parts = vec![self.key.clone(), self.title.clone()];
        if let Some(r) = &self.repository_label {
            parts.push(r.clone());
        }
        if let Some(p) = &self.execution_path {
            parts.push(p.clone());
        }
        parts.join(" ").to_ascii_lowercase()
    }
}

/// Disposition choices shown in the secondary dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispositionDialogKind {
    CrossFolder,
    MissingHome,
}

impl DispositionDialogKind {
    pub fn choices(self) -> &'static [ResumeDispositionUi] {
        match self {
            Self::CrossFolder => &[
                ResumeDispositionUi::OpenOriginal,
                ResumeDispositionUi::ForkCurrent,
                ResumeDispositionUi::Cancel,
            ],
            Self::MissingHome => &[
                ResumeDispositionUi::Locate,
                ResumeDispositionUi::ForkCurrent,
                ResumeDispositionUi::Cancel,
            ],
        }
    }
}

/// Result of handling a key while the picker is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    Pending,
    /// User selected a same-scope session key to resume in place.
    ResumeSameScope { key: String },
    /// User confirmed a disposition for a cross-folder or missing-home row.
    Disposition {
        key: String,
        disposition: ResumeDispositionUi,
    },
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DispositionDialogState {
    kind: DispositionDialogKind,
    key: String,
    choice_idx: usize,
}

/// Presentation state for the scope-aware resume picker.
#[derive(Debug, Clone)]
pub struct ResumeScopePicker {
    scope: PickerScopeMode,
    focus: PickerFocus,
    query: String,
    rows: Vec<ResumePickerRow>,
    selected: usize,
    dialog: Option<DispositionDialogState>,
    all_rows: Vec<ResumePickerRow>,
}

impl ResumeScopePicker {
    /// Open picker with Local default (issue contract).
    pub fn open_local(rows: Vec<ResumePickerRow>) -> Self {
        let mut s = Self {
            scope: PickerScopeMode::Local,
            focus: PickerFocus::Results,
            query: String::new(),
            all_rows: rows,
            rows: Vec::new(),
            selected: 0,
            dialog: None,
        };
        s.refilter();
        s
    }

    pub fn scope_mode(&self) -> PickerScopeMode {
        self.scope
    }

    pub fn focus(&self) -> PickerFocus {
        self.focus
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn visible_rows(&self) -> &[ResumePickerRow] {
        &self.rows
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    pub fn dialog_kind(&self) -> Option<DispositionDialogKind> {
        self.dialog.as_ref().map(|d| d.kind)
    }

    pub fn dialog_choices(&self) -> Option<&'static [ResumeDispositionUi]> {
        self.dialog.as_ref().map(|d| d.kind.choices())
    }

    pub fn dialog_choice_index(&self) -> Option<usize> {
        self.dialog.as_ref().map(|d| d.choice_idx)
    }

    /// Scope control labels for rendering (`local_selected` true when Local on).
    pub fn scope_control_labels(&self) -> (&'static str, &'static str, bool) {
        ("Local", "Global", self.scope == PickerScopeMode::Local)
    }

    pub fn set_rows(&mut self, rows: Vec<ResumePickerRow>) {
        self.all_rows = rows;
        self.refilter();
    }

    fn refilter(&mut self) {
        let q = self.query.trim().to_ascii_lowercase();
        self.rows = self
            .all_rows
            .iter()
            .filter(|r| match self.scope {
                PickerScopeMode::Local => {
                    if r.is_legacy_unscoped {
                        return false;
                    }
                    q.is_empty() || r.metadata_haystack().contains(&q)
                }
                PickerScopeMode::Global => q.is_empty() || r.metadata_haystack().contains(&q),
            })
            .cloned()
            .collect();
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
    }

    pub fn handle_key(&mut self, key: &Key) -> PickerAction {
        if matches!(key, Key::Escape) {
            if self.dialog.take().is_some() {
                self.focus = PickerFocus::Results;
                return PickerAction::Pending;
            }
            return PickerAction::Dismissed;
        }

        if self.dialog.is_some() {
            return self.handle_dialog_key(key);
        }

        match key {
            Key::Tab => {
                self.focus = self.focus.next(false);
                PickerAction::Pending
            }
            Key::BackTab => {
                self.focus = self.focus.prev(false);
                PickerAction::Pending
            }
            Key::Char(' ') | Key::Enter if self.focus == PickerFocus::ScopeControl => {
                self.toggle_scope();
                PickerAction::Pending
            }
            Key::Left | Key::Right if self.focus == PickerFocus::ScopeControl => {
                self.toggle_scope();
                PickerAction::Pending
            }
            Key::Char(c) if self.focus == PickerFocus::Query && !c.is_control() => {
                self.query.push(*c);
                self.refilter();
                PickerAction::Pending
            }
            Key::Backspace if self.focus == PickerFocus::Query => {
                self.query.pop();
                self.refilter();
                PickerAction::Pending
            }
            Key::Up if self.focus == PickerFocus::Results => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                PickerAction::Pending
            }
            Key::Down if self.focus == PickerFocus::Results => {
                if !self.rows.is_empty() && self.selected + 1 < self.rows.len() {
                    self.selected += 1;
                }
                PickerAction::Pending
            }
            Key::Enter | Key::Char(' ') if self.focus == PickerFocus::Results => self.activate_row(),
            _ => PickerAction::Pending,
        }
    }

    /// Mouse click on the Local|Global control.
    pub fn mouse_click_scope(&mut self, want_global: bool) {
        let target = if want_global {
            PickerScopeMode::Global
        } else {
            PickerScopeMode::Local
        };
        if self.scope != target {
            self.scope = target;
            self.refilter();
        }
        self.focus = PickerFocus::ScopeControl;
    }

    /// Mouse click on a result row index.
    pub fn mouse_click_row(&mut self, index: usize) -> PickerAction {
        if index >= self.rows.len() {
            return PickerAction::Pending;
        }
        self.selected = index;
        self.focus = PickerFocus::Results;
        self.activate_row()
    }

    fn toggle_scope(&mut self) {
        self.scope = self.scope.toggle();
        self.refilter();
    }

    fn activate_row(&mut self) -> PickerAction {
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return PickerAction::Pending;
        };
        if row.home_missing {
            self.dialog = Some(DispositionDialogState {
                kind: DispositionDialogKind::MissingHome,
                key: row.key,
                choice_idx: 0,
            });
            self.focus = PickerFocus::DispositionDialog;
            return PickerAction::Pending;
        }
        if row.cross_folder {
            self.dialog = Some(DispositionDialogState {
                kind: DispositionDialogKind::CrossFolder,
                key: row.key,
                choice_idx: 0,
            });
            self.focus = PickerFocus::DispositionDialog;
            return PickerAction::Pending;
        }
        PickerAction::ResumeSameScope { key: row.key }
    }

    fn handle_dialog_key(&mut self, key: &Key) -> PickerAction {
        let Some(dialog) = self.dialog.as_mut() else {
            return PickerAction::Pending;
        };
        let n = dialog.kind.choices().len();
        match key {
            Key::Up | Key::Left => {
                if dialog.choice_idx > 0 {
                    dialog.choice_idx -= 1;
                }
                PickerAction::Pending
            }
            Key::Down | Key::Right => {
                if dialog.choice_idx + 1 < n {
                    dialog.choice_idx += 1;
                }
                PickerAction::Pending
            }
            Key::Enter | Key::Char(' ') => {
                let disposition = dialog.kind.choices()[dialog.choice_idx];
                let key_s = dialog.key.clone();
                self.dialog = None;
                self.focus = PickerFocus::Results;
                if matches!(disposition, ResumeDispositionUi::Cancel) {
                    PickerAction::Pending
                } else {
                    PickerAction::Disposition {
                        key: key_s,
                        disposition,
                    }
                }
            }
            _ => PickerAction::Pending,
        }
    }

    /// Render helper line for tests / simple paint.
    pub fn render_scope_control_line(&self) -> String {
        let (local, global, local_on) = self.scope_control_labels();
        let focus_mark = if self.focus == PickerFocus::ScopeControl {
            "*"
        } else {
            " "
        };
        if local_on {
            format!("{focus_mark}[{local}] {global}")
        } else {
            format!("{focus_mark}{local} [{global}]")
        }
    }
}

#[cfg(test)]
#[path = "resume_scope_picker_tests.rs"]
mod tests;

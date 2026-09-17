//! Session discovery presentation: modal-local focus, safe rows and stable IDs.
use crate::components::{
    component::Component,
    fuzzy::fuzzy_filter,
    list_rows::SelectionStyle,
    select_list::{SelectItem, SelectList, SelectResult},
    select_overlay::build_select_overlay,
    theme,
    utils::sanitize_truncate_chars_with_ellipsis,
};
use crate::protocol::session_payloads::SessionListScope;
use crate::shell::keys::Key;

#[derive(Debug, PartialEq, Eq)]
pub enum ResumePickerEvent {
    ScopeChanged(SessionListScope),
    Selected(String),
    Dismissed,
    Pending,
}
/// The three focusable sections, in Tab order after `Results`. The user-facing
/// names (`Sessions`, `Local/Global`, `Search`) live in [`Focus::label`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Scope,
    Query,
    Results,
}
impl Focus {
    fn label(self) -> &'static str {
        match self {
            Focus::Results => "Sessions",
            Focus::Scope => "Local/Global",
            Focus::Query => "Search",
        }
    }
}
/// Content-relative columns of the scope switch's two labels, covering both
/// renderings (`Scope: [Local]  Global` and `Scope:  Local  [Global]`) after the
/// two-cell focus marker.
const LOCAL_COLS: std::ops::RangeInclusive<usize> = 9..=15;
const GLOBAL_COLS: std::ops::RangeInclusive<usize> = 17..=24;
/// Content rows above the first result: title, scope, search, `Sessions`.
const HEADER_ROWS: usize = 4;
/// Where a section marker goes: `▸ ` on the focused section, blank elsewhere.
fn marker(focused: bool) -> String {
    if focused {
        theme::accent("▸ ")
    } else {
        "  ".into()
    }
}
#[derive(Default)]
struct HitLayout {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    border: usize,
}
pub struct ResumePicker {
    list: SelectList,
    items: Vec<SelectItem>,
    scope: SessionListScope,
    focus: Focus,
    query: String,
    hits: HitLayout,
    result_rows: usize,
}
impl Default for ResumePicker {
    fn default() -> Self {
        Self::new(Vec::new(), SessionListScope::Local)
    }
}
impl ResumePicker {
    pub fn new(items: Vec<SelectItem>, scope: SessionListScope) -> Self {
        let mut picker = Self {
            list: SelectList::new(Vec::new(), 12),
            items: Vec::new(),
            scope,
            focus: Focus::Results,
            query: String::new(),
            hits: HitLayout::default(),
            result_rows: 12,
        };
        picker.sync_items(items);
        picker
    }
    pub fn sync_items(&mut self, items: Vec<SelectItem>) {
        self.items = items
            .into_iter()
            .map(|mut item| {
                item.label = safe(&item.label);
                item.description = item.description.map(|s| safe(&s));
                item
            })
            .collect();
        self.filter();
    }
    fn filter(&mut self) {
        let items: Vec<_> = fuzzy_filter(&self.items, &self.query, |i| &i.label)
            .into_iter()
            .cloned()
            .collect();
        self.list.sync_items(items);
    }
    pub fn item_count(&self) -> usize {
        self.list.item_count()
    }
    pub fn render_text(&mut self, width: usize) -> String {
        self.list.render(width).join("\n")
    }
    pub fn selected_item(&self) -> Option<&SelectItem> {
        self.list.selected_item()
    }
    #[cfg(any(test, feature = "test-harness"))]
    pub fn items_for_tests(&self) -> &[SelectItem] {
        self.list.items_for_tests()
    }
    fn change_scope(&mut self, scope: SessionListScope) -> ResumePickerEvent {
        if self.scope == scope {
            return ResumePickerEvent::Pending;
        }
        self.scope = scope;
        self.sync_items(Vec::new()); // Old-scope rows are never actionable while refreshing.
        ResumePickerEvent::ScopeChanged(scope)
    }
    fn toggle_scope(&mut self) -> ResumePickerEvent {
        self.change_scope(match self.scope {
            SessionListScope::Local => SessionListScope::Global,
            SessionListScope::Global => SessionListScope::Local,
        })
    }
    pub fn handle_key(&mut self, key: &Key) -> ResumePickerEvent {
        self.handle_input(key)
    }
    pub fn handle_input(&mut self, key: &Key) -> ResumePickerEvent {
        match key {
            Key::Escape => return ResumePickerEvent::Dismissed,
            Key::Tab => {
                self.focus = match self.focus {
                    Focus::Scope => Focus::Query,
                    Focus::Query => Focus::Results,
                    Focus::Results => Focus::Scope,
                }
            }
            Key::BackTab => {
                self.focus = match self.focus {
                    Focus::Scope => Focus::Results,
                    Focus::Query => Focus::Scope,
                    Focus::Results => Focus::Query,
                }
            }
            Key::MousePress(x, y) => return self.mouse(*x as usize, *y as usize),
            _ => return self.focused_input(key),
        }
        ResumePickerEvent::Pending
    }
    fn focused_input(&mut self, key: &Key) -> ResumePickerEvent {
        match (self.focus, key) {
            (Focus::Scope, Key::Enter | Key::Char(' ')) => return self.toggle_scope(),
            (Focus::Scope, Key::Left) => return self.change_scope(SessionListScope::Local),
            (Focus::Scope, Key::Right) => return self.change_scope(SessionListScope::Global),
            (Focus::Query, Key::Char(c))
                if (c.is_alphanumeric() || c.is_ascii_punctuation() || *c == ' ')
                    && self.query.chars().count() < 64 =>
            {
                self.query.push(*c);
                self.filter();
            }
            (Focus::Query, Key::Backspace) => {
                self.query.pop();
                self.filter();
            }
            (Focus::Query, Key::Enter) => self.focus = Focus::Results,
            (Focus::Results, key) if self.result_rows > 0 => {
                let key = match key {
                    Key::Char(' ') => &Key::Enter,
                    Key::ScrollDown => &Key::Down,
                    Key::ScrollUp => &Key::Up,
                    key => key,
                };
                self.list.handle_input(key);
                if let SelectResult::Selected(value) = self.list.take_result() {
                    return ResumePickerEvent::Selected(value);
                }
            }
            _ => {}
        }
        ResumePickerEvent::Pending
    }
    fn mouse(&mut self, x: usize, y: usize) -> ResumePickerEvent {
        let h = &self.hits;
        if (h.left..h.left + h.width).contains(&x) && (h.top..h.top + h.height).contains(&y) {
            let row = y - h.top;
            let col = x - h.left;
            if row == h.border + 1 && col >= h.border * 2 {
                self.focus = Focus::Scope;
                let col = col - h.border * 2;
                return if LOCAL_COLS.contains(&col) {
                    self.change_scope(SessionListScope::Local)
                } else if GLOBAL_COLS.contains(&col) {
                    self.change_scope(SessionListScope::Global)
                } else {
                    ResumePickerEvent::Pending
                };
            }
            if row == h.border + 2 {
                self.focus = Focus::Query;
            }
            if row == h.border + 3 {
                self.focus = Focus::Results;
            }
            if row >= h.border + HEADER_ROWS {
                let offset = row - (h.border + HEADER_ROWS);
                if offset < self.result_rows
                    && let Some(id) = self
                        .list
                        .visible_item(offset)
                        .map(|item| item.value.clone())
                {
                    self.focus = Focus::Results;
                    for _ in 0..self.list.item_count() {
                        if self.list.selected_item().is_some_and(|s| s.value == id) {
                            return ResumePickerEvent::Selected(id);
                        }
                        self.list.handle_input(&Key::Down);
                    }
                }
            }
        }
        ResumePickerEvent::Pending
    }
    pub fn render_overlay(&mut self, width: usize, height: usize) -> (Vec<String>, usize) {
        self.render(width, height)
    }
    pub fn render(&mut self, width: usize, height: usize) -> (Vec<String>, usize) {
        let scope = match self.scope {
            SessionListScope::Local => "Scope: [Local]  Global",
            SessionListScope::Global => "Scope:  Local  [Global]",
        };
        let focus = self.focus;
        let cursor = if focus == Focus::Query { "▏" } else { "" };
        let footer = [Focus::Results, Focus::Scope, Focus::Query]
            .map(|f| {
                if f == focus {
                    theme::accent(f.label())
                } else {
                    f.label().to_string()
                }
            })
            .join(" ▸ ");
        self.list.set_selection_style(if focus == Focus::Results {
            SelectionStyle::Active
        } else {
            SelectionStyle::Inactive
        });
        let (lines, panel_width) = build_select_overlay(width, height, |content_width| {
            let mut lines = vec![
                "Resume session".into(),
                format!("{}{scope}", marker(focus == Focus::Scope)),
                format!(
                    "{}Search: {}{cursor}",
                    marker(focus == Focus::Query),
                    self.query
                ),
                format!("{}Sessions", marker(focus == Focus::Results)),
            ];
            // Budget before rendering: outer margins, borders, the header rows,
            // footer, overflow indicator and wrapped selected details all consume rows.
            let border_rows = if width.saturating_sub(4) >= 6 { 2 } else { 0 };
            let budget = height.saturating_sub(4 + border_rows + HEADER_ROWS + 1);
            let details = self
                .list
                .selected_item()
                .and_then(|item| item.description.as_deref())
                .map(|description| crate::components::utils::wrap_text(description, content_width))
                .unwrap_or_default();
            let (result_rows, detail_rows, indicator) =
                fit_result_window(budget, self.list.item_count(), details.len());
            self.result_rows = result_rows;
            self.list.set_max_visible(self.result_rows);
            if self.result_rows > 0 {
                if let Some(selected) = self.list.selected_item().map(|item| item.value.clone()) {
                    debug_assert!(
                        (0..self.result_rows).any(|offset| {
                            self.list
                                .visible_item(offset)
                                .is_some_and(|item| item.value == selected)
                        }),
                        "selected identity must stay inside the fitted result window"
                    );
                }
                let mut rendered = self.list.render(content_width);
                // Same allocation used for scrolling and hit-testing: never let
                // the overflow indicator steal a row the budget did not reserve.
                rendered.truncate(self.result_rows + indicator);
                lines.extend(rendered);
            }
            lines.extend(details.into_iter().take(detail_rows));
            lines.push(format!("Tab: {footer} · ↑↓ · Enter open · Esc close"));
            lines
        });
        let visible_height = lines.len().min(height.saturating_sub(4));
        self.hits = HitLayout {
            left: width.saturating_sub(panel_width) / 2,
            top: height.saturating_sub(visible_height) / 2,
            width: panel_width,
            height: visible_height,
            border: usize::from(panel_width >= 6),
        };
        (lines, panel_width)
    }
}
fn safe(text: &str) -> String {
    sanitize_truncate_chars_with_ellipsis(text, 512, "…")
}

/// Fit results into the modal body after scope/query/details/footer chrome.
///
/// `budget` is rows remaining inside the overlay content area. The overflow
/// indicator is reserved only when the window cannot show every item *and*
/// a spare row exists (or can be stolen without dropping the last result).
fn fit_result_window(budget: usize, item_count: usize, detail_len: usize) -> (usize, usize, usize) {
    if budget == 0 {
        return (0, 0, 0);
    }
    let detail_rows = detail_len.min(budget.saturating_sub(1));
    let leftover = budget - detail_rows;
    let mut result_rows = leftover.min(12);
    let mut indicator = 0;
    if result_rows > 0 && item_count > result_rows {
        if leftover > result_rows {
            indicator = 1;
        } else if result_rows >= 2 {
            result_rows -= 1;
            indicator = 1;
        }
    }
    debug_assert!(result_rows + indicator + detail_rows <= budget);
    (result_rows, detail_rows, indicator)
}

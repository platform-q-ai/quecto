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
/// names (`Sessions`, `Scope`, `Search`) live in [`Focus::label`].
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
            Focus::Scope => "Scope",
            Focus::Query => "Search",
        }
    }
}
/// Scope switch text; the bracketed option is the active one. The labels are
/// presentation only — the wire scope stays `local` / `global`.
const SCOPE_LOCAL_FOLDER: &str = "Scope:   [Local Folder]   All Folders";
const SCOPE_ALL_FOLDERS: &str = "Scope:    Local Folder   [All Folders]";
/// Content-relative columns of the scope switch's two labels (brackets
/// included) in both renderings, counted from after the two-cell focus marker.
const LOCAL_FOLDER_COLS: std::ops::RangeInclusive<usize> = 11..=24;
const ALL_FOLDERS_COLS: std::ops::RangeInclusive<usize> = 27..=39;
/// Content rows above the first result: title, blank, scope, search, blank,
/// `Sessions`. Very short terminals drop the two blank rows ([`Header`]).
const HEADER_ROWS: usize = 6;
/// List rows sit two cells deeper than the `Sessions` header's label.
const LIST_INDENT: &str = "    ";
/// The block indent of the details and footer rows.
const BLOCK_INDENT: &str = "  ";
/// Row positions of the header sections; `compact` drops the blank rows so a
/// very short terminal still shows results.
#[derive(Clone, Copy, Default)]
struct Header {
    compact: bool,
}
impl Header {
    fn rows(self) -> usize {
        HEADER_ROWS - 2 * usize::from(self.compact)
    }
    fn scope(self) -> usize {
        2 - usize::from(self.compact)
    }
    fn search(self) -> usize {
        self.scope() + 1
    }
    fn sessions(self) -> usize {
        self.rows() - 1
    }
}
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
    header: Header,
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
            if row == h.border + h.header.scope() && col >= h.border * 2 {
                self.focus = Focus::Scope;
                let col = col - h.border * 2;
                return if LOCAL_FOLDER_COLS.contains(&col) {
                    self.change_scope(SessionListScope::Local)
                } else if ALL_FOLDERS_COLS.contains(&col) {
                    self.change_scope(SessionListScope::Global)
                } else {
                    ResumePickerEvent::Pending
                };
            }
            if row == h.border + h.header.search() {
                self.focus = Focus::Query;
            }
            if row == h.border + h.header.sessions() {
                self.focus = Focus::Results;
            }
            if row >= h.border + h.header.rows() {
                let offset = row - (h.border + h.header.rows());
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
            SessionListScope::Local => SCOPE_LOCAL_FOLDER,
            SessionListScope::Global => SCOPE_ALL_FOLDERS,
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
        // Budget before rendering: outer margins, borders, the header rows,
        // footer, overflow indicator, wrapped selected details and the blank
        // separator rows all consume rows. When the full header leaves fewer
        // than two body rows, the header's blank rows go before any result does.
        let border_rows = if width.saturating_sub(4) >= 6 { 2 } else { 0 };
        let chrome = 4 + border_rows + 1;
        let header = Header {
            compact: height.saturating_sub(chrome + HEADER_ROWS) < 2,
        };
        let budget = height.saturating_sub(chrome + header.rows());
        let (lines, panel_width) = build_select_overlay(width, height, |content_width| {
            let blank = || (!header.compact).then(String::new);
            let mut lines: Vec<String> = [
                Some("Resume session".into()),
                blank(),
                Some(format!("{}{scope}", marker(focus == Focus::Scope))),
                Some(format!(
                    "{}Search:  {}{cursor}",
                    marker(focus == Focus::Query),
                    self.query
                )),
                blank(),
                Some(format!("{}Sessions", marker(focus == Focus::Results))),
            ]
            .into_iter()
            .flatten()
            .collect();
            debug_assert_eq!(lines.len(), header.rows());
            let detail_width = content_width.saturating_sub(BLOCK_INDENT.len()).max(1);
            let details = self
                .list
                .selected_item()
                .and_then(|item| item.description.as_deref())
                .map(|description| crate::components::utils::wrap_text(description, detail_width))
                .unwrap_or_default();
            let fit = fit_result_window(budget, self.list.item_count(), details.len());
            self.result_rows = fit.result_rows;
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
                let list_width = content_width.saturating_sub(LIST_INDENT.len()).max(1);
                let mut rendered = self.list.render(list_width);
                // Same allocation used for scrolling and hit-testing: never let
                // the overflow indicator steal a row the budget did not reserve.
                rendered.truncate(self.result_rows + fit.indicator);
                lines.extend(rendered.iter().map(|row| format!("{LIST_INDENT}{row}")));
            }
            lines.extend(std::iter::repeat_n(String::new(), fit.gap_before_details));
            lines.extend(
                details
                    .into_iter()
                    .take(fit.detail_rows)
                    .map(|row| format!("{BLOCK_INDENT}{row}")),
            );
            lines.extend(std::iter::repeat_n(String::new(), fit.gap_before_footer));
            // The full footer is 62 cells (two-cell indent included); narrower
            // panels drop the key hints before the section names so the focus
            // cue always survives.
            lines.push(if content_width >= 62 {
                format!("{BLOCK_INDENT}Tab: {footer} · ↑↓ · Enter open · Esc close")
            } else {
                format!("{BLOCK_INDENT}Tab: {footer}")
            });
            lines
        });
        let visible_height = lines.len().min(height.saturating_sub(4));
        self.hits = HitLayout {
            left: width.saturating_sub(panel_width) / 2,
            top: height.saturating_sub(visible_height) / 2,
            width: panel_width,
            height: visible_height,
            border: usize::from(panel_width >= 6),
            header,
        };
        (lines, panel_width)
    }
}
fn safe(text: &str) -> String {
    sanitize_truncate_chars_with_ellipsis(text, 512, "…")
}

/// Row allocation of the modal body below the `Sessions` header.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ResultWindow {
    pub(super) result_rows: usize,
    pub(super) indicator: usize,
    pub(super) detail_rows: usize,
    /// Blank row between the list and the selected-row details (0 or 1).
    pub(super) gap_before_details: usize,
    /// Blank row between the body and the footer (0 or 1).
    pub(super) gap_before_footer: usize,
}
/// Fit results into the modal body after scope/query/details/footer chrome.
///
/// `budget` is rows remaining inside the overlay content area. The overflow
/// indicator is reserved only when the window cannot show every item *and*
/// a spare row exists (or can be stolen without dropping the last result).
/// Blank separator rows only ever take rows nothing else claimed, so short
/// terminals lose the gaps before they lose a result: the details gap first
/// keeps the list and the details apart, then the footer gap.
pub(super) fn fit_result_window(
    budget: usize,
    item_count: usize,
    detail_len: usize,
) -> ResultWindow {
    if budget == 0 {
        return ResultWindow::default();
    }
    let detail_rows = detail_len.min(budget.saturating_sub(1));
    let leftover = budget - detail_rows;
    // A window never claims rows the list cannot fill (the empty placeholder
    // needs one), so the separator rows land right under the last row.
    let mut result_rows = leftover.min(12).min(item_count.max(1));
    let mut indicator = 0;
    if result_rows > 0 && item_count > result_rows {
        if leftover > result_rows {
            indicator = 1;
        } else if result_rows >= 2 {
            result_rows -= 1;
            indicator = 1;
        }
    }
    let mut spare = budget - (result_rows + indicator + detail_rows);
    let gap_before_details = usize::from(detail_rows > 0 && spare > 0);
    spare -= gap_before_details;
    let gap_before_footer = usize::from(spare > 0);
    let fit = ResultWindow {
        result_rows,
        indicator,
        detail_rows,
        gap_before_details,
        gap_before_footer,
    };
    debug_assert!(
        fit.result_rows
            + fit.indicator
            + fit.detail_rows
            + fit.gap_before_details
            + fit.gap_before_footer
            <= budget
    );
    fit
}

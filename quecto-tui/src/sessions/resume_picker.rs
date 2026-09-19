//! Session discovery presentation: modal-local focus, safe rows and stable IDs.
//! The rows are the harness's answer, shown as given: the search box reports
//! its text ([`ResumePickerEvent::QueryChanged`]) and filters nothing itself.
use crate::components::{
    component::Component,
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
    /// The search text was edited (#2010); the rows stay until an answer replaces them.
    QueryChanged(String),
    Selected(String),
    Dismissed,
    Pending,
}
/// What the rows on screen are worth for the text in the box and the scope
/// on screen. Only `Settled` rows may be acted on (R1-T1). While `Searching`
/// — a SEARCH of the text in the box is in the air — an Enter is owed to the
/// settled answer's top-ranked row; a listing (`Loading`) is owed nothing
/// (R2-T1: its top row is merely the newest session). `Stalled` and
/// `Disconnected` rows (nobody is asking) act on nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowsState {
    #[default]
    Settled,
    Loading,
    Searching,
    Stalled,
    Disconnected,
}
/// The search box holds any session key (R1-T11); the harness searches at
/// most this many visible characters, so nothing typed here is ever refused.
pub const QUERY_CAP: usize = 256;
const PASTE_REFUSED: &str = "Paste refused: longer than 256 characters";
const PASTE_CUT: &str = "Pasted the first line only";
const STALLED: &str = "Search did not answer — edit the text or change Scope to retry";
const DISCONNECTED: &str = "Disconnected — once reconnected, edit the text or change Scope";
/// The cue of an owed Enter (R2-T3), and its form for a narrow panel.
const OWED: &str = "Sessions · Searching… ⏎ will open the top match";
const OWED_SHORT: &str = "Sessions · ⏎ Searching…";
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
    scope: SessionListScope,
    focus: Focus,
    query: String,
    hits: HitLayout,
    result_rows: usize,
    rows_state: RowsState,
    /// An Enter typed while `Searching`, owed to the settled answer's top
    /// row. ANY later key or click withdraws it (R2-T3); a repeated Enter
    /// owes it again.
    pending_enter: bool,
    /// The user moved the cursor since the last query or scope edit.
    cursor_placed: bool,
    /// One status line under the rows: truncated, no match, refused, fallback.
    notice: Option<String>,
    /// What became of the last paste, until the next edit.
    paste_notice: Option<&'static str>,
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
            scope,
            focus: Focus::Results,
            query: String::new(),
            hits: HitLayout::default(),
            result_rows: 12,
            rows_state: RowsState::Settled,
            pending_enter: false,
            cursor_placed: false,
            notice: None,
            paste_notice: None,
        };
        picker.sync_items(items);
        picker
    }
    pub fn set_notice(&mut self, notice: Option<String>) {
        self.notice = notice.map(|text| safe(&text));
    }
    pub fn rows_state(&self) -> RowsState {
        self.rows_state
    }
    /// What the rows are worth now. Settling honours an Enter typed ahead of
    /// the answer: the value of the answer's top-ranked row, once — `None`
    /// when no Enter is owed or nothing matched.
    pub fn set_rows_state(&mut self, state: RowsState) -> Option<String> {
        self.rows_state = state;
        // Still searching: the Enter stays owed. Any other state withdraws it.
        let owed = self.pending_enter && state == RowsState::Settled;
        self.pending_enter &= state == RowsState::Searching;
        if !owed {
            return None;
        }
        // An Enter is owed only while the cursor is unplaced, and unplaced
        // rows were put on their top row by `sync_items`.
        self.list.selected_item().map(|item| item.value.clone())
    }
    /// The owed Enter is withdrawn by the shell: the search was not answered
    /// within its first flight, so a retry never pays it (R2-T3).
    pub fn withdraw_enter(&mut self) {
        self.pending_enter = false;
    }
    /// A query or scope edit: the rows now belong to an older question — a
    /// search of the text, or the listing an empty box asks for.
    fn unsettle(&mut self) {
        self.rows_state = if self.query.trim().is_empty() {
            RowsState::Loading
        } else {
            RowsState::Searching
        };
        (self.pending_enter, self.cursor_placed) = (false, false);
        (self.notice, self.paste_notice) = (None, None);
    }
    /// Replace the rows. An answer is ranked, so the cursor goes to its top
    /// row — unless the user placed it and that session is still there (R1-T8).
    pub fn sync_items(&mut self, items: Vec<SelectItem>) {
        let placed = self.list.selected_item().map(|item| item.value.clone());
        let placed = placed.filter(|_| self.cursor_placed);
        let items: Vec<_> = items
            .into_iter()
            .map(|mut item| {
                item.label = safe(&item.label);
                item.description = item.description.map(|s| safe(&s));
                item
            })
            .collect();
        self.list.sync_items(items);
        // Rows that replace "no rows" are actionable before the next render
        // re-fits the window (it always does, and only ever shrinks this).
        if self.result_rows == 0 {
            self.result_rows = self.list.item_count().min(1);
        }
        let kept = self.list.selected_item().map(|item| &item.value) == placed.as_ref();
        if !(kept && placed.is_some()) {
            self.list.select_first();
            self.cursor_placed = false;
        }
    }
    /// The search text as typed; what it matches is the harness's decision.
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn scope(&self) -> SessionListScope {
        self.scope
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
        self.unsettle();
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
        // Whatever the user does next withdraws an owed Enter (R2-T3): a
        // focus change, a click, an edit, a cursor move, Escape. Only the
        // Enter arm below owes one (again).
        self.pending_enter = false;
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
                if typeable(*c) && self.query.chars().count() < QUERY_CAP =>
            {
                self.query.push(*c);
                return self.query_changed();
            }
            // A paste is search text whichever section has focus (R2-T7).
            (_, Key::Paste(text)) => return self.paste(text),
            (Focus::Query, Key::Backspace) if self.query.pop().is_some() => {
                return self.query_changed();
            }
            (Focus::Query, Key::Enter) => self.focus = Focus::Results,
            (Focus::Results, Key::Enter | Key::Char(' '))
                if self.rows_state != RowsState::Settled =>
            {
                // Deferred, never acted on stale rows (R1-T1) — and only to a
                // SEARCH of text the user typed here, never to a listing
                // (R2-T1). A cursor the user placed on the stale rows points
                // at no settled row: nothing.
                self.pending_enter = self.rows_state == RowsState::Searching
                    && !self.query.trim().is_empty()
                    && !self.cursor_placed;
            }
            (Focus::Results, key) if self.result_rows > 0 => {
                let key = match key {
                    Key::Char(' ') => &Key::Enter,
                    Key::ScrollDown => &Key::Down,
                    Key::ScrollUp => &Key::Up,
                    key => key,
                };
                if self.list.handle_input(key) && *key != Key::Enter {
                    self.cursor_placed = true;
                }
                if let SelectResult::Selected(value) = self.list.take_result() {
                    return ResumePickerEvent::Selected(value);
                }
            }
            _ => {}
        }
        ResumePickerEvent::Pending
    }
    fn query_changed(&mut self) -> ResumePickerEvent {
        self.unsettle();
        ResumePickerEvent::QueryChanged(self.query.clone())
    }
    /// A paste is search text, whichever section has focus (R1-T11, R2-T7):
    /// its first non-empty line — terminals send `\r` between lines — through
    /// the typing filter, and the box takes the focus. More lines are dropped
    /// with a notice. One that does not fit is refused whole — a truncated
    /// key would search for something the user never meant.
    fn paste(&mut self, text: &str) -> ResumePickerEvent {
        let lines = text.split(['\n', '\r']).map(str::trim);
        let mut lines = lines.filter(|line| !line.is_empty());
        let first = lines.next().unwrap_or_default();
        let pasted: String = first.chars().filter(|c| typeable(*c)).collect();
        if pasted.is_empty() {
            return ResumePickerEvent::Pending;
        }
        self.focus = Focus::Query;
        if self.query.chars().count() + pasted.chars().count() > QUERY_CAP {
            self.paste_notice = Some(PASTE_REFUSED);
            return ResumePickerEvent::Pending;
        }
        self.query.push_str(&pasted);
        let event = self.query_changed();
        self.paste_notice = lines.next().map(|_| PASTE_CUT);
        event
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
                            return self.clicked(id);
                        }
                        self.list.handle_input(&Key::Down);
                    }
                }
            }
        }
        ResumePickerEvent::Pending
    }
    /// A click selects only a settled row (R1-T1); on any other it is a
    /// cursor placement, which also withdraws an Enter typed ahead.
    fn clicked(&mut self, id: String) -> ResumePickerEvent {
        if self.rows_state == RowsState::Settled {
            return ResumePickerEvent::Selected(id);
        }
        self.cursor_placed = true;
        ResumePickerEvent::Pending
    }
    /// `Sessions`, and what the rows under it are worth when not settled —
    /// short, so a narrow panel still tells the states apart (R2-T6); the
    /// advice is the notice line's ([`Self::status_line`]), which wraps.
    fn sessions_header(&self, width: usize) -> &'static str {
        match self.rows_state {
            RowsState::Settled => "Sessions",
            RowsState::Loading => "Sessions · Loading…",
            RowsState::Searching if !self.pending_enter => "Sessions · Searching…",
            RowsState::Searching if OWED.chars().count() <= width => OWED,
            RowsState::Searching => OWED_SHORT,
            RowsState::Stalled => "Sessions · No answer",
            RowsState::Disconnected => "Sessions · Disconnected",
        }
    }
    /// The one status line under the rows: why nothing is asking, else the
    /// answer's notice and what became of a paste.
    fn status_line(&self) -> Option<String> {
        match (self.rows_state, &self.notice, self.paste_notice) {
            (RowsState::Stalled, ..) => Some(STALLED.into()),
            (RowsState::Disconnected, ..) => Some(DISCONNECTED.into()),
            (_, Some(notice), Some(paste)) => Some(format!("{notice} · {paste}")),
            (_, notice, paste) => notice.clone().or(paste.map(String::from)),
        }
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
                Some(format!(
                    "{}{}",
                    marker(focus == Focus::Results),
                    self.sessions_header(content_width.saturating_sub(2))
                )),
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
            // The notice (at most two rows) is budgeted before the list, and
            // stands in for the list's own placeholder when there are no rows.
            let notice_width = content_width.saturating_sub(LIST_INDENT.len()).max(1);
            let mut notice = (self.status_line())
                .map(|text| crate::components::utils::wrap_text(&text, notice_width))
                .unwrap_or_default();
            notice.truncate(2.min(budget.saturating_sub(1)));
            let no_rows = self.list.item_count() == 0
                && (!notice.is_empty() || self.rows_state != RowsState::Settled);
            let mut fit =
                fit_result_window(budget - notice.len(), self.list.item_count(), details.len());
            if no_rows {
                fit.gap_before_footer += fit.result_rows;
                fit.result_rows = 0;
            }
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
            lines.extend(
                notice
                    .iter()
                    .map(|row| format!("{LIST_INDENT}{}", theme::dim(row))),
            );
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
/// What the search box accepts, typed or pasted alike (R2-T5): anything the
/// harness's visible-text fold keeps — no control, bidi or zero-width
/// character, and no whitespace but the space.
fn typeable(c: char) -> bool {
    c == ' ' || !(c.is_whitespace() || super::local_filter::invisible(c))
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

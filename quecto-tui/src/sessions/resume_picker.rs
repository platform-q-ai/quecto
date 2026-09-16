//! Session discovery presentation: modal-local focus, safe rows and stable IDs.
use crate::components::{
    component::Component,
    fuzzy::fuzzy_filter,
    select_list::{SelectItem, SelectList, SelectResult},
    select_overlay::build_select_overlay,
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Scope,
    Query,
    Results,
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
    filtered_ids: Vec<String>,
    scope: SessionListScope,
    focus: Focus,
    query: String,
    hits: HitLayout,
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
            filtered_ids: Vec::new(),
            scope,
            focus: Focus::Results,
            query: String::new(),
            hits: HitLayout::default(),
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
        self.filtered_ids = items.iter().map(|i| i.value.clone()).collect();
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
            (Focus::Results, key) => {
                self.list.handle_input(if *key == Key::Char(' ') {
                    &Key::Enter
                } else {
                    key
                });
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
                return match col {
                    0..=6 => self.change_scope(SessionListScope::Local),
                    10..=17 => self.change_scope(SessionListScope::Global),
                    _ => ResumePickerEvent::Pending,
                };
            }
            if row == h.border + 2 {
                self.focus = Focus::Query;
            }
            if row >= h.border + 3 {
                let selected = self
                    .list
                    .selected_item()
                    .and_then(|s| self.filtered_ids.iter().position(|id| id == &s.value))
                    .unwrap_or(0);
                let start = (selected + 1).saturating_sub(12);
                let offset = row - (h.border + 3);
                if offset < 12
                    && let Some(id) = self.filtered_ids.get(start + offset).cloned()
                {
                    self.focus = Focus::Results;
                    for _ in 0..self.filtered_ids.len() {
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
            SessionListScope::Local => "[Local] | Global",
            SessionListScope::Global => " Local  | [Global]",
        };
        let focus = match self.focus {
            Focus::Scope => "scope",
            Focus::Query => "query",
            Focus::Results => "results",
        };
        let (lines, panel_width) = build_select_overlay(width, height, |content_width| {
            let mut lines = vec![
                "Resume session".into(),
                scope.into(),
                format!("Query: {}", self.query),
            ];
            lines.extend(self.list.render(content_width));
            if let Some(description) = self
                .list
                .selected_item()
                .and_then(|item| item.description.as_deref())
            {
                lines.extend(crate::components::utils::wrap_text(
                    description,
                    content_width,
                ));
            }
            lines.push(format!(
                "Tab focus ({focus}) · Enter/Space select · Esc cancel"
            ));
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

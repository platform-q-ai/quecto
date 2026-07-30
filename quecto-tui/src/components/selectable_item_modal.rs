//! Reusable selectable-item modal for generic enabled/disabled item lists.
//!
//! The component owns normalized, sanitized row data and exposes only stable item
//! IDs in its result so callers keep domain persistence outside the UI layer.

use std::collections::{BTreeSet, HashSet};
use std::fmt;

use crate::components::ansi::sanitize_control;
use crate::components::component::Component;
use crate::components::fuzzy::fuzzy_filter;
use crate::components::list_navigator::ListNavigator;
use crate::components::list_rows::{DescriptionMode, ListRow, render_windowed};
use crate::components::select_overlay::build_select_overlay;
use crate::components::theme;
use crate::components::utils::{truncate_to_width, visible_width};
use crate::shell::keys::Key;

/// Maximum filter-query length; matches existing selector bounds.
pub(crate) const MAX_QUERY_LEN: usize = 64;
const MAX_VISIBLE_ITEMS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectableItemModalResult {
    Applied(BTreeSet<String>),
    Dismissed,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectableItemModalError {
    MissingIdAccessor,
    MissingLabelAccessor,
    DuplicateId(String),
}

impl fmt::Display for SelectableItemModalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingIdAccessor => write!(f, "selectable-item modal requires an id accessor"),
            Self::MissingLabelAccessor => {
                write!(f, "selectable-item modal requires a label accessor")
            }
            Self::DuplicateId(id) => write!(f, "duplicate selectable-item id: {id}"),
        }
    }
}

impl std::error::Error for SelectableItemModalError {}

#[derive(Debug, Clone)]
struct SelectableItemRow {
    id: String,
    label: String,
    description: Option<String>,
    search_text: String,
}

/// Provider contract for domain-owned selectable item sources.
pub trait SelectableItemProvider {
    type Item;

    fn selectable_items(&self) -> Vec<Self::Item>;
    fn enabled_item_ids(&self) -> BTreeSet<String>;
    fn item_id(&self, item: &Self::Item) -> String;
    fn item_label(&self, item: &Self::Item) -> String;
    fn item_description(&self, _item: &Self::Item) -> Option<String> {
        None
    }
    fn item_search_metadata(&self, _item: &Self::Item) -> Vec<String> {
        Vec::new()
    }
    fn apply_enabled_item_ids(&mut self, enabled_ids: BTreeSet<String>);
    fn dismiss_selectable_items(&mut self) {}
}

type StringAccessor<T> = Box<dyn Fn(&T) -> String + Send>;
type OptionalStringAccessor<T> = Box<dyn Fn(&T) -> Option<String> + Send>;
type MetadataAccessor<T> = Box<dyn Fn(&T) -> Vec<String> + Send>;

pub struct SelectableItemModalBuilder<T> {
    items: Vec<T>,
    enabled_ids: BTreeSet<String>,
    id: Option<StringAccessor<T>>,
    label: Option<StringAccessor<T>>,
    description: Option<OptionalStringAccessor<T>>,
    search_metadata: Option<MetadataAccessor<T>>,
}

impl<T> Default for SelectableItemModalBuilder<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            enabled_ids: BTreeSet::new(),
            id: None,
            label: None,
            description: None,
            search_metadata: None,
        }
    }
}

impl<T> SelectableItemModalBuilder<T> {
    pub fn items(mut self, items: Vec<T>) -> Self {
        self.items = items;
        self
    }

    pub fn enabled_ids(mut self, enabled_ids: BTreeSet<String>) -> Self {
        self.enabled_ids = enabled_ids;
        self
    }

    pub fn id(mut self, f: impl Fn(&T) -> String + Send + 'static) -> Self {
        self.id = Some(Box::new(f));
        self
    }

    pub fn label(mut self, f: impl Fn(&T) -> String + Send + 'static) -> Self {
        self.label = Some(Box::new(f));
        self
    }

    pub fn description(mut self, f: impl Fn(&T) -> Option<String> + Send + 'static) -> Self {
        self.description = Some(Box::new(f));
        self
    }

    pub fn search_metadata(mut self, f: impl Fn(&T) -> Vec<String> + Send + 'static) -> Self {
        self.search_metadata = Some(Box::new(f));
        self
    }

    pub fn build(self) -> Result<SelectableItemModal, SelectableItemModalError> {
        let id = self.id.ok_or(SelectableItemModalError::MissingIdAccessor)?;
        let label = self
            .label
            .ok_or(SelectableItemModalError::MissingLabelAccessor)?;
        let mut seen = HashSet::new();
        let mut rows = Vec::with_capacity(self.items.len());

        for item in &self.items {
            let id = id(item);
            if !seen.insert(id.clone()) {
                return Err(SelectableItemModalError::DuplicateId(id));
            }
            let sanitized_id = sanitize_control(&id);
            let label = sanitize_control(&label(item));
            let description = self
                .description
                .as_ref()
                .and_then(|f| f(item))
                .map(|s| sanitize_control(&s));
            let metadata: Vec<String> = self
                .search_metadata
                .as_ref()
                .map(|f| f(item))
                .unwrap_or_default()
                .into_iter()
                .map(|s| sanitize_control(&s))
                .collect();
            let mut search_parts = vec![sanitized_id, label.clone()];
            if let Some(desc) = &description {
                search_parts.push(desc.clone());
            }
            search_parts.extend(metadata);
            rows.push(SelectableItemRow {
                id,
                label,
                description,
                search_text: search_parts.join(" "),
            });
        }

        let enabled_ids = self
            .enabled_ids
            .into_iter()
            .filter(|id| seen.contains(id))
            .collect::<BTreeSet<_>>();

        let mut modal = SelectableItemModal {
            rows,
            visible_indices: Vec::new(),
            original_enabled: enabled_ids.clone(),
            working_enabled: enabled_ids,
            query: String::new(),
            navigator: ListNavigator::new(),
            result: SelectableItemModalResult::Pending,
            cached_max_label_width: 10,
        };
        modal.update_filter();
        Ok(modal)
    }
}

#[derive(Debug)]
pub struct SelectableItemModal {
    rows: Vec<SelectableItemRow>,
    visible_indices: Vec<usize>,
    original_enabled: BTreeSet<String>,
    working_enabled: BTreeSet<String>,
    query: String,
    navigator: ListNavigator,
    result: SelectableItemModalResult,
    cached_max_label_width: usize,
}

impl SelectableItemModal {
    pub fn builder<T>() -> SelectableItemModalBuilder<T> {
        SelectableItemModalBuilder::default()
    }

    pub fn from_provider<P: SelectableItemProvider>(
        provider: &P,
    ) -> Result<Self, SelectableItemModalError>
    where
        P::Item: 'static,
    {
        let items = provider.selectable_items();
        let enabled = provider.enabled_item_ids();
        let ids: Vec<String> = items.iter().map(|item| provider.item_id(item)).collect();
        let labels: Vec<String> = items.iter().map(|item| provider.item_label(item)).collect();
        let descriptions: Vec<Option<String>> = items
            .iter()
            .map(|item| provider.item_description(item))
            .collect();
        let metadata: Vec<Vec<String>> = items
            .iter()
            .map(|item| provider.item_search_metadata(item))
            .collect();

        Self::builder()
            .items((0..items.len()).collect::<Vec<usize>>())
            .enabled_ids(enabled)
            .id(move |idx| ids[*idx].clone())
            .label(move |idx| labels[*idx].clone())
            .description(move |idx| descriptions[*idx].clone())
            .search_metadata(move |idx| metadata[*idx].clone())
            .build()
    }

    pub fn take_result(&mut self) -> SelectableItemModalResult {
        std::mem::replace(&mut self.result, SelectableItemModalResult::Pending)
    }

    pub fn selected_item(&self) -> Option<&str> {
        self.selected_row().map(|row| row.id.as_str())
    }

    pub fn visible_count(&self) -> usize {
        self.visible_indices.len()
    }

    pub fn toggle_selected(&mut self) {
        let Some(id) = self.selected_row().map(|row| row.id.clone()) else {
            return;
        };
        if !self.working_enabled.insert(id.clone()) {
            self.working_enabled.remove(&id);
        }
    }

    pub fn enable_visible(&mut self) {
        for idx in &self.visible_indices {
            self.working_enabled.insert(self.rows[*idx].id.clone());
        }
    }

    pub fn disable_visible(&mut self) {
        for idx in &self.visible_indices {
            self.working_enabled.remove(&self.rows[*idx].id);
        }
    }

    pub fn original_enabled_ids(&self) -> &BTreeSet<String> {
        &self.original_enabled
    }

    fn selected_row(&self) -> Option<&SelectableItemRow> {
        let idx = *self.visible_indices.get(self.navigator.selected())?;
        self.rows.get(idx)
    }

    fn update_filter(&mut self) {
        let previous_id = self.selected_item().map(str::to_owned);
        self.visible_indices = if self.query.is_empty() {
            (0..self.rows.len()).collect()
        } else {
            fuzzy_filter(&self.rows, &self.query, |row| row.search_text.as_str())
                .into_iter()
                .filter_map(|row| {
                    self.rows
                        .iter()
                        .position(|candidate| std::ptr::eq(candidate, row))
                })
                .collect()
        };
        self.cached_max_label_width = self
            .visible_indices
            .iter()
            .map(|idx| visible_width(&self.rows[*idx].label) + 4)
            .max()
            .unwrap_or(10)
            .min(40);
        if let Some(previous_id) = previous_id
            && let Some(pos) = self
                .visible_indices
                .iter()
                .position(|idx| self.rows[*idx].id == previous_id)
        {
            self.navigator.set_selected(pos);
        }
        self.navigator.clamp(self.visible_indices.len());
    }

    fn search_line(&self, width: usize) -> String {
        let line = if self.query.is_empty() {
            format!("  {}", theme::dim("Search: _"))
        } else {
            format!("  Search: {}{}", self.query, theme::dim("_"))
        };
        truncate_to_width(&line, width, None)
    }
}

impl Component for SelectableItemModal {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(truncate_to_width(
            &format!(
                "  {} {}",
                theme::bold("Select Items"),
                theme::dim("(type to filter)")
            ),
            width,
            None,
        ));
        lines.push(self.search_line(width));
        lines.push(String::new());

        if self.visible_indices.is_empty() {
            lines.push(truncate_to_width(
                &format!("  {}", theme::dim("No matching items")),
                width,
                None,
            ));
            return lines;
        }

        let mode = DescriptionMode::AlignedCached {
            label_width: self.cached_max_label_width,
        };
        lines.extend(render_windowed(
            &self.visible_indices,
            &self.navigator,
            MAX_VISIBLE_ITEMS,
            width,
            "  ",
            mode,
            |idx| {
                let row = &self.rows[*idx];
                let marker = if self.working_enabled.contains(&row.id) {
                    "[x] "
                } else {
                    "[ ] "
                };
                ListRow {
                    label: format!("{marker}{}", row.label),
                    description: row.description.clone(),
                    marker: "",
                    dim_label: false,
                }
            },
        ));
        lines
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        match key {
            Key::Up => self.navigator.move_previous(self.visible_indices.len()),
            Key::Down => self.navigator.move_next(self.visible_indices.len()),
            Key::Enter => {
                self.result = SelectableItemModalResult::Applied(self.working_enabled.clone())
            }
            Key::Escape => {
                self.working_enabled = self.original_enabled.clone();
                self.result = SelectableItemModalResult::Dismissed;
            }
            Key::Backspace => {
                self.query.pop();
                self.update_filter();
            }
            Key::Char(' ') => self.toggle_selected(),
            Key::Char(c) => {
                if self.query.len() < MAX_QUERY_LEN {
                    self.query.push(*c);
                    self.update_filter();
                }
            }
            Key::CtrlShift('a') => self.enable_visible(),
            Key::CtrlShift('d') => self.disable_visible(),
            _ => return false,
        }
        true
    }

    fn invalidate(&mut self) {}
}

pub fn build_selectable_item_modal_overlay(
    title: &str,
    footer: &str,
    modal: &mut SelectableItemModal,
    terminal_width: usize,
    terminal_height: usize,
) -> (Vec<String>, usize) {
    build_select_overlay(terminal_width, terminal_height, |content_width| {
        let mut content_lines = vec![theme::bold(title)];
        content_lines.extend(modal.render(content_width));
        content_lines.push(theme::dim(footer));
        content_lines
    })
}

#[cfg(test)]
#[path = "selectable_item_modal_tests.rs"]
mod tests;

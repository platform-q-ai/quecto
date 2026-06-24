//! Widget system — persistent components above or below the editor.
//!
//! Widgets are keyed by name and can be set, updated, or cleared.
//! The widget container renders all active widgets in insertion order.

use crate::interface::component::Component;
use crate::interface::components::text::Text;
use std::collections::BTreeMap;

/// A single widget entry.
struct WidgetEntry {
    component: Box<dyn Component>,
}

/// Container that manages named widgets.
///
/// Widgets are rendered in sorted key order for deterministic display.
pub struct WidgetContainer {
    widgets: BTreeMap<String, WidgetEntry>,
}

impl WidgetContainer {
    pub fn new() -> Self {
        Self {
            widgets: BTreeMap::new(),
        }
    }

    /// Set a widget by key. Replaces any existing widget with the same key.
    pub fn set(&mut self, key: &str, component: Box<dyn Component>) {
        self.widgets
            .insert(key.to_string(), WidgetEntry { component });
    }

    /// Set a simple text widget by key.
    pub fn set_text(&mut self, key: &str, text: &str) {
        self.set(key, Box::new(Text::new(text)));
    }

    /// Remove a widget by key.
    pub fn clear(&mut self, key: &str) {
        self.widgets.remove(key);
    }

    /// Remove all widgets.
    pub fn clear_all(&mut self) {
        self.widgets.clear();
    }

    /// Whether there are any widgets.
    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }

    /// Number of widgets.
    pub fn len(&self) -> usize {
        self.widgets.len()
    }
}

impl Default for WidgetContainer {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for WidgetContainer {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for entry in self.widgets.values_mut() {
            lines.extend(entry.component.render(width));
        }
        lines
    }

    fn invalidate(&mut self) {
        for entry in self.widgets.values_mut() {
            entry.component.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_set_and_render() {
        let mut wc = WidgetContainer::new();
        wc.set_text("status", "Build: OK");
        let lines = wc.render(40);
        assert!(!lines.is_empty());
        assert!(lines[0].contains("Build: OK"));
    }

    #[test]
    fn widget_clear() {
        let mut wc = WidgetContainer::new();
        wc.set_text("status", "Build: OK");
        wc.clear("status");
        assert!(wc.is_empty());
        let lines = wc.render(40);
        assert!(lines.is_empty());
    }

    #[test]
    fn widget_clear_all() {
        let mut wc = WidgetContainer::new();
        wc.set_text("a", "1");
        wc.set_text("b", "2");
        wc.clear_all();
        assert!(wc.is_empty());
    }

    #[test]
    fn widgets_render_in_sorted_order() {
        let mut wc = WidgetContainer::new();
        wc.set_text("b", "Second");
        wc.set_text("a", "First");
        let lines = wc.render(40);
        let joined = lines.join("\n");
        let first_pos = joined.find("First").unwrap();
        let second_pos = joined.find("Second").unwrap();
        assert!(first_pos < second_pos, "a should render before b");
    }

    #[test]
    fn widget_replace() {
        let mut wc = WidgetContainer::new();
        wc.set_text("status", "old");
        wc.set_text("status", "new");
        assert_eq!(wc.len(), 1);
        let lines = wc.render(40);
        assert!(lines[0].contains("new"));
    }
}

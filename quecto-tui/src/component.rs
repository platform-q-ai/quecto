//! Component system — trait and Container, matching Pi TUI's design.
//!
//! Components render to `Vec<String>` (one per line), handle keyboard input,
//! and cache their output until `invalidate()` is called.

/// A renderable TUI component.
///
/// All components must implement this trait. The TUI calls `render()` each
/// frame with the current terminal width; the component returns one string
/// per line. Each line must not exceed `width` visible characters.
pub trait Component: Send {
    /// Render the component to lines for the given viewport width.
    fn render(&self, width: usize) -> Vec<String>;

    /// Handle a keyboard input event. Return `true` if the input was consumed.
    fn handle_input(&mut self, _key: &crate::keys::Key) -> bool {
        false
    }

    /// Clear cached rendering state. Called when theme changes or state updates.
    fn invalidate(&mut self) {}
}

/// Groups child components vertically.
///
/// `Container` renders each child in order, concatenating their output lines.
pub struct Container {
    children: Vec<Box<dyn Component>>,
}

impl Container {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) {
        self.children.push(component);
    }

    pub fn remove_child(&mut self, index: usize) {
        if index < self.children.len() {
            self.children.remove(index);
        }
    }

    pub fn clear(&mut self) {
        self.children.clear();
    }

    pub fn child_count(&self) -> usize {
        self.children.len()
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Container {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for child in &self.children {
            lines.extend(child.render(width));
        }
        lines
    }

    fn handle_input(&mut self, key: &crate::keys::Key) -> bool {
        // Forward input to children; first consumer wins.
        for child in &mut self.children {
            if child.handle_input(key) {
                return true;
            }
        }
        false
    }

    fn invalidate(&mut self) {
        for child in &mut self.children {
            child.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::text::Text;

    #[test]
    fn container_renders_children_in_order() {
        let mut c = Container::new();
        c.add_child(Box::new(Text::new("first")));
        c.add_child(Box::new(Text::new("second")));
        let lines = c.render(80);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "first");
        assert_eq!(lines[1], "second");
    }

    #[test]
    fn container_empty_renders_empty() {
        let c = Container::new();
        assert!(c.render(80).is_empty());
    }

    #[test]
    fn container_clear() {
        let mut c = Container::new();
        c.add_child(Box::new(Text::new("a")));
        c.add_child(Box::new(Text::new("b")));
        assert_eq!(c.child_count(), 2);
        c.clear();
        assert_eq!(c.child_count(), 0);
    }
}

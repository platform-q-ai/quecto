//! Component system — trait shared by TUI widgets.
//!
//! Components render to `Vec<String>` (one per line), handle keyboard input,
//! and cache their output until `invalidate()` is called.

/// A renderable TUI component.
///
/// All components must implement this trait. The TUI calls `render()` each
/// frame with the current terminal width; the component returns one string
/// per line. Each line must not exceed `width` visible characters.
///
/// `render(&mut self)` takes a mutable reference so components can update
/// their render cache inline, avoiding interior mutability.
pub trait Component: Send {
    /// Render the component to lines for the given viewport width.
    fn render(&mut self, width: usize) -> Vec<String>;

    /// Handle a keyboard input event. Return `true` if the input was consumed.
    fn handle_input(&mut self, _key: &crate::shell::keys::Key) -> bool {
        false
    }

    /// Clear cached rendering state. Called when theme changes or state updates.
    fn invalidate(&mut self) {}
}

#[cfg(test)]
#[path = "component_tests.rs"]
mod tests;

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
    fn handle_input(&mut self, _key: &crate::interface::keys::Key) -> bool {
        false
    }

    /// Clear cached rendering state. Called when theme changes or state updates.
    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::keys::Key;

    struct DefaultOnly;

    impl Component for DefaultOnly {
        fn render(&mut self, width: usize) -> Vec<String> {
            vec![format!("width={width}")]
        }
    }

    #[test]
    fn trait_default_methods_do_not_consume_input_or_mutate_rendering() {
        let mut component = DefaultOnly;

        assert_eq!(component.render(7), vec!["width=7".to_string()]);
        assert!(!component.handle_input(&Key::Enter));
        component.invalidate();
        assert_eq!(component.render(3), vec!["width=3".to_string()]);
    }
}

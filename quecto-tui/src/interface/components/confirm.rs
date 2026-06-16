//! Confirm dialog — Yes/No modal for destructive actions.

use crate::interface::component::Component;
use crate::interface::keys::Key;
use crate::interface::theme;
use crate::interface::utils::truncate_to_width;

/// Result of a confirm dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    Confirmed,
    Cancelled,
    Pending,
}

/// A simple Yes/No confirmation dialog.
pub struct ConfirmDialog {
    title: String,
    message: String,
    selected: bool, // true = Yes, false = No
    result: ConfirmResult,
}

impl ConfirmDialog {
    pub fn new(title: &str, message: &str) -> Self {
        Self {
            title: title.to_string(),
            message: message.to_string(),
            selected: false, // Default to No (safer)
            result: ConfirmResult::Pending,
        }
    }

    pub fn take_result(&mut self) -> ConfirmResult {
        std::mem::replace(&mut self.result, ConfirmResult::Pending)
    }
}

impl Component for ConfirmDialog {
    fn render(&mut self, width: usize) -> Vec<String> {
        let _inner = width.saturating_sub(4); // 2 padding each side
        let border = theme::accent(&"─".repeat(width));
        let pad = "  ";

        let mut lines = Vec::new();
        lines.push(border.clone());
        lines.push(String::new());
        lines.push(truncate_to_width(
            &format!("{}{}", pad, theme::bold(&theme::accent(&self.title))),
            width,
            None,
        ));
        lines.push(String::new());

        // Wrap message.
        for msg_line in self.message.lines() {
            lines.push(truncate_to_width(
                &format!("{}{}", pad, msg_line),
                width,
                None,
            ));
        }

        lines.push(String::new());

        // Buttons.
        let yes = if self.selected {
            theme::bold(&theme::accent("[Yes]"))
        } else {
            theme::dim("[Yes]")
        };
        let no = if !self.selected {
            theme::bold(&theme::accent("[No]"))
        } else {
            theme::dim("[No]")
        };
        lines.push(truncate_to_width(
            &format!("{}{}   {}", pad, yes, no),
            width,
            None,
        ));

        lines.push(String::new());
        lines.push(border);

        lines
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        match key {
            Key::Left | Key::Right | Key::Tab => {
                self.selected = !self.selected;
                true
            }
            Key::Enter => {
                self.result = if self.selected {
                    ConfirmResult::Confirmed
                } else {
                    ConfirmResult::Cancelled
                };
                true
            }
            Key::Escape => {
                self.result = ConfirmResult::Cancelled;
                true
            }
            Key::Char('y') | Key::Char('Y') => {
                self.result = ConfirmResult::Confirmed;
                true
            }
            Key::Char('n') | Key::Char('N') => {
                self.result = ConfirmResult::Cancelled;
                true
            }
            _ => true, // Consume all input while dialog is active.
        }
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_title_and_message() {
        let mut d = ConfirmDialog::new("Confirm", "Are you sure?");
        let lines = d.render(40);
        let joined: String = lines.join("\n");
        let plain: String = joined
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        assert!(plain.contains("Confirm"), "should show title: {}", plain);
        assert!(
            plain.contains("Are you sure?"),
            "should show message: {}",
            plain
        );
    }

    #[test]
    fn enter_on_yes_confirms() {
        let mut d = ConfirmDialog::new("Test", "?");
        d.handle_input(&Key::Left); // Move to Yes
        d.handle_input(&Key::Enter);
        assert_eq!(d.take_result(), ConfirmResult::Confirmed);
    }

    #[test]
    fn enter_on_no_cancels() {
        let mut d = ConfirmDialog::new("Test", "?");
        // Default is No.
        d.handle_input(&Key::Enter);
        assert_eq!(d.take_result(), ConfirmResult::Cancelled);
    }

    #[test]
    fn escape_cancels() {
        let mut d = ConfirmDialog::new("Test", "?");
        d.handle_input(&Key::Escape);
        assert_eq!(d.take_result(), ConfirmResult::Cancelled);
    }

    #[test]
    fn y_key_confirms() {
        let mut d = ConfirmDialog::new("Test", "?");
        d.handle_input(&Key::Char('y'));
        assert_eq!(d.take_result(), ConfirmResult::Confirmed);
    }

    #[test]
    fn n_key_cancels() {
        let mut d = ConfirmDialog::new("Test", "?");
        d.handle_input(&Key::Char('n'));
        assert_eq!(d.take_result(), ConfirmResult::Cancelled);
    }

    #[test]
    fn tab_toggles_selection() {
        let mut d = ConfirmDialog::new("Test", "?");
        assert!(!d.selected); // starts on No
        d.handle_input(&Key::Tab);
        assert!(d.selected); // now on Yes
        d.handle_input(&Key::Tab);
        assert!(!d.selected); // back to No
    }
}

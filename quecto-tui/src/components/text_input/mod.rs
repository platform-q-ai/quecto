//! # Text-input system
//!
//! Dedicated owner of the TUI main-prompt draft: multi-line buffer, cursor,
//! submit/clear, paste, bash-mode detection, bordered render, and
//! **submitted-input history**.
//!
//! ## API boundary
//!
//! | Concern | Through |
//! |---|---|
//! | Draft text | [`Editor::text`], [`Editor::set_text`] |
//! | Cursor / line for overlays | [`Editor::cursor_col`], [`Editor::current_line`] |
//! | Typing / keys / paste | [`Component::handle_input`](crate::components::component::Component::handle_input) |
//! | Submit | [`Editor::take_submit`] (Enter sets it internally) |
//! | History | [`Editor::add_to_history`] + Up/Down inside `handle_input` |
//! | Token replace (`@files`) | [`Editor::replace_before_cursor`] |
//! | Cursor visibility | [`Editor::set_show_cursor`] |
//! | Render | [`Component::render`](crate::components::component::Component::render) |
//!
//! **Outside this system:** slash-command and `@files` autocomplete widgets,
//! modal selectors (model/effort/resume/rewind), panel focus, and App key
//! routing. Those call the API above; they must not own a parallel draft
//! buffer or submit history.
//!
//! **Mutation:** all buffer/history changes go through the methods above.
//! Fields are private — no external field poking.
//!
//! History implementation lives in [`history`] (private to this module).

mod editor;
mod history;

pub use editor::Editor;

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;

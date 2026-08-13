//! Tab activity indicators (#1466): per-tab spinner (turn in flight) and
//! unread dot (output since last viewed) for the tab bar, plus the
//! background-tab render gate and the tab-switch key chords.

use super::*;

impl App {
    /// Demote a routed event's paint to [`SourcedRender::Silent`] when its
    /// owner tab is not the focused one (#1466 decision 3), marking the tab
    /// unread instead — the repaint happens once, on switch.
    pub(super) fn background_render_gate(
        &mut self,
        tab: crate::shell::connection::TabId,
        paint: super::app_event_loop::SourcedRender,
    ) -> super::app_event_loop::SourcedRender {
        if tab == self.active_tab {
            return paint;
        }
        if let Some(c) = self.conn_mut(tab) {
            c.unread_output = true;
        }
        super::app_event_loop::SourcedRender::Silent
    }

    /// Handle a tab-switch key chord (#1466 decision 5): Alt+digit focuses
    /// the Nth tab (kitty Ctrl+digit parses to the same key); Alt+Tab /
    /// Ctrl+Tab cycle forward, Shift variants cycle back. Returns whether the
    /// key was consumed; unknown ordinals consume and no-op.
    pub(super) fn handle_tab_switch_key(&mut self, key: &Key) -> bool {
        match key {
            Key::Alt(c @ '1'..='9') => {
                self.focus_tab_ordinal(*c as usize - '0' as usize);
            }
            Key::TabSwitchNext => {
                self.switch_tab_next();
            }
            Key::TabSwitchPrev => {
                self.switch_tab_prev();
            }
            _ => return false,
        }
        true
    }
    /// Render the tab bar (#1466 decision 4/5): one line listing every open
    /// tab as `N:name`, with the activity spinner while that tab's turn is in
    /// flight and the unread dot when output arrived since it was last viewed.
    /// `None` with a single tab, so single-tab frames stay byte-identical.
    ///
    /// Overflow (#1466 decision 5): when the strip is wider than the terminal
    /// it scrolls so the ACTIVE tab stays visible — leading cells are dropped
    /// behind a `‹` marker and a trailing `›` marks clipped cells on the right.
    pub(super) fn render_tab_bar(&self, width: usize) -> Option<String> {
        use crate::components::theme::{self, SPINNER_FRAMES};
        let ids = self.ordered_tab_ids();
        if ids.len() < 2 || width == 0 {
            return None;
        }
        let mut cells: Vec<(String, usize)> = Vec::with_capacity(ids.len());
        let mut active_idx = 0;
        for (i, tab) in ids.iter().enumerate() {
            let conn = self.conn_for(*tab)?;
            let indicator = if self.tab_spinner_active(*tab) {
                let frame = conn
                    .spinner
                    .as_ref()
                    .map(|s| s.frame_index())
                    .unwrap_or_default();
                format!(" {}", SPINNER_FRAMES[frame % SPINNER_FRAMES.len()])
            } else if self.tab_unread(*tab) {
                " ●".to_string()
            } else {
                String::new()
            };
            let text = format!(" {}:{}{} ", i + 1, conn.display_name(), indicator);
            let cell_width = crate::components::utils::visible_width(&text);
            let styled = if *tab == self.active_tab {
                active_idx = i;
                theme::accent(&text)
            } else {
                theme::dim(&text)
            };
            cells.push((styled, cell_width));
        }
        // Scroll: drop leading cells until the active tab fits in the window.
        let mut start = 0;
        while start < active_idx
            && cells[start..=active_idx].iter().map(|c| c.1).sum::<usize>() + 1 > width
        {
            start += 1;
        }
        let mut line = String::new();
        let mut used = 0;
        if start > 0 {
            line.push_str(&theme::dim("‹"));
            used += 1;
        }
        for (styled, cell_width) in &cells[start..] {
            if used + cell_width > width {
                line.push_str(&theme::dim("›"));
                break;
            }
            line.push_str(styled);
            used += cell_width;
        }
        Some(line)
    }

    /// Spinner semantics (#1466 decision 4): a tab shows the spinner while its
    /// master turn is in flight.
    pub(crate) fn tab_spinner_active(&self, tab: crate::shell::connection::TabId) -> bool {
        self.conn_for(tab)
            .is_some_and(|c| c.agent_state.is_running())
    }

    /// Unread-dot semantics (#1466 decision 4): any output arrived on this tab
    /// since it was last viewed; cleared on switch.
    pub(crate) fn tab_unread(&self, tab: crate::shell::connection::TabId) -> bool {
        self.conn_for(tab).is_some_and(|c| c.unread_output)
    }
}

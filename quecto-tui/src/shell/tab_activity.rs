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

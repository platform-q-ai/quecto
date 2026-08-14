//! Tab activity indicators (#1466): per-tab spinner (turn in flight) and
//! unread dot (output since last viewed) for the tab bar, plus the
//! background-tab render gate and the tab-switch key chords.

use super::*;

/// Max rendered columns of a custom tab name in the bar (spike design).
const TAB_NAME_MAX: usize = 16;

/// What a click on a tab-bar column selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabBarHit {
    Select(crate::shell::connection::TabId),
    New,
}

/// A clickable region of the tab bar: absolute terminal columns → action.
pub(crate) type TabBarHitRange = (std::ops::Range<usize>, TabBarHit);

/// Solid-block styling (spike design): reverse video so the block follows the
/// terminal theme. Active = cyan block; inactive = dim block. Composed from
/// `components::theme` helpers (PR #1485 review) so a theme-wide change can
/// never silently miss the tab bar.
fn active_block(text: &str) -> String {
    crate::components::theme::reverse(&crate::components::theme::cyan(text))
}

fn inactive_block(text: &str) -> String {
    crate::components::theme::reverse(&crate::components::theme::dim(text))
}

impl App {
    /// Demote a routed event's paint to [`SourcedRender::Silent`] when its
    /// owner tab is not the focused one (#1466 decision 3), marking the tab
    /// unread instead — the repaint happens once, on switch.
    ///
    /// Exception (PR #1485 review): a turn-state TRANSITION (`was_running`
    /// differs from the tab's running state after the event) keeps its paint.
    /// After a turn ends `needs_animation_tick` disarms, so a Silent demotion
    /// of the ending event would leave the bar's spinner frozen forever.
    pub(super) fn background_render_gate(
        &mut self,
        tab: crate::shell::connection::TabId,
        paint: super::app_event_loop::SourcedRender,
        was_running: bool,
    ) -> super::app_event_loop::SourcedRender {
        if tab == self.active_tab {
            return paint;
        }
        if let Some(c) = self.conn_mut(tab) {
            c.unread_output = true;
        }
        if was_running != self.tab_spinner_active(tab) {
            return paint;
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
    /// Render the tab bar (#1466 fix pass, spike design): herdr-style
    /// reverse-video number blocks — active tab a cyan block, inactive tabs
    /// dim blocks. Unnamed tabs render the bare 1-based number (` N `);
    /// custom-named tabs render ` N:name ` with the name truncated to
    /// [`TAB_NAME_MAX`] columns behind an ellipsis. The activity spinner /
    /// unread dot render INSIDE the block. A trailing dim ` + ` new-tab
    /// button ends the bar. `None` with a single tab, so single-tab frames
    /// stay byte-identical.
    ///
    /// Overflow (#1466 decision 5): when the strip is wider than the terminal
    /// it scrolls so the ACTIVE tab stays visible — leading cells are dropped
    /// behind a `‹` marker and a trailing `›` marks clipped cells on the right.
    pub(super) fn render_tab_bar(&self, width: usize) -> Option<String> {
        self.tab_bar_layout(width).map(|(line, _)| line)
    }

    /// Mouse hit ranges for the tab bar rendered at `width` body columns:
    /// absolute terminal column ranges (past the optional left panel) mapping
    /// to the tab (or new-tab button) a click there selects.
    pub(crate) fn tab_bar_hit_ranges(&self, width: usize) -> Vec<TabBarHitRange> {
        self.tab_bar_layout(width)
            .map(|(_, hits)| hits)
            .unwrap_or_default()
    }

    /// Shared layout for the bar line and its click hit ranges, so the two
    /// can never disagree about geometry.
    fn tab_bar_layout(&self, width: usize) -> Option<(String, Vec<TabBarHitRange>)> {
        use crate::components::theme::{self, SPINNER_FRAMES};
        let ids = self.ordered_tab_ids();
        if ids.len() < 2 || width == 0 {
            return None;
        }
        let (panel_width, divider_width, _) = self.frame_split();
        let body_start_col = panel_width + divider_width;
        let mut cells: Vec<(String, usize, crate::shell::connection::TabId)> =
            Vec::with_capacity(ids.len());
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
            // Herdr-style block label: bare 1-based number unless the tab has
            // a CUSTOM name — never a default ":Master" suffix.
            let text = match conn.name.as_deref().filter(|n| !n.is_empty()) {
                Some(name) => {
                    // Truncate only past the cap: a name of exactly
                    // TAB_NAME_MAX columns renders in full, no ellipsis.
                    let shown = if crate::components::utils::visible_width(name) > TAB_NAME_MAX {
                        crate::components::utils::sanitize_truncate_width_with_ellipsis(
                            name,
                            TAB_NAME_MAX,
                            "…",
                        )
                    } else {
                        name.to_string()
                    };
                    format!(" {}:{}{} ", i + 1, shown, indicator)
                }
                None => format!(" {}{} ", i + 1, indicator),
            };
            let cell_width = crate::components::utils::visible_width(&text);
            let styled = if *tab == self.active_tab {
                active_idx = i;
                active_block(&text)
            } else {
                inactive_block(&text)
            };
            cells.push((styled, cell_width, *tab));
        }
        // Reserve room for the trailing dim ` + ` new-tab button.
        let plus_width = 3;
        // Scroll: drop leading cells until the active tab (plus its gap and
        // the ` + ` button) fits in the window.
        let fits = |start: usize| {
            let lead = 1 + usize::from(start > 0); // leading space (+ ‹)
            let cell_cols: usize = cells[start..=active_idx].iter().map(|c| c.1 + 1).sum();
            lead + cell_cols + plus_width <= width
        };
        let mut start = 0;
        while start < active_idx && !fits(start) {
            start += 1;
        }
        let mut line = String::from(" ");
        let mut used = 1;
        let mut hits: Vec<TabBarHitRange> = Vec::new();
        if start > 0 {
            line.push_str(&theme::dim("‹"));
            used += 1;
        }
        let mut trailing_gap = false;
        for (styled, cell_width, tab) in &cells[start..] {
            if used + cell_width + plus_width + 1 > width {
                // Clipped tail: the `›` marker takes the trailing gap's column.
                if trailing_gap {
                    line.pop();
                    used -= 1;
                }
                line.push_str(&theme::dim("›"));
                used += 1;
                break;
            }
            hits.push((
                body_start_col + used..body_start_col + used + cell_width,
                TabBarHit::Select(*tab),
            ));
            line.push_str(styled);
            used += cell_width;
            line.push(' ');
            used += 1;
            trailing_gap = true;
        }
        hits.push((
            body_start_col + used..body_start_col + used + plus_width,
            TabBarHit::New,
        ));
        line.push_str(&theme::dim(" + "));
        Some((line, hits))
    }

    /// Handle a mouse press on the tab-bar row (row 0 with 2+ tabs). Clicking
    /// a tab's block focuses it; clicking the trailing ` + ` opens a live tab.
    /// Returns whether the click was consumed.
    pub(super) fn handle_tab_bar_click(&mut self, col: u16, row: u16) -> bool {
        if row != 0 || !self.tab_bar_visible() {
            return false;
        }
        let (_, _, width) = self.frame_split();
        let col = col as usize;
        let hit = self
            .tab_bar_hit_ranges(width)
            .into_iter()
            .find(|(range, _)| range.contains(&col))
            .map(|(_, hit)| hit);
        match hit {
            Some(TabBarHit::Select(tab)) => {
                let _ = self.switch_tab(tab);
            }
            Some(TabBarHit::New) => {
                let tab = self.open_live_tab(None);
                self.notify(
                    &format!("Opened tab {} (connecting…)", tab.0),
                    crate::components::notification::NotifyLevel::Info,
                );
            }
            None => return false,
        }
        true
    }

    /// Whether the tab bar renders at all (2+ tabs).
    pub(crate) fn tab_bar_visible(&self) -> bool {
        self.tabs.len() > 1
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

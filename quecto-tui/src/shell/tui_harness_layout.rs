//! Layout probes split for line budget (#1465).

use super::super::app_render_helpers::strip_ansi;
use super::TuiHarness;

impl TuiHarness {
    /// The bottom stack (below-chat section), ANSI-stripped — for asserting on
    /// what does (or no longer does) render in the input/footer area (#820).
    pub fn bottom_stack(&mut self) -> String {
        // Render at the reduced body width the real frame uses once the panel
        // is on (#820 review), not the full terminal width.
        let width = self.app.body_width();
        self.app
            .compose_bottom(width)
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The main-pane (top) region of the frame — everything above the bottom
    /// stack — ANSI-stripped, for asserting on the relocated workflow bar (#820).
    pub fn main_pane(&mut self) -> String {
        // Slice the real frame at the same body width compose_frame used, so the
        // top/bottom split matches what the user sees (#820 review).
        let width = self.app.body_width();
        let bottom_len = self.app.compose_bottom(width).len();
        let frame = self.app.compose_frame();
        let top = &frame[..frame.len().saturating_sub(bottom_len)];
        top.iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The left sub-agent panel region of the frame — the first `panel_width`
    /// columns of every line, ANSI-stripped — for asserting on panel content
    /// such as the read-only observer marker (#966).
    pub fn left_panel(&mut self) -> String {
        let (panel_width, _, _) = self.app.frame_split();
        self.app
            .compose_frame()
            .iter()
            .map(|l| {
                let stripped = strip_ansi(l);
                let chars: Vec<char> = stripped.chars().collect();
                let take: Vec<char> = chars.into_iter().take(panel_width).collect();
                take.into_iter().collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub async fn drain_commands(&mut self) -> Vec<String> {
        // A command reaches this channel only after several async hops (writer
        // task → socket → reader task → channel), so a multi-command batch is
        // not all visible at once. Returning on the FIRST arrival would observe
        // a scheduler-dependent subset — the source of the flaky recovery
        // assertions. Instead, wait for the stream to go QUIET: keep polling
        // until no new command has arrived for a short settle window, so the
        // whole batch is collected (in wire order, which `send_command` now
        // makes deterministic). Bounded overall so a genuinely empty stream
        // still returns.
        const SETTLE_POLLS: usize = 15;
        const MAX_POLLS: usize = 400;
        let mut out = Vec::new();
        let mut idle = 0;
        for _ in 0..MAX_POLLS {
            let mut got = false;
            while let Ok(line) = self.cmd_rx.try_recv() {
                out.push(line);
                got = true;
            }
            if got {
                idle = 0;
            } else if !out.is_empty() {
                idle += 1;
                if idle >= SETTLE_POLLS {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        out
    }
}

use super::*;

impl App {
    pub(super) fn next_idle_service_deadline(&self) -> Option<tokio::time::Instant> {
        let notification_deadline = self
            .notifications
            .next_expiry()
            .map(tokio::time::Instant::from_std);
        let subagent_gc_deadline =
            next_exited_subagent_gc_deadline(&self.conn.roster.tracked, EXITED_SUBAGENT_GRACE);
        match (notification_deadline, subagent_gc_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }
    }

    pub(super) fn needs_animation_tick(&self, kitty_fallback_pending: bool) -> bool {
        kitty_fallback_pending
            || self.conn.spinner.is_some()
            || self.conn.agent_state.is_running()
            || self.active_session().footer.is_streaming()
            || self.active_subagent_running()
            || self.conn.roster.tracked_active_count() > 0
    }

    pub(super) fn service_animation_tick(
        &mut self,
        kitty_fallback_done: &mut bool,
        kitty_deadline: tokio::time::Instant,
    ) -> bool {
        let mut needs_render = false;
        if let Some(spinner) = &mut self.conn.spinner {
            if spinner.tick() {
                needs_render = true;
            }
        }
        // GC expired notifications.
        if self.notifications.gc() {
            needs_render = true;
        }
        // GC exited subagent bars (#540).
        if self.gc_exited_subagents() {
            needs_render = true;
        }
        // Animate the subagent spinner / advance elapsed-time clocks.
        if self.tick_subagent_animation() {
            needs_render = true;
        }
        // Kitty fallback — enable modifyOtherKeys if no response.
        if !*kitty_fallback_done && tokio::time::Instant::now() >= kitty_deadline {
            if !self.kitty.active {
                self.kitty.enable_modify_other_keys();
            }
            *kitty_fallback_done = true;
        }
        needs_render
    }
}

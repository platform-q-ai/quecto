use super::*;

impl App {
    pub(super) fn needs_animation_tick(&self, kitty_fallback_pending: bool) -> bool {
        kitty_fallback_pending
            || self.spinner.is_some()
            || !self.notifications.is_empty()
            || self.agent_state.is_running()
            || self.active_session().footer.is_streaming()
            || self.active_subagent_running()
            || self.subagent_local.values().any(|entry| {
                subagent_status_is_active(&entry.info.status) || entry.exited_at.is_some()
            })
    }

    pub(super) fn service_animation_tick(
        &mut self,
        kitty_fallback_done: &mut bool,
        kitty_deadline: tokio::time::Instant,
    ) -> bool {
        let mut needs_render = false;
        if let Some(spinner) = &mut self.spinner {
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

use super::*;

impl App {
    pub(super) fn next_idle_service_deadline(&self) -> Option<tokio::time::Instant> {
        let notification_deadline = self
            .notifications
            .next_expiry()
            .map(tokio::time::Instant::from_std);
        let subagent_gc_deadline =
            next_exited_subagent_gc_deadline(&self.ac().roster.tracked, EXITED_SUBAGENT_GRACE);
        // A metadata search that is never answered is given up on time (#2010).
        let search_deadline = self.ac().sessions.search.deadline();
        // …and an Enter owed to its answer expires on time (R2-T3).
        let picker = self.ac().sessions.resume_selector.as_ref();
        let enter_deadline = picker.and_then(|picker| picker.enter_deadline());
        let deadlines = [notification_deadline, subagent_gc_deadline];
        let search = [search_deadline, enter_deadline];
        deadlines.into_iter().chain(search).flatten().min()
    }

    pub(super) fn needs_animation_tick(&self, kitty_fallback_pending: bool) -> bool {
        self.needs_admission_tick()
            || kitty_fallback_pending
            || self.ac().spinner.is_some()
            || self.ac().agent_state.is_running()
            || self.active_session().footer.is_streaming()
            || self.active_subagent_running()
            || self.ac().roster.tracked_active_count() > 0
    }

    pub(super) fn service_animation_tick(
        &mut self,
        kitty_fallback_done: &mut bool,
        kitty_deadline: tokio::time::Instant,
    ) -> bool {
        let mut needs_render = self.tick_admission_labels();
        let state = &mut self.conn;
        if let Some(spinner) = &mut state.spinner {
            if spinner.tick() {
                needs_render = true;
            }
        }
        if self.service_search_timeout(self.clock.now()) {
            needs_render = true;
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

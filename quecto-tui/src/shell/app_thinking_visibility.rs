use super::*;

impl App {
    pub(super) fn toggle_thinking_visibility(&mut self) {
        let show = !self.ac().master_session.chat.show_thinking();
        self.set_thinking_visibility(show);
        super::thinking_preferences::save_thinking_visible(show);
        self.notify(
            if show {
                "Thinking visible"
            } else {
                "Thinking hidden"
            },
            crate::components::notification::NotifyLevel::Info,
        );
    }

    pub(super) fn set_thinking_visibility(&mut self, show: bool) {
        self.ac_mut().master_session.chat.set_show_thinking(show);
        for session in self.ac_mut().roster.sessions.values_mut() {
            session.chat.set_show_thinking(show);
            session.live_inflight.set_show_thinking(show);
        }
    }
}

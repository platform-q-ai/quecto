use crate::components::notification::NotifyLevel;
use crate::shell::app::App;

impl App {
    pub(crate) fn toggle_thinking_visibility(&mut self) {
        let visible = !crate::shell::thinking_prefs::load_thinking_preferences().visible;
        self.ac_mut()
            .master_session
            .chat
            .set_thinking_visible(visible);
        self.ac_mut()
            .master_session
            .live_inflight
            .set_thinking_visible(visible);
        for session in self.ac_mut().roster.sessions.values_mut() {
            session.chat.set_thinking_visible(visible);
            session.live_inflight.set_thinking_visible(visible);
        }
        crate::shell::thinking_prefs::store_thinking_preferences(
            crate::shell::thinking_prefs::ThinkingPreferences { visible },
        );
        let state = if visible { "shown" } else { "hidden" };
        self.notify(&format!("Thinking {state}"), NotifyLevel::Info);
    }
}

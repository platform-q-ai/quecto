use super::*;
impl App {
    pub(super) fn reset_workspace(&mut self) -> Vec<crate::shell::child_watch::ChildWatch> {
        self.persist_default_durability();

        let mut master = self
            .tabs
            .remove(&crate::shell::connection::TabId::MASTER)
            .expect("workspace reset requires a master tab");
        let mut watches = Vec::new();
        for (_, mut state) in self.tabs.drain() {
            state.transport.abort_feed();
            watches.extend(state.child_exit_watch.take());
        }
        master.name = None;
        master.session_key = None;
        master.pending_session_resume = None;
        master.roster = crate::agents::view::ConnectionRoster::new();
        self.tabs
            .insert(crate::shell::connection::TabId::MASTER, master);
        self.active_tab = crate::shell::connection::TabId::MASTER;
        self.routing_tab_override = None;
        self.editor.set_text("");
        self.subagents = crate::agents::view::SubagentUi::new();
        self.workspace_id = crate::shell::workspace_manifest::generate_workspace_id();
        self.workspace_label = crate::shell::workspace_manifest::generate_workspace_label();
        self.reset_session("New session started");
        self.persist_default_durability();
        watches
    }
}

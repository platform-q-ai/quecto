use crate::components::component::Component;
use crate::components::notification::NotifyLevel;
use crate::protocol::client::Command;
use crate::shell::app::App;
use crate::shell::keys::Key;
impl App {
    pub(super) fn open_tool_policy_modal(&mut self) {
        use crate::components::selectable_item_modal::ScopeSelection;
        let mut entries = self.tool_catalogue.values().cloned().collect::<Vec<_>>();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        if entries.is_empty() {
            self.send_command(Command::GetToolCatalogue {
                id: Some("tool-policy-open".into()),
            });
            self.notify("Loading tool catalogue…", NotifyLevel::Info);
            return;
        }
        let scopes = entries
            .iter()
            .map(|e| {
                let id = if e.stable_id.is_empty() {
                    e.name.clone()
                } else {
                    e.stable_id.clone()
                };
                let scope = match e.effective_scope {
                    Some(crate::protocol::client::ToolScope::Parent) => ScopeSelection::Parent,
                    Some(crate::protocol::client::ToolScope::Child) => ScopeSelection::Child,
                    Some(crate::protocol::client::ToolScope::Both) => ScopeSelection::Both,
                    _ => ScopeSelection::None,
                };
                (id, scope)
            })
            .collect();
        self.tool_policy.modal =
            crate::components::selectable_item_modal::SelectableItemModal::builder()
                .items(entries)
                .scopes(scopes)
                .id(|e| {
                    if e.stable_id.is_empty() {
                        e.name.clone()
                    } else {
                        e.stable_id.clone()
                    }
                })
                .label(|e| e.name.clone())
                .description(|e| e.source.clone())
                .build()
                .ok();
    }

    pub(super) fn handle_tool_policy_key(&mut self, key: &Key) {
        use crate::components::selectable_item_modal::{ScopeSelection, SelectableItemModalResult};
        let Some(modal) = &mut self.tool_policy.modal else {
            return;
        };
        modal.handle_input(key);
        match modal.take_result() {
            SelectableItemModalResult::AppliedScopes(scopes) => {
                self.tool_policy.modal = None;
                let mutations = scopes
                    .into_iter()
                    .map(
                        |(tool_id, scope)| crate::protocol::client::ToolPolicyMutation {
                            tool_id: Some(tool_id),
                            name: None,
                            reason: Some("tui tool policy modal".into()),
                            scope: match scope {
                                ScopeSelection::None => crate::protocol::client::ToolScope::None,
                                ScopeSelection::Parent => {
                                    crate::protocol::client::ToolScope::Parent
                                }
                                ScopeSelection::Child => crate::protocol::client::ToolScope::Child,
                                ScopeSelection::Both => crate::protocol::client::ToolScope::Both,
                            },
                        },
                    )
                    .collect();
                self.send_command(Command::SetToolPolicy {
                    id: Some("tool-policy-apply".into()),
                    mutations,
                    mode: crate::protocol::client::ToolPolicyApplyMode::ImmediateIfIdle,
                });
            }
            SelectableItemModalResult::Dismissed => self.tool_policy.modal = None,
            _ => {}
        }
    }
}

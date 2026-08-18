use std::collections::BTreeMap;

use super::*;
use crate::components::selectable_item_modal::{
    ScopeSelection, SearchFieldWeights, SelectableItemModal, SelectableItemModalResult,
};
use crate::protocol::client::{
    Command, ToolCatalogueEntry, ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyOperation,
    ToolScope,
};

impl From<ToolScope> for ScopeSelection {
    fn from(value: ToolScope) -> Self {
        match value {
            ToolScope::None => Self::None,
            ToolScope::Parent => Self::Parent,
            ToolScope::Child => Self::Child,
            ToolScope::Both => Self::Both,
        }
    }
}

impl From<ScopeSelection> for ToolScope {
    fn from(value: ScopeSelection) -> Self {
        match value {
            ScopeSelection::None => Self::None,
            ScopeSelection::Parent => Self::Parent,
            ScopeSelection::Child => Self::Child,
            ScopeSelection::Both => Self::Both,
        }
    }
}

impl App {
    pub(super) fn open_tool_policy_modal(&mut self) {
        let id = self.ac().namespaced_id(&format!(
            "tool-policy-catalogue-{}",
            super::app_events::uuid_like()
        ));
        self.tool_policy_modal_pending_catalogue_id = Some(id.clone());
        self.send_command(Command::GetToolCatalogue { id: Some(id) });
        self.notify("Requested tool catalogue", NotifyLevel::Info);
    }

    pub(super) fn is_pending_tool_policy_catalogue_response(&self, id: Option<&str>) -> bool {
        self.tool_policy_modal_pending_catalogue_id.as_deref() == id
    }

    pub(super) fn open_pending_tool_policy_modal_after_catalogue_update(&mut self) {
        if self.tool_policy_modal_pending_catalogue_id.take().is_some() {
            self.open_tool_policy_modal_now();
        }
    }

    pub(super) fn open_tool_policy_modal_now(&mut self) {
        let mut entries = self.tool_catalogue.values().cloned().collect::<Vec<_>>();
        entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.stable_id.cmp(&b.stable_id)));
        let scopes = entries
            .iter()
            .map(|entry| {
                let id = catalogue_key(entry);
                let scope = entry
                    .profile_scope
                    .or_else(|| entry.profile_enabled.map(legacy_profile_enabled_scope))
                    .unwrap_or(ToolScope::Both)
                    .into();
                (id, scope)
            })
            .collect::<BTreeMap<_, _>>();
        match SelectableItemModal::builder()
            .items(entries)
            .id(|entry: &ToolCatalogueEntry| catalogue_key(entry))
            .label(|entry| entry.name.clone())
            .description(|entry| entry.source.clone())
            .search_metadata(|entry| vec![entry.stable_id.clone()])
            .search_weights(SearchFieldWeights::tool_lookup())
            .build()
        {
            Ok(modal) => {
                self.tool_policy_modal = Some(
                    modal
                        .with_scope_selection(scopes)
                        .with_space_toggle_while_filtering(),
                )
            }
            Err(error) => self.notify(
                &format!("Tool policy modal unavailable: {error}"),
                NotifyLevel::Error,
            ),
        }
    }

    pub(super) fn handle_tool_policy_modal_key(&mut self, key: &Key) {
        let Some(modal) = &mut self.tool_policy_modal else {
            return;
        };
        modal.handle_input(key);
        match modal.take_result() {
            SelectableItemModalResult::AppliedScopes(scopes) => {
                self.tool_policy_modal = None;
                let mutations = scopes
                    .into_iter()
                    .filter_map(|(id, scope)| {
                        self.tool_catalogue
                            .get(&id)
                            .map(|entry| ToolPolicyMutation {
                                tool_id: (!entry.stable_id.is_empty())
                                    .then(|| entry.stable_id.clone()),
                                name: Some(entry.name.clone()),
                                scope: scope.into(),
                                reason: Some("tui tool policy modal".into()),
                            })
                    })
                    .collect::<Vec<_>>();
                self.send_command(Command::SetToolPolicy {
                    id: Some(self.ac().namespaced_id("tool-policy-apply")),
                    mutations,
                    mode: ToolPolicyApplyMode::ImmediateIfIdle,
                    operation: ToolPolicyOperation::Replace,
                    unlisted_scope: Some(ToolScope::None),
                    persist: true,
                });
            }
            SelectableItemModalResult::Dismissed => {
                self.tool_policy_modal = None;
                self.tool_policy_modal_pending_catalogue_id = None;
            }
            _ => {}
        }
    }
}

fn catalogue_key(entry: &ToolCatalogueEntry) -> String {
    if entry.stable_id.is_empty() {
        entry.name.clone()
    } else {
        entry.stable_id.clone()
    }
}

pub(super) fn legacy_profile_enabled_scope(enabled: bool) -> ToolScope {
    if enabled {
        ToolScope::Both
    } else {
        ToolScope::None
    }
}

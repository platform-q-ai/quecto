//! Panel row model for the sub-agent side panel: the flattened master +
//! tree + environment-group row listing (#1369 slice 4), split from
//! `controller_subagent_panel.rs` (750-line cap).

use super::*;

impl App {
    /// Flattened panel rows: the master pinned at the top, then the sub-agent
    /// tree depth-ordered by `parent_id` (grandchildren under their parent).
    pub(super) fn panel_rows(&self) -> Vec<PanelRow> {
        let master_wf = {
            let wf = &self.conn.master_session.workflow_bar;
            (wf.total > 0).then_some((wf.done, wf.total))
        };
        let mut rows = vec![PanelRow {
            id: None,
            env_key: None,
            prefix: String::new(),
            label: "Master Agent".to_string(),
            status: self.master_status().to_string(),
            workflow: master_wf,
        }];
        let groups = self.environment_groups();
        for (node, prefix) in self.subagent_tree_order(&groups) {
            match node {
                PanelNode::Environment(env_key) => {
                    // One selectable row naming the shared environment; its
                    // members nest below via the tree walk (#1369 slice 4).
                    // The node key is the grouping identity (uuid when
                    // reported, review #1392); the painted label is the ref.
                    let env = self.environment_info(&env_key);
                    let env_ref = env
                        .map(|e| e.environment_ref.clone())
                        .unwrap_or_else(|| env_key.clone());
                    let name = env
                        .and_then(|e| e.name.as_deref())
                        .filter(|n| !n.is_empty());
                    let label = match name {
                        Some(name) => format!("{env_ref} {name}"),
                        None => env_ref,
                    };
                    rows.push(PanelRow {
                        label,
                        // Worst-wins across members (review #1392): a stale
                        // forwarded copy must not paint a dying env `running`.
                        status: self.environment_status(&env_key),
                        workflow: None,
                        id: None,
                        env_key: Some(env_key),
                        prefix,
                    });
                }
                PanelNode::Agent(id) => {
                    let info = self.subagents.tracked.get(&id).map(|t| &t.info);
                    let workflow = info
                        .and_then(|i| i.workflow.as_ref())
                        .filter(|w| w.steps_total > 0)
                        .map(|w| (w.steps_completed, w.steps_total));
                    // Store/select by durable UUID identity; paint the human display
                    // label (display_name / compatibility agentId) in the panel (#1378).
                    let label = info
                        .map(|i| {
                            i.display_name
                                .as_deref()
                                .filter(|s| !s.is_empty())
                                .unwrap_or(i.agent_id.as_str())
                                .to_string()
                        })
                        .unwrap_or_else(|| id.clone());
                    rows.push(PanelRow {
                        label,
                        status: info.map(|i| i.status.clone()).unwrap_or_default(),
                        workflow,
                        id: Some(id),
                        env_key: None,
                        prefix,
                    });
                }
            }
        }
        rows
    }

    /// Depth-first `(node, tree_prefix)` listing of the sub-agent tree. Root
    /// sub-agents (no in-map parent) sit under the master; `tree_prefix` is the
    /// connector stalk (`├ `/`└ ` with `│ `/`  ` ancestor continuation) so the
    /// panel draws tree lines back up to each parent. Order follows sorted ids.
    ///
    /// Every script-managed environment (#1369 slice 4, follow-up revision:
    /// solo environments included) contributes one environment node after the
    /// agent roots, with the member agents as its children — suppressed from
    /// the root list so no member is duplicated.
    fn subagent_tree_order(
        &self,
        groups: &std::collections::BTreeMap<String, Vec<String>>,
    ) -> Vec<(PanelNode, String)> {
        use std::collections::{BTreeMap, BTreeSet};
        let grouped: BTreeSet<&str> = groups.values().flatten().map(String::as_str).collect();
        // Parent key: `None` = under the master; `Some(key)` = under the node
        // with that key (an agent id, or an environment node key for grouped
        // members). Environment node keys can never collide with sanitized
        // agent ids because of the `\0` byte.
        let mut children: BTreeMap<Option<String>, Vec<PanelNode>> = BTreeMap::new();
        for (id, tracked) in &self.subagents.tracked {
            let parent = if grouped.contains(id.as_str()) {
                // Grouped members always nest under their environment row.
                tracked
                    .info
                    .environment
                    .as_ref()
                    .map(|e| PanelNode::env_key(e.group_key()))
            } else {
                // Treat an unknown parent as a root so its subtree is not lost.
                tracked
                    .info
                    .parent_id
                    .clone()
                    .filter(|p| self.subagents.tracked.contains_key(p))
            };
            children
                .entry(parent)
                .or_default()
                .push(PanelNode::Agent(id.clone()));
        }
        // Push siblings reversed (with their connector) so popping preserves order.
        // Stack item: (node, own_prefix, descendant_continuation_prefix).
        let push_children =
            |stack: &mut Vec<(PanelNode, String, String)>, kids: &[PanelNode], cont: &str| {
                let n = kids.len();
                for (i, kid) in kids.iter().enumerate().rev() {
                    let last = i == n - 1;
                    stack.push((
                        kid.clone(),
                        format!("{cont}{}", if last { "└ " } else { "├ " }),
                        format!("{cont}{}", if last { "  " } else { "│ " }),
                    ));
                }
            };
        let mut roots: Vec<PanelNode> = children.remove(&None).unwrap_or_default();
        roots.extend(groups.keys().map(|r| PanelNode::Environment(r.clone())));
        let mut out = Vec::new();
        let mut stack: Vec<(PanelNode, String, String)> = Vec::new();
        push_children(&mut stack, &roots, "");
        while let Some((node, own_prefix, cont)) = stack.pop() {
            if let Some(kids) = children.get(&Some(node.child_key())) {
                push_children(&mut stack, kids, &cont);
            }
            out.push((node, own_prefix));
        }
        out
    }
}

pub(super) struct PanelRow {
    pub(super) id: Option<String>,
    /// `Some(group key)` when this is a selectable environment row for a
    /// shared environment (#1369 slice 4); `id` is `None` for such rows. The
    /// key is the grouping identity (environment uuid when reported, else the
    /// session-scoped ref — review #1392), not necessarily the painted ref.
    pub(super) env_key: Option<String>,
    /// Tree connector stalk drawn before the name (`├ `/`└ ` + ancestor `│ `).
    pub(super) prefix: String,
    pub(super) label: String,
    pub(super) status: String,
    /// `(steps_completed, steps_total)` when the agent has an active workflow —
    /// drives the per-step progress bar drawn beneath the name row.
    pub(super) workflow: Option<(u32, u32)>,
}

impl PanelRow {
    /// Whether this is a selectable environment row (not master, not an agent).
    pub(super) fn is_environment(&self) -> bool {
        self.id.is_none() && self.env_key.is_some()
    }
}

/// One node in the panel tree walk: a tracked agent, or a shared-environment
/// grouping row (#1369 slice 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PanelNode {
    Agent(String),
    Environment(String),
}

impl PanelNode {
    /// Parent-map key for members nesting under an environment node. The `\0`
    /// byte cannot appear in sanitized agent ids, so environment keys can
    /// never collide with them.
    fn env_key(env_ref: &str) -> String {
        format!("\0env:{env_ref}")
    }

    /// The key this node's children are registered under in the parent map.
    fn child_key(&self) -> String {
        match self {
            Self::Agent(id) => id.clone(),
            Self::Environment(env_ref) => Self::env_key(env_ref),
        }
    }
}

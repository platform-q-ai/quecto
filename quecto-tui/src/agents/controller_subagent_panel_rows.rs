#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PanelRowKind {
    Master,
    Agent(String),
    Container(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PanelRow {
    pub(super) kind: PanelRowKind,
    pub(super) id: Option<String>,
    /// Tree connector stalk drawn before the name (`├ `/`└ ` + ancestor `│ `).
    pub(super) prefix: String,
    pub(super) label: String,
    pub(super) status: String,
    /// `(steps_completed, steps_total)` when the agent has an active workflow —
    /// drives the per-step progress bar drawn beneath the name row.
    pub(super) workflow: Option<(u32, u32)>,
}

use crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope;
use serde::{Deserialize, Deserializer, Serialize};

/// One image on `prompt` / `steer` / `follow_up` as spelled on the wire
/// (#2422): `{"mimeType", "data"}`, admitted by `quecto_image`.
pub use quecto_image::ImagePayload;

#[derive(Debug, Clone, Serialize)]
pub struct PresentJsonValue(Option<serde_json::Value>);

fn present_json_value<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PresentJsonValue>, D::Error> {
    Option::<serde_json::Value>::deserialize(deserializer)
        .map(|value| Some(PresentJsonValue(value)))
}

/// Discovery scope of `list_sessions` (#2009) as spelled on the wire; the
/// dispatch edge maps it onto the application's scope.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionListScopeCommand {
    #[default]
    Local,
    Global,
}

impl From<SessionListScopeCommand> for crate::application::sessions::dto::SessionListScope {
    fn from(scope: SessionListScopeCommand) -> Self {
        match scope {
            SessionListScopeCommand::Local => Self::Local,
            SessionListScopeCommand::Global => Self::Global,
        }
    }
}

// ─── Commands (stdin) ────────────────────────────────────────────────────────
/// A command received over the UDS socket.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentCommand {
    /// Send a user message to the agent.
    Prompt {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
        /// Images attached to the message (#2422), validated at dispatch;
        /// `null` is no images.
        #[serde(
            default,
            deserialize_with = "quecto_image::images_or_null",
            skip_serializing_if = "Vec::is_empty"
        )]
        images: Vec<ImagePayload>,
        /// Required when the agent is currently running.
        #[serde(rename = "streamingBehavior", skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<StreamingBehavior>,
    },
    /// Interrupt after the current tool, then deliver this message.
    Steer {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
        /// Images attached to the message (#2422), validated at dispatch;
        /// `null` is no images.
        #[serde(
            default,
            deserialize_with = "quecto_image::images_or_null",
            skip_serializing_if = "Vec::is_empty"
        )]
        images: Vec<ImagePayload>,
    },
    /// Deliver this message when the agent finishes.
    FollowUp {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
        /// Images attached to the message (#2422), validated at dispatch;
        /// `null` is no images.
        #[serde(
            default,
            deserialize_with = "quecto_image::images_or_null",
            skip_serializing_if = "Vec::is_empty"
        )]
        images: Vec<ImagePayload>,
    },
    /// Cancel the current agent run.
    Abort {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Return current session state.
    GetState {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<u64>,
        #[serde(rename = "agent_id", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    /// Return conversation history. Optional `count` returns the last N messages.
    ///
    /// When `agent_id` is set, the request is forwarded to that spawned
    /// sub-agent and its history is returned instead of the connected agent's
    /// own. Omit `count` for the default protocol page, set `count` for an
    /// older-client newest-slice request, and set `before` to page backward.
    /// It is never silently answered from the connected/parent agent's history.
    GetReport {
        #[serde(default)]
        export_raw: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    GetMessages {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(rename = "agent_id", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    /// Return committed transcript changes after `sinceRev` in `epoch`.
    Sync {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        epoch: u64,
        #[serde(rename = "sinceRev")]
        since_rev: u64,
        #[serde(rename = "agent_id", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    /// Deprecated alias for `get_messages` with `count`.
    ///
    /// When `agent_id` is set, the request is forwarded to that spawned
    /// sub-agent (reusing the `agent_cmd get_messages` capability) and its
    /// message tail is returned instead of the connected agent's own history.
    GetMessagesTail {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        count: usize,
        #[serde(rename = "agent_id", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    /// Return token usage and cost statistics.
    GetSessionStats {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Persist the current session immediately, optionally marking subagents
    /// with an explicit restore reason for ordinary-exit barriers.
    PersistSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(
            rename = "restoreReason",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        restore_reason: Option<String>,
    },
    /// Return configured and built-in models from the runtime registry.
    ListModels {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Refresh the refreshable catalogue sources (all, or one named source)
    /// through the application refresh use case and report per-source
    /// outcomes. The only operation that touches the network.
    RefreshModels {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    /// Return persisted CLI sessions available for resume.
    ListSessions {
        #[serde(default)]
        scope: SessionListScopeCommand,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Search saved-session metadata (#2010): title, exact key, repository
    /// label and path — literal text, never a pattern, never transcript
    /// content. `generation` is the client's own counter, echoed unchanged.
    /// `generation` and `limit` are decoded as any JSON value (R1-H5): the
    /// edge brings a number into range and answers anything else with a
    /// correlated refusal, so a client awaiting its id never hangs on them.
    SearchSessionMetadata {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        query: String,
        #[serde(default)]
        scope: SessionListScopeCommand,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        generation: serde_json::Value,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        limit: serde_json::Value,
    },
    /// Switch to a fresh user-chat session.
    NewSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Switch the active UDS session to a persisted session by exact key.
    /// `action` is retained only as a presence-sensitive compatibility field:
    /// every supplied value, including JSON null, receives a correlated refusal.
    ResumeSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        session: String,
        #[serde(
            default,
            deserialize_with = "present_json_value",
            skip_serializing_if = "Option::is_none"
        )]
        action: Option<PresentJsonValue>,
        #[serde(
            default,
            rename = "expectedHomeVersion",
            skip_serializing_if = "Option::is_none"
        )]
        expected_home_version: Option<String>,
    },
    /// Switch the active model at runtime.
    ///
    /// Accepts either:
    /// - legacy `{ "model": "provider/modelId" }`, or
    /// - compatible `{ "provider": "...", "modelId": "..." }`.
    ///
    /// With `persist` (`"local"` | `"global"`, #2024 S2) the resolved
    /// qualified id is also recorded as `agents.defaults.model` of that
    /// configuration layer; absent, the switch is in-memory only.
    SetModel {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(rename = "modelId", skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        persist: Option<String>,
    },
    /// Switch the active reasoning-effort level at runtime (#1067).
    /// Session-scoped: validated against the active model's provider
    /// vocabulary and applied to every subsequent turn. With `persist`
    /// (`"local"` | `"global"`, #2024 S2) the level is also recorded as
    /// `agents.defaults.effort` of that configuration layer.
    SetEffort {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        effort: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        persist: Option<String>,
    },
    /// Return the complete rich tool catalogue for control/query clients.
    #[serde(alias = "list_tools")]
    GetToolCatalogue {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Mutate profile-owned live tool policy through the catalogue-backed path.
    #[serde(rename_all = "camelCase")]
    SetToolPolicy {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        mutations: Vec<ToolPolicyMutationCommand>,
        #[serde(default = "default_tool_policy_apply_mode")]
        mode: ToolPolicyApplyModeCommand,
        #[serde(default = "default_tool_policy_operation")]
        operation: ToolPolicyOperationCommand,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unlisted_scope: Option<ProfileAvailabilityScope>,
        /// Persist successful choices to tools.policy in the active config.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        persist: bool,
    },
    /// Force a provider/model config reload.
    Reload {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Register tools provided by an extension client.
    RegisterTools {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        tools: Vec<ToolRegistration>,
    },
    /// Remove previously registered tools.
    UnregisterTools {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        tools: Vec<String>,
    },
    /// Return a tool execution result (response to an `execute_tool` event).
    ToolResult {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        content: String,
        #[serde(rename = "isError", default)]
        is_error: bool,
        /// Images the tool returned (#2423): `[{"mimeType","data"}]`, kept
        /// as sent and admitted when the result is delivered, so a malformed
        /// entry still resolves the call (as an error result naming it);
        /// absent or `null` is no images.
        #[serde(
            rename = "imageBlocks",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        image_blocks: Option<WireImageBlocks>,
    },
    /// Clear conversation history in-place without restarting the agent.
    ClearHistory {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Rewind conversation history to a selected user-message boundary.
    ///
    /// Prefer `messageId`. `messageIndex` is retained for one-window-older
    /// clients (#1059) and honoured only while the conversation fits in one
    /// history page; beyond that it is rejected as ambiguous.
    RewindTo {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(
            rename = "messageIndex",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        message_index: Option<usize>,
        #[serde(rename = "messageId", default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    /// Toggle core workflow automation for this UDS session.
    SetWorkflowAutomation {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "autoContinue", skip_serializing_if = "Option::is_none")]
        auto_continue: Option<bool>,
        #[serde(rename = "completionNudge", skip_serializing_if = "Option::is_none")]
        completion_nudge: Option<bool>,
    },
    /// Return the current list of spawned subagents and their live status (#524).
    GetSubagents {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<u64>,
    },
    /// Terminate and remove every tracked sub-agent.
    DeleteAllSubagents {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    GetMessage {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Stable domain message UUID (wire: camelCase `messageId`).
        #[serde(rename = "messageId")]
        message_id: String,
        /// When set, forward lookup to a spawned child (same as get_messages).
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        /// Select a tool call whose arguments should be recovered instead of message content.
        #[serde(
            rename = "toolCallId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        tool_call_id: Option<String>,
        /// Byte offset for ranged content or tool-call argument recovery (#1094/#1107).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<usize>,
        /// Independent byte offset for ranged visible-thinking recovery (#1231).
        #[serde(
            rename = "thinkingOffset",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        thinking_offset: Option<usize>,
        /// Maximum bytes of content to return for ranged recovery (#1094).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
}
/// A `tool_result`'s `imageBlocks` as read off the wire (#2423), bounded
/// (review M1): a list keeps at most one entry past
/// [`quecto_image::MAX_IMAGES_PER_MESSAGE`], enough to refuse it, and
/// only counts the rest without keeping them; anything else but `null`
/// (absent) is not a list. Admitted when the result is delivered. Read
/// straight from the line (the tool_result intercept), the list streams;
/// through the tagged `AgentCommand` the line is buffered first (#2432).
#[derive(Debug, Clone, PartialEq)]
pub enum WireImageBlocks {
    /// A list: its first entries and how many it had.
    List {
        kept: Vec<serde_json::Value>,
        len: usize,
    },
    /// Any value that is not a list.
    NotAList,
}

impl<'de> Deserialize<'de> for WireImageBlocks {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(WireImageBlocksVisitor)
    }
}

struct WireImageBlocksVisitor;

impl WireImageBlocksVisitor {
    fn not_a_list<E>(self) -> Result<WireImageBlocks, E> {
        Ok(WireImageBlocks::NotAList)
    }
}

impl<'de> serde::de::Visitor<'de> for WireImageBlocksVisitor {
    type Value = WireImageBlocks;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut kept = Vec::new();
        while kept.len() <= quecto_image::MAX_IMAGES_PER_MESSAGE {
            match seq.next_element::<serde_json::Value>()? {
                Some(entry) => kept.push(entry),
                None => {
                    let len = kept.len();
                    return Ok(WireImageBlocks::List { kept, len });
                }
            }
        }
        let mut len = kept.len();
        while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
            len += 1;
        }
        Ok(WireImageBlocks::List { kept, len })
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map
            .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
            .is_some()
        {}
        self.not_a_list()
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(WireImageBlocks::List {
            kept: Vec::new(),
            len: 0,
        })
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        self.not_a_list()
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        self.not_a_list()
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        self.not_a_list()
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        self.not_a_list()
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        self.not_a_list()
    }
}

/// Writes the entries kept (all of a list within the limit); a value that
/// was not a list is written as `{}`, which reads back as not a list.
impl Serialize for WireImageBlocks {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::List { kept, .. } => kept.serialize(serializer),
            Self::NotAList => serde_json::Map::new().serialize(serializer),
        }
    }
}

/// Tool registration payload for `register_tools`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolRegistration {
    pub name: String,
    pub description: String,
    #[serde(rename = "parametersSchema", default = "default_params_schema")]
    pub parameters_schema: String,
    #[serde(rename = "stableId", default, skip_serializing_if = "Option::is_none")]
    pub stable_id: Option<String>,
    /// How long the agent waits for this tool's result, in whole seconds
    /// (#2423): 1 to 600; absent or `null` is 30. Kept as sent, so any
    /// other value refuses the registration with an exact message rather
    /// than failing the whole command's parse.
    #[serde(
        rename = "timeoutSeconds",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_seconds: Option<serde_json::Value>,
}
fn default_params_schema() -> String {
    r#"{"type":"object"}"#.to_string()
}

/// One requested catalogue-backed tool policy mutation.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyMutationCommand {
    /// Stable catalogue id when known by the caller. Current registry mutation
    /// keys are tool names, so this is accepted as an alias for `name` until the
    /// domain mutator grows stable-id addressing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub scope: ProfileAvailabilityScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Wire spelling for live tool policy reconciliation timing.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyApplyModeCommand {
    ImmediateIfIdle,
    AtNextTurnBoundary,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyOperationCommand {
    Patch,
    Replace,
}

fn default_tool_policy_apply_mode() -> ToolPolicyApplyModeCommand {
    ToolPolicyApplyModeCommand::ImmediateIfIdle
}

fn default_tool_policy_operation() -> ToolPolicyOperationCommand {
    ToolPolicyOperationCommand::Patch
}

impl AgentCommand {
    /// Return the optional correlation id.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Prompt { id, .. } => id.as_deref(),
            Self::Steer { id, .. } => id.as_deref(),
            Self::FollowUp { id, .. } => id.as_deref(),
            Self::Abort { id } => id.as_deref(),
            Self::GetState { id, .. } => id.as_deref(),
            Self::GetReport { id, .. } => id.as_deref(),
            Self::GetMessages { id, .. } => id.as_deref(),
            Self::Sync { id, .. } => id.as_deref(),
            Self::GetToolCatalogue { id } => id.as_deref(),
            Self::SetToolPolicy { id, .. } => id.as_deref(),
            Self::Reload { id } => id.as_deref(),
            Self::GetMessagesTail { id, .. } => id.as_deref(),
            Self::GetSessionStats { id } => id.as_deref(),
            Self::PersistSession { id, .. } => id.as_deref(),
            Self::ListModels { id } => id.as_deref(),
            Self::RefreshModels { id, .. } => id.as_deref(),
            Self::ListSessions { id, .. } => id.as_deref(),
            Self::SearchSessionMetadata { id, .. } => id.as_deref(),
            Self::NewSession { id } => id.as_deref(),
            Self::ResumeSession { id, .. } => id.as_deref(),
            Self::SetModel { id, .. } => id.as_deref(),
            Self::SetEffort { id, .. } => id.as_deref(),
            Self::RegisterTools { id, .. } => id.as_deref(),
            Self::UnregisterTools { id, .. } => id.as_deref(),
            Self::ToolResult { .. } => None,
            Self::ClearHistory { id } => id.as_deref(),
            Self::RewindTo { id, .. } => id.as_deref(),
            Self::SetWorkflowAutomation { id, .. } => id.as_deref(),
            Self::GetSubagents { id, .. } => id.as_deref(),
            Self::DeleteAllSubagents { id } => id.as_deref(),
            Self::GetMessage { id, .. } => id.as_deref(),
        }
    }
    /// Return the command name string for use in responses.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::Abort { .. } => "abort",
            Self::GetState { .. } => "get_state",
            Self::GetReport { .. } => "get_report",
            Self::GetMessages { .. } => "get_messages",
            Self::Sync { .. } => "sync",
            Self::GetMessagesTail { .. } => "get_messages_tail",
            Self::GetSessionStats { .. } => "get_session_stats",
            Self::PersistSession { .. } => "persist_session",
            Self::ListModels { .. } => "list_models",
            Self::RefreshModels { .. } => "refresh_models",
            Self::ListSessions { .. } => "list_sessions",
            Self::SearchSessionMetadata { .. } => "search_session_metadata",
            Self::NewSession { .. } => "new_session",
            Self::ResumeSession { .. } => "resume_session",
            Self::SetModel { .. } => "set_model",
            Self::SetEffort { .. } => "set_effort",
            Self::GetToolCatalogue { .. } => "get_tool_catalogue",
            Self::SetToolPolicy { .. } => "set_tool_policy",
            Self::Reload { .. } => "reload",
            Self::RegisterTools { .. } => "register_tools",
            Self::UnregisterTools { .. } => "unregister_tools",
            Self::ToolResult { .. } => "tool_result",
            Self::ClearHistory { .. } => "clear_history",
            Self::RewindTo { .. } => "rewind_to",
            Self::SetWorkflowAutomation { .. } => "set_workflow_automation",
            Self::GetSubagents { .. } => "get_subagents",
            Self::DeleteAllSubagents { .. } => "delete_all_subagents",
            Self::GetMessage { .. } => "get_message",
        }
    }
}

/// How to handle a `prompt` command when the agent is already running.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StreamingBehavior {
    /// Interrupt after the current tool; deliver the message next.
    Steer,
    /// Wait until the agent finishes; then deliver the message.
    FollowUp,
}

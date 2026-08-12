use crate::domain::session::PersistedSubagentRosterEntry;
use crate::domain::workflow::WorkflowRunPersisted;

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SessionFile {
    pub(super) key: String,
    pub(super) messages: Vec<MessageRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) workflow_run: Option<WorkflowRunPersisted>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) subagent_roster: Vec<PersistedSubagentRosterEntry>,
}

#[derive(serde::Serialize)]
pub(super) struct SessionFileRef<'a> {
    pub(super) key: &'a str,
    pub(super) messages: Vec<MessageRecordRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) workflow_run: Option<&'a WorkflowRunPersisted>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub(super) subagent_roster: &'a [PersistedSubagentRosterEntry],
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub(super) enum SessionRecord {
    #[serde(rename = "snapshot")]
    Snapshot(SessionFile),
    #[serde(rename = "append")]
    Append {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_index: Option<usize>,
        messages: Vec<MessageRecord>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow_run: Option<WorkflowRunPersisted>,
        #[serde(default, skip_serializing_if = "skip_if_false")]
        workflow_run_cleared: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_roster: Option<Vec<PersistedSubagentRosterEntry>>,
    },
}

#[derive(serde::Serialize)]
#[serde(tag = "type")]
pub(super) enum SessionRecordRef<'a> {
    #[serde(rename = "snapshot")]
    Snapshot(SessionFileRef<'a>),
    #[serde(rename = "append")]
    Append {
        #[serde(skip_serializing_if = "Option::is_none")]
        start_index: Option<usize>,
        messages: Vec<MessageRecordRef<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workflow_run: Option<&'a WorkflowRunPersisted>,
        #[serde(skip_serializing_if = "skip_if_false")]
        workflow_run_cleared: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        subagent_roster: Option<&'a [PersistedSubagentRosterEntry]>,
    },
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(super) struct MessageRecord {
    pub(super) role: String,
    pub(super) content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) tool_calls: Vec<ToolCallRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tool_call_id: Option<String>,
    // Context-pruning metadata (all optional for backward compat)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) turn: Option<u32>,
    /// `None` = absent in old files (use constructor default);
    /// `Some(true/false)` = explicitly persisted value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) is_pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "skip_if_false")]
    pub(super) is_manifest: bool,
    #[serde(default, skip_serializing_if = "skip_if_false")]
    pub(super) is_collapsed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) input_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) spill_id: Option<String>,
    #[serde(default, skip_serializing_if = "skip_if_false")]
    pub(super) is_error: bool,
    /// Stop reason for assistant messages (serialised as raw Anthropic string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) stop_reason: Option<String>,
    /// Extended thinking blocks from assistant messages (#437-5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) thinking_blocks: Vec<ThinkingBlockRecord>,
}

#[derive(serde::Serialize)]
pub(super) struct MessageRecordRef<'a> {
    pub(super) role: &'a str,
    pub(super) content: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) tool_calls: Vec<ToolCallRecordRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) turn: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) is_pinned: Option<bool>,
    #[serde(skip_serializing_if = "skip_if_false")]
    pub(super) is_manifest: bool,
    #[serde(skip_serializing_if = "skip_if_false")]
    pub(super) is_collapsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_preview: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) spill_id: Option<&'a str>,
    #[serde(skip_serializing_if = "skip_if_false")]
    pub(super) is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) thinking_blocks: Vec<ThinkingBlockRecordRef<'a>>,
}

pub(super) fn skip_if_false(v: &bool) -> bool {
    !v
}

/// Lightweight view of a session file used by `list()` to derive the title,
/// message count and key without constructing full message records (#765).
/// The `session_header_*` tests pin the shared field names so this independent
/// serde view cannot silently drift from [`SessionFile`] / [`MessageRecord`].
#[derive(serde::Deserialize)]
pub(super) struct SessionHeader<'a> {
    #[serde(borrow)]
    pub(super) key: std::borrow::Cow<'a, str>,
    #[serde(default, borrow)]
    pub(super) messages: Vec<MessageHeader<'a>>,
}

/// Per-message header: just the role (for counting/title selection) and the
/// content (for the title). Every other field is ignored by serde.
#[derive(serde::Deserialize)]
pub(super) struct MessageHeader<'a> {
    #[serde(borrow)]
    pub(super) role: std::borrow::Cow<'a, str>,
    #[serde(default, borrow)]
    pub(super) content: std::borrow::Cow<'a, str>,
}

/// Uses the same strings that `StopReason::parse` accepts so that
/// round-trips are lossless regardless of which provider produced the value.

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(super) struct ToolCallRecord {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) arguments: String,
}

#[derive(serde::Serialize)]
pub(super) struct ToolCallRecordRef<'a> {
    pub(super) id: &'a str,
    pub(super) name: &'a str,
    pub(super) arguments: &'a str,
}

/// Serializable representation of a thinking block (#437-5).
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub(super) enum ThinkingBlockRecord {
    /// Normal thinking block with visible reasoning text and signature.
    #[serde(rename = "normal")]
    Normal { thinking: String, signature: String },
    /// Redacted thinking block (reasoning hidden by safety filters).
    #[serde(rename = "redacted")]
    Redacted { data: String },
}

#[derive(serde::Serialize)]
#[serde(tag = "type")]
pub(super) enum ThinkingBlockRecordRef<'a> {
    #[serde(rename = "normal")]
    Normal {
        thinking: &'a str,
        signature: &'a str,
    },
    #[serde(rename = "redacted")]
    Redacted { data: &'a str },
}

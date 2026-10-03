use crate::domain::conversation::stored_images::{ImageRef, MessageImageRefs};
use crate::domain::message::{Message, Role, StopReason, ThinkingBlock, ToolCall};
use crate::domain::session::PersistedSubagentRosterEntry;
use crate::domain::workflow::WorkflowRunPersisted;
use crate::infrastructure::persistence::session_images::ImageRefRecord;
use crate::infrastructure::turn_origin_names::{self as names, origin_from_name, origin_name};
use serde::Deserialize;
use std::collections::BTreeSet;

use super::{role_to_str, str_to_role};

fn deserialize_subagent_roster_lossy<'de, D>(
    deserializer: D,
) -> Result<Vec<PersistedSubagentRosterEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let rows = Option::<Vec<serde_json::Value>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(rows
        .into_iter()
        .filter_map(|row| serde_json::from_value(row).ok())
        .collect())
}

fn deserialize_optional_subagent_roster_lossy<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<PersistedSubagentRosterEntry>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Vec<serde_json::Value>>::deserialize(deserializer).map(|rows| {
        rows.map(|rows| {
            rows.into_iter()
                .filter_map(|row| serde_json::from_value(row).ok())
                .collect()
        })
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SessionFile {
    pub(super) key: String,
    pub(super) messages: Vec<MessageRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) workflow_run: Option<WorkflowRunPersisted>,
    #[serde(
        default,
        deserialize_with = "deserialize_subagent_roster_lossy",
        skip_serializing_if = "Vec::is_empty"
    )]
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
        #[serde(
            default,
            deserialize_with = "deserialize_optional_subagent_roster_lossy",
            skip_serializing_if = "Option::is_none"
        )]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ordinal: Option<u64>,
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
    pub(super) turn_origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) user_kind: Option<String>,
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
    /// A tool result's images, by reference to their sidecars (#2424).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) images: Vec<ImageRefRecord>,
    /// A user message's images, by reference to their sidecars (#2424).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) user_images: Vec<ImageRefRecord>,
}

#[derive(serde::Serialize)]
pub(super) struct MessageRecordRef<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ordinal: Option<u64>,
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
    pub(super) turn_origin: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) user_kind: Option<&'static str>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) images: Vec<ImageRefRecord>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) user_images: Vec<ImageRefRecord>,
}

/// The digests of the images `records` name.
pub(super) fn record_digests(records: &[MessageRecordRef<'_>]) -> BTreeSet<String> {
    records
        .iter()
        .flat_map(|record| record.images.iter().chain(&record.user_images))
        .map(|image| image.sha256.clone())
        .collect()
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
    /// A Responses API reasoning item, replayed to its model (#2162).
    #[serde(rename = "encrypted_reasoning")]
    EncryptedReasoning {
        origin: String,
        #[serde(default)]
        leads_to: Option<String>,
        item: String,
    },
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
    #[serde(rename = "encrypted_reasoning")]
    EncryptedReasoning {
        origin: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        leads_to: Option<&'a str>,
        item: &'a str,
    },
}

impl<'a> From<&'a ThinkingBlock> for ThinkingBlockRecordRef<'a> {
    fn from(block: &'a ThinkingBlock) -> Self {
        match block {
            ThinkingBlock::Normal {
                thinking,
                signature,
            } => Self::Normal {
                thinking,
                signature,
            },
            ThinkingBlock::Redacted { data } => Self::Redacted { data },
            ThinkingBlock::EncryptedReasoning {
                origin,
                leads_to,
                item,
            } => Self::EncryptedReasoning {
                origin,
                leads_to: leads_to.as_deref(),
                item,
            },
        }
    }
}

impl From<&ThinkingBlock> for ThinkingBlockRecord {
    fn from(block: &ThinkingBlock) -> Self {
        match ThinkingBlockRecordRef::from(block) {
            ThinkingBlockRecordRef::Normal {
                thinking,
                signature,
            } => Self::Normal {
                thinking: thinking.to_string(),
                signature: signature.to_string(),
            },
            ThinkingBlockRecordRef::Redacted { data } => Self::Redacted {
                data: data.to_string(),
            },
            ThinkingBlockRecordRef::EncryptedReasoning {
                origin,
                leads_to,
                item,
            } => Self::EncryptedReasoning {
                origin: origin.to_string(),
                leads_to: leads_to.map(str::to_string),
                item: item.to_string(),
            },
        }
    }
}

impl From<ThinkingBlockRecord> for ThinkingBlock {
    fn from(record: ThinkingBlockRecord) -> Self {
        match record {
            ThinkingBlockRecord::Normal {
                thinking,
                signature,
            } => Self::Normal {
                thinking,
                signature,
            },
            ThinkingBlockRecord::Redacted { data } => Self::Redacted { data },
            ThinkingBlockRecord::EncryptedReasoning {
                origin,
                leads_to,
                item,
            } => Self::EncryptedReasoning {
                origin,
                leads_to,
                item,
            },
        }
    }
}

/// The title-and-count view of a transcript the list reads (#765).
pub(super) fn parse_session_header(data: &str) -> Result<SessionHeader<'_>, serde_json::Error> {
    if let Ok(header) = serde_json::from_str::<SessionHeader<'_>>(data) {
        return Ok(header);
    }

    let mut key = std::borrow::Cow::Borrowed("");
    let mut messages = Vec::new();
    let mut parsed_any = false;
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let record: SessionRecord = match serde_json::from_str(line) {
            Ok(record) => record,
            Err(err) if parsed_any => {
                tracing::warn!(error = %err, "ignoring incomplete trailing session record");
                break;
            }
            Err(err) => return Err(err),
        };
        parsed_any = true;
        match record {
            SessionRecord::Snapshot(file) => {
                key = file.key.into();
                messages = file
                    .messages
                    .into_iter()
                    .map(|message| MessageHeader {
                        role: message.role.into(),
                        content: message.content.into(),
                    })
                    .collect();
            }
            SessionRecord::Append {
                messages: added, ..
            } => {
                messages.extend(added.into_iter().map(|message| MessageHeader {
                    role: message.role.into(),
                    content: message.content.into(),
                }));
            }
        }
    }
    Ok(SessionHeader { key, messages })
}

fn image_records(refs: &[ImageRef]) -> Vec<ImageRefRecord> {
    refs.iter().map(ImageRefRecord::from).collect()
}

/// The record of `msg` as it is written: its images by reference (#2424).
pub(super) fn message_to_record_ref(msg: &Message) -> MessageRecordRef<'_> {
    let images = MessageImageRefs::of(msg);
    MessageRecordRef {
        ordinal: msg.ordinal,
        role: role_to_str(&msg.role),
        content: &msg.content,
        tool_calls: msg
            .tool_calls
            .iter()
            .map(|tc| ToolCallRecordRef {
                id: &tc.id,
                name: &tc.name,
                arguments: &tc.arguments,
            })
            .collect(),
        tool_call_id: msg.tool_call_id.as_deref(),
        turn: msg.turn,
        is_pinned: Some(msg.is_pinned),
        is_manifest: msg.is_manifest,
        is_collapsed: msg.is_collapsed,
        turn_origin: origin_name(msg.turn_origin),
        user_kind: names::user_kind_name(msg.user_kind),
        tool_name: msg.tool_name.as_deref(),
        input_preview: msg.input_preview.as_deref(),
        spill_id: msg.spill_id.as_deref(),
        is_error: msg.is_error,
        stop_reason: msg.stop_reason.as_ref().map(|sr| sr.to_string()),
        thinking_blocks: msg
            .thinking_blocks
            .iter()
            .map(ThinkingBlockRecordRef::from)
            .collect(),
        images: image_records(&images.tool),
        user_images: image_records(&images.user),
    }
}

/// The owned record of a live message, its images hashed into references.
pub(super) fn message_to_record(msg: &Message) -> MessageRecord {
    record_with_images(msg, &MessageImageRefs::of(msg))
}

/// The owned record of `msg` naming `images`: what a message read back
/// from a transcript was written as (its images are still references).
pub(super) fn record_with_images(msg: &Message, images: &MessageImageRefs) -> MessageRecord {
    MessageRecord {
        ordinal: msg.ordinal,
        role: role_to_str(&msg.role).to_string(),
        content: msg.content.clone(),
        tool_calls: msg
            .tool_calls
            .iter()
            .map(|tc| ToolCallRecord {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            })
            .collect(),
        tool_call_id: msg.tool_call_id.clone(),
        turn: msg.turn,
        is_pinned: Some(msg.is_pinned),
        is_manifest: msg.is_manifest,
        is_collapsed: msg.is_collapsed,
        turn_origin: origin_name(msg.turn_origin).map(str::to_string),
        user_kind: names::user_kind_name(msg.user_kind).map(str::to_string),
        tool_name: msg.tool_name.clone(),
        input_preview: msg.input_preview.clone(),
        spill_id: msg.spill_id.clone(),
        is_error: msg.is_error,
        stop_reason: msg.stop_reason.as_ref().map(|sr| sr.to_string()),
        thinking_blocks: msg
            .thinking_blocks
            .iter()
            .map(ThinkingBlockRecord::from)
            .collect(),
        images: image_records(&images.tool),
        user_images: image_records(&images.user),
    }
}

/// The message a record holds, without its images (a reader that never
/// restores them: the bounded read of another agent's transcript).
pub(super) fn record_to_message(rec: MessageRecord) -> Message {
    record_to_message_and_images(rec).0
}

/// The message a record holds, and the references of its images, which the
/// load restores from their sidecars (#2424).
pub(super) fn record_to_message_and_images(rec: MessageRecord) -> (Message, MessageImageRefs) {
    let images = MessageImageRefs {
        tool: rec.images.into_iter().map(Into::into).collect(),
        user: rec.user_images.into_iter().map(Into::into).collect(),
    };
    let tool_calls = rec
        .tool_calls
        .into_iter()
        .map(|tc| ToolCall {
            id: tc.id,
            name: tc.name,
            arguments: tc.arguments,
        })
        .collect();
    let mut msg = match str_to_role(&rec.role) {
        Role::System => Message::system(rec.content),
        Role::User => Message::user(rec.content),
        Role::Assistant => Message::assistant(rec.content, tool_calls),
        Role::Tool => Message::tool(rec.tool_call_id.unwrap_or_default(), rec.content),
    };
    msg.ordinal = rec.ordinal;
    msg.turn = rec.turn;
    msg.is_manifest = rec.is_manifest;
    msg.is_collapsed = rec.is_collapsed;
    msg.turn_origin = origin_from_name(rec.turn_origin.as_deref());
    msg.user_kind = names::user_kind_from_name(rec.user_kind.as_deref());
    msg.tool_name = rec.tool_name;
    msg.input_preview = rec.input_preview;
    msg.spill_id = rec.spill_id;
    msg.is_error = rec.is_error;
    msg.stop_reason = rec.stop_reason.as_deref().map(StopReason::parse);
    msg.is_pinned = rec.is_pinned.unwrap_or(msg.is_pinned);
    msg.thinking_blocks = rec.thinking_blocks.into_iter().map(Into::into).collect();
    (msg, images)
}

#[path = "session_store_bounded.rs"]
pub(in crate::infrastructure::persistence) mod session_store_bounded;

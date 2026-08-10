use crate::conversation::turn_recovery::ordered_by_refs;
use crate::protocol::agent_ledger_payloads::{LedgerMessage, SyncDelta};
use std::collections::HashMap;

pub(crate) const LEDGER_RETAINED_MESSAGE_CAP: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LedgerEntry {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    ToolExecution {
        tool_call_id: String,
        tool_name: String,
        args: String,
        result: Option<String>,
        is_error: bool,
    },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct LedgerTranscript {
    messages: HashMap<String, LedgerMessage>,
    order: Vec<String>,
}

impl LedgerTranscript {
    pub(crate) fn apply_sync_delta(&mut self, delta: &SyncDelta) -> Vec<LedgerEntry> {
        if delta.resync {
            self.messages.clear();
            self.order.clear();
        }
        for message in &delta.messages {
            if let Some(id) = message.id().filter(|s| !s.is_empty()) {
                let id = id.to_string();
                if !self.messages.contains_key(&id) {
                    self.order.push(id.clone());
                }
                self.messages.insert(id, message.clone());
            }
        }
        self.enforce_retention();
        self.entries()
    }

    #[cfg(test)]
    pub(crate) fn retained_message_count(&self) -> usize {
        self.messages.len()
    }

    fn enforce_retention(&mut self) {
        let overflow = self.order.len().saturating_sub(LEDGER_RETAINED_MESSAGE_CAP);
        if overflow == 0 {
            return;
        }
        let dropped: Vec<_> = self.order.drain(0..overflow).collect();
        for id in dropped {
            self.messages.remove(&id);
        }
    }

    /// Current committed projection (order × messages), without mutating state.
    pub(crate) fn entries(&self) -> Vec<LedgerEntry> {
        ledger_entries(&self.order, &self.messages)
    }
}

fn ledger_entries(refs: &[String], responses: &HashMap<String, LedgerMessage>) -> Vec<LedgerEntry> {
    let mut entries = Vec::new();
    let mut tools = HashMap::<String, usize>::new();
    let mut suppressed_calls = std::collections::HashSet::<String>::new();
    // Ordering is the domain's rule, not this function's: walk in ref order,
    // never arrival/map order.
    for message in ordered_by_refs(refs, responses) {
        let content = message.content();
        match message.role() {
            // Sub-agent notes are user-role turns on the wire but operator
            // status in the UI; the live event path already renders them (#1338).
            "user"
                if !content.is_empty()
                    && !crate::protocol::presentation_payloads::is_subagent_note(content) =>
            {
                entries.push(LedgerEntry::User {
                    text: content.to_string(),
                });
            }
            "assistant" => {
                for call in message.tool_calls() {
                    let id = call.id().to_string();
                    if id.is_empty() {
                        continue;
                    }
                    let name = call.name().to_string();
                    if super::suppress_tool_box(&name) {
                        suppressed_calls.insert(id);
                        continue;
                    }
                    let args = call.arguments();
                    tools.insert(id.clone(), entries.len());
                    entries.push(LedgerEntry::ToolExecution {
                        tool_call_id: id,
                        tool_name: name,
                        args,
                        result: None,
                        is_error: false,
                    });
                }
                if !content.is_empty() {
                    entries.push(LedgerEntry::Assistant {
                        text: content.to_string(),
                    });
                }
            }
            "tool" => {
                let call_id = message.tool_call_id().to_string();
                if suppressed_calls.contains(&call_id) {
                    continue;
                }
                let name = message.tool_name().to_string();
                if let Some(idx) = tools.get(&call_id).copied()
                    && let Some(LedgerEntry::ToolExecution {
                        result,
                        is_error: err,
                        ..
                    }) = entries.get_mut(idx)
                {
                    *result = Some(content.to_string());
                    *err = message.is_error();
                    continue;
                }
                if !call_id.is_empty() {
                    entries.push(LedgerEntry::ToolExecution {
                        tool_call_id: call_id,
                        tool_name: name,
                        args: String::new(),
                        result: Some(content.to_string()),
                        is_error: message.is_error(),
                    });
                }
            }
            _ => {}
        }
    }
    entries
}

#[cfg(test)]
#[path = "ledger_sync_tests.rs"]
mod ledger_sync_tests;

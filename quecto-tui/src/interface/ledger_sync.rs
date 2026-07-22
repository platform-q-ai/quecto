use crate::interface::app::app_message_recovery::recovered_chat_entries;
use crate::interface::components::chat::ChatEntry;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDelta {
    pub epoch: u64,
    pub rev: u64,
    #[serde(default)]
    pub messages: Vec<serde_json::Value>,
    pub next_rev: Option<u64>,
    #[serde(default)]
    pub caught_up: bool,
    #[serde(default)]
    pub resync: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LedgerTranscript {
    messages: HashMap<String, serde_json::Value>,
    order: Vec<String>,
}

impl LedgerTranscript {
    pub fn apply_sync_delta(&mut self, delta: &SyncDelta) -> Vec<ChatEntry> {
        if delta.resync {
            self.messages.clear();
            self.order.clear();
        }
        for message in &delta.messages {
            if let Some(id) = message
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                let id = id.to_string();
                if !self.messages.contains_key(&id) {
                    self.order.push(id.clone());
                }
                self.messages.insert(id, message.clone());
            }
        }
        recovered_chat_entries(&self.order, &self.messages)
    }
}

pub fn supports_sync(data: &serde_json::Value) -> bool {
    data.get("sync").and_then(|v| v.as_u64()).unwrap_or(0) >= 1
        || data
            .get("capabilities")
            .and_then(|c| c.get("sync"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            >= 1
}

#[cfg(test)]
#[path = "ledger_sync_tests.rs"]
mod ledger_sync_tests;

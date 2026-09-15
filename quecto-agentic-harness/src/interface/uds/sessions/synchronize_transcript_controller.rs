//! Controller of the `sync` command (#1857, #1973): maps the wire fields
//! (the client's numeric `epoch` and `sinceRev`) onto the application's
//! sync request, for the idle dispatch loop and the busy reader task
//! alike. The reader task's strict parse of a parent-local `sync` line
//! lives here too: exactly `type=sync` without an `agent_id`, with numeric
//! `epoch` and `sinceRev`; anything else is not this command and stays on
//! its own route. No policy: reset or delta, and where a cut delta
//! continues, are the use case's; the reset window and the frame budget
//! are the wire module's.
use std::sync::Arc;

use crate::application::sessions::dto::{SyncRequest, TranscriptSync};
use crate::application::sessions::use_cases::SynchronizeTranscript;
use crate::domain::message::Message;

/// The wire fields of a parent-local `sync` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFields {
    pub request_id: Option<String>,
    pub epoch: u64,
    pub since_rev: u64,
}

impl SyncFields {
    /// One raw client line as a parent-local `sync`: `None` for anything
    /// else — another command, a child-addressed `sync` (`agent_id`), a
    /// missing or non-numeric `epoch`/`sinceRev`, or no JSON object.
    pub fn parse_line(line: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        if value.get("type")?.as_str()? != "sync" || value.get("agent_id").is_some() {
            return None;
        }
        Some(Self {
            request_id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
            epoch: value.get("epoch")?.as_u64()?,
            since_rev: value.get("sinceRev")?.as_u64()?,
        })
    }
}

pub struct SynchronizeTranscriptController {
    synchronize: Arc<SynchronizeTranscript>,
}

impl SynchronizeTranscriptController {
    pub fn new(synchronize: Arc<SynchronizeTranscript>) -> Self {
        Self { synchronize }
    }

    /// `sync` for a client at `since_rev` of `epoch`: the newest
    /// `reset_window` messages when it must resynchronise, else the
    /// committed messages after `since_rev` that `carry` — the transport's
    /// frame predicate — accepts, in commit order.
    pub async fn sync(
        &self,
        epoch: u64,
        since_rev: u64,
        reset_window: usize,
        carry: impl FnMut(&Message) -> bool,
    ) -> TranscriptSync {
        let request = SyncRequest {
            epoch,
            since_rev,
            reset_window,
        };
        self.synchronize.execute(&request, carry).await
    }
}

impl std::fmt::Debug for SynchronizeTranscriptController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SynchronizeTranscriptController")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "synchronize_transcript_controller_tests.rs"]
mod tests;

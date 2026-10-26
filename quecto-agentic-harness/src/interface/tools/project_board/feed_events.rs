//! The board's change feed as its front ends show it (epic #2705 B10): one
//! event's data, the allowlisted schema the server streams and `quecto
//! board watch` prints alike: numbers and a kind, never a task's text.
use serde_json::{Value, json};

use crate::application::project_board::ports::feed::FeedEvent;

/// An event's data: its kind and numbers, and nothing else.
pub fn event_json(event: &FeedEvent) -> Value {
    match event {
        FeedEvent::Epoch { epoch } => json!({"kind": "epoch", "epoch": epoch}),
        FeedEvent::Revision(revision) => json!({"kind": "revision",
            "epoch": revision.epoch, "revision": revision.revision}),
        FeedEvent::Reset { after } => json!({"kind": "reset", "after": after}),
    }
}

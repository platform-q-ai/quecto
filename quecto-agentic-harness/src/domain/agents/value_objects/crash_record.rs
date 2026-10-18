//! Why a harness process died (#2192): the record its panic hook leaves
//! before the process ends. Pure types; the file lives in infrastructure.
use serde::{Deserialize, Serialize};

/// The longest panic message or location a record keeps, in bytes: a
/// record is written from a panic hook and read into a one-line notice.
pub const MAX_CRASH_TEXT_BYTES: usize = 2048;

/// The most running tool calls a record names.
pub const MAX_RUNNING_CALLS: usize = 16;

/// A panic, as the hook saw it: its message and `file:line:column`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanicReport {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

impl PanicReport {
    /// A report whose texts are cut, on a character boundary, to
    /// [`MAX_CRASH_TEXT_BYTES`].
    pub fn new(message: &str, location: Option<&str>) -> Self {
        Self {
            message: bounded(message).to_string(),
            location: location.map(|at| bounded(at).to_string()),
        }
    }
}

/// A fatal panic, as the dying process recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashRecord {
    #[serde(flatten)]
    pub panic: PanicReport,
    /// The tool call whose own code panicked: set only when the panic came
    /// from inside that call's scope (one it could not contain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    /// The tool calls that were running when it happened, oldest first. A
    /// panic outside any call cannot be attributed to one of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running: Vec<String>,
    /// The contained panic this one struck while it unwound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earlier: Option<PanicReport>,
    /// Written while a contained panic unwound, and withdrawn once the call
    /// contained it: one that is still here means the process ended first.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub provisional: bool,
    /// The session the record was written for, exactly as its key is
    /// (#2192 review): a record's file name is derived from the key, so a
    /// reader takes a record only when it names the very session asked
    /// about. Not bounded on read: a reader compares it with the key it
    /// holds and rejects any other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub pid: u32,
    pub unix_ms: u64,
}

impl CrashRecord {
    pub fn new(panic: PanicReport, pid: u32, unix_ms: u64) -> Self {
        Self {
            panic,
            call: None,
            running: Vec::new(),
            earlier: None,
            provisional: false,
            session: None,
            pid,
            unix_ms,
        }
    }

    /// The call whose own code panicked.
    pub fn in_call(mut self, tool: &str) -> Self {
        self.call = Some(bounded(tool).to_string());
        self
    }

    /// The calls that were running, at most [`MAX_RUNNING_CALLS`] of them.
    pub fn running(mut self, calls: Vec<String>) -> Self {
        self.running = calls
            .into_iter()
            .take(MAX_RUNNING_CALLS)
            .map(|tool| bounded(&tool).to_string())
            .collect();
        self
    }

    /// The contained panic this one struck while it unwound.
    pub fn after(mut self, earlier: Option<PanicReport>) -> Self {
        self.earlier = earlier;
        self
    }

    /// The record with every text bounded again, as a record read from a
    /// file anyone may have written is.
    pub fn bounded(mut self) -> Self {
        let report =
            |report: &PanicReport| PanicReport::new(&report.message, report.location.as_deref());
        self.earlier = self.earlier.as_ref().map(report);
        self.panic = PanicReport::new(&self.panic.message, self.panic.location.as_deref());
        self.call = self.call.as_deref().map(|tool| bounded(tool).to_string());
        let running = std::mem::take(&mut self.running);
        self.running(running)
    }

    /// The record as written for the session `key`.
    pub fn for_session(mut self, key: &str) -> Self {
        self.session = Some(key.to_string());
        self
    }

    /// Whether the record says it was written for the session `key`.
    pub fn is_for_session(&self, key: &str) -> bool {
        self.session.as_deref() == Some(key)
    }

    /// Mark the record provisional (see [`CrashRecord::provisional`]).
    pub fn provisional(mut self) -> Self {
        self.provisional = true;
        self
    }
}

fn bounded(text: &str) -> &str {
    let mut end = text.len().min(MAX_CRASH_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
#[path = "crash_record_tests.rs"]
mod tests;

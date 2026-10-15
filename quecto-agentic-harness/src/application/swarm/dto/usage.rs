//! Usage accounting and inference admission (#2274): the
//! requests and answers of `RecordRequestUsage` and `ReadRequestAdmission`,
//! and the `request_usage` rows `BoardUsage` reads and writes.
//!
//! As in [`super::control`], `actor` is the member the call acts as and
//! every other argument stays the JSON value the caller passed: Python
//! type-checks a token limit, a strictness flag and a request record at
//! run time.
use serde_json::Value;

use super::{ControlReceipt, RunSnapshotView};

/// `Workbench._record_request(record)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordRequestUsageRequest {
    pub actor: String,
    pub record: Value,
}

/// How `_record_request` took a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestDelivery {
    /// A new request id: one row inserted.
    Recorded,
    /// An identical record from the same actor: nothing written.
    Redelivered,
    /// Accepted as the same record, and the stored payload rewritten
    /// because the runtime names an executable digest (a digest the stored
    /// record already named included).
    Replaced,
}

/// `_record_request`'s answer: the control receipt. Usage is measured and
/// reported, never limited: no request pauses a run.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedRequest {
    pub delivery: RequestDelivery,
    pub receipt: ControlReceipt,
}

/// `_request_admission`'s answer: the run and its members, as the
/// inference admission reads them.
#[derive(Clone, Debug, PartialEq)]
pub struct RequestAdmission {
    pub view: RunSnapshotView,
}

/// A stored `request_usage` row as the redelivery check reads it: its
/// actor as stored and its payload as `json.loads` reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRequestUsage {
    pub actor: Value,
    pub payload: Value,
}

/// One new `request_usage` row: the record's text as the board's
/// `encode()` wrote it (the very bytes the size cap measured), its
/// measurement, and the four reported counts copied from the record, in the
/// table's column order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewRequestUsage {
    pub request_id: String,
    pub actor: String,
    pub payload: String,
    pub tokens: u64,
    pub unknown: u64,
    pub attempts: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
}

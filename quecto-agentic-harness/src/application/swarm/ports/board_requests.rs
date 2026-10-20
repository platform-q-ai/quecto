//! The coordination board's request ledger (`Store.retry`, #2272).

use crate::domain::swarm::value_objects::board_error::BoardError;
use serde_json::Value;

/// The work a new request runs: its result, which the ledger stores.
pub type RequestAction<'a> = dyn FnMut() -> Result<Value, BoardError> + 'a;

/// The request ledger (`Store.retry`, #2272): idempotent retries of a
/// member's request id.
pub trait BoardRequests {
    /// A request `actor` made before replays its stored result when
    /// `payload` encodes to the same text, and is refused with
    /// `request id reused with different payload` otherwise; a new one
    /// runs `action` and stores its result, while the ledger has room.
    /// The replayed result is the stored text as `json.loads` reads it.
    fn retry(
        &self,
        actor: &str,
        request: &str,
        payload: &Value,
        action: &mut RequestAction<'_>,
    ) -> Result<Value, BoardError>;
}

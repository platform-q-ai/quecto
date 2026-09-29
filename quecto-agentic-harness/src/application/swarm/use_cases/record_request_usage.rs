//! `Workbench._record_request(record)` (#2274): the harness records one
//! model request a member made, and the token budget applies.
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::board_control::{edited, receipt};
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::board_usage::apply_usage_budget;
use crate::application::swarm::dto::{
    NewRequestUsage, RecordRequestUsageRequest, RecordedRequest, RequestDelivery,
};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::{
    Access, BoardError, MAX_REQUEST_PAYLOAD_BYTES, MAX_REQUEST_ROWS, Redelivery, python_equal,
    redelivery, request_measurement,
};

/// The record is measured before the operation gate
/// (`request_measurement`). Through the gate for reading (any member, a
/// dead one included) its encoded text is bounded to 32,768 bytes; a
/// request id the ledger holds is a redelivery (the same actor and record,
/// the runtime's digest possibly now known, which replaces the stored
/// record) or refused; a new one is inserted while the ledger holds fewer
/// than 10,000 rows. The budget then applies, and the answer is the
/// control receipt.
pub struct RecordRequestUsage {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl RecordRequestUsage {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            encoding,
        }
    }

    /// # Errors
    /// An invalid record, an oversized one, a reused request id, a full
    /// ledger, an authorisation refusal, an edited record, or the store's.
    pub fn execute(
        &self,
        request: RecordRequestUsageRequest,
    ) -> Result<RecordedRequest, BoardError> {
        let RecordRequestUsageRequest { actor, record } = request;
        let (tokens, unknown, attempts) = request_measurement(&record)?;
        let (Value::Object(fields), Some(Value::String(request_id))) =
            (&record, record.get("request_id"))
        else {
            // `request_measurement` accepts only an object with a text id.
            return Err(BoardError::new("invalid request observation"));
        };
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            &actor,
            reading,
            |transaction, _| {
                let payload = self.encoding.encode(&record)?;
                if payload.len() > MAX_REQUEST_PAYLOAD_BYTES {
                    return Err(BoardError::new(format!(
                        "request diagnostic exceeds {MAX_REQUEST_PAYLOAD_BYTES} bytes"
                    )));
                }
                let delivery = match transaction.request_usage(request_id)? {
                    Some(prior) => {
                        let Value::Object(previous) = &prior.payload else {
                            return Err(edited("request usage"));
                        };
                        let same_actor = python_equal(&prior.actor, &Value::from(actor.as_str()));
                        match redelivery(previous, fields, same_actor) {
                            Redelivery::Same => RequestDelivery::Redelivered,
                            Redelivery::DigestKnown => {
                                transaction.update_request_usage(request_id, &record)?;
                                RequestDelivery::DigestKnown
                            }
                            Redelivery::Different => {
                                return Err(BoardError::new(
                                    "request observation ID reused with different data",
                                ));
                            }
                        }
                    }
                    None if transaction.request_usage_count()? < MAX_REQUEST_ROWS => {
                        let reported = |field: &str| fields.get(field).and_then(Value::as_u64);
                        transaction.insert_request_usage(&NewRequestUsage {
                            request_id: request_id.clone(),
                            actor: actor.clone(),
                            record: record.clone(),
                            tokens,
                            unknown,
                            attempts,
                            input_tokens: reported("input_tokens"),
                            output_tokens: reported("output_tokens"),
                            cache_read_tokens: reported("cache_read_tokens"),
                            cache_write_tokens: reported("cache_write_tokens"),
                        })?;
                        RequestDelivery::Recorded
                    }
                    None => {
                        return Err(BoardError::new(
                            "request diagnostic ledger full; export before starting another run",
                        ));
                    }
                };
                let effect = apply_usage_budget(transaction, &*self.clock, &actor)?;
                Ok(RecordedRequest {
                    delivery,
                    effect,
                    receipt: receipt(transaction, &*self.clock)?,
                })
            },
        )
    }
}

#[cfg(test)]
#[path = "record_request_usage_tests.rs"]
mod tests;

//! Strict schema validation for catalogue publication, unlike crash-tolerant load.
use crate::domain::error::DomainError;

use super::session_store_records::{SessionFile, SessionRecord};

pub fn validate_catalogue_record(bytes: &[u8]) -> Result<(), DomainError> {
    if serde_json::from_slice::<SessionFile>(bytes).is_ok() {
        return Ok(());
    }
    let mut records = serde_json::Deserializer::from_slice(bytes).into_iter::<SessionRecord>();
    match records.next() {
        Some(Ok(SessionRecord::Snapshot(_))) => (),
        _ => {
            return Err(DomainError::Session(
                "catalogue requires a valid session snapshot".into(),
            ));
        }
    }
    for record in records {
        record.map_err(|e| DomainError::Session(e.to_string()))?;
    }
    Ok(())
}

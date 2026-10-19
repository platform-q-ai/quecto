//! Availability policy: what a catalogue entry's transport adapter,
//! credential status and any held provider refusal make of its
//! availability (#1575, #2435; moved from the application by #2578).

use crate::domain::catalogue::value_objects::availability::{
    Availability, AvailabilityStatus, UnavailableReason,
};
use crate::domain::catalogue::value_objects::identity::TransportKind;

/// Whether a transport adapter exists for this transport kind. Every named
/// kind the domain enumerates has an adapter today; a transport a catalogue
/// file declared that no adapter implements does not (#1575, AC3), so its
/// entries stay known-but-unrunnable by construction.
pub fn transport_has_adapter(transport: &TransportKind) -> bool {
    !matches!(transport, TransportKind::Unsupported { .. })
}

/// Translate adapter support and credential status into derived availability.
pub fn derive_availability(
    transport: TransportKind,
    adapter_supported: bool,
    credential_available: bool,
) -> Availability {
    if adapter_supported && credential_available {
        return Availability::runnable();
    }
    let mut reasons = Vec::new();
    if !credential_available {
        reasons.push(UnavailableReason::MissingCredential);
    }
    if !adapter_supported {
        reasons.push(UnavailableReason::UnsupportedTransport { transport });
    }
    // No adapter caps the ladder at Configured; a missing credential alone
    // leaves the entry Available (everything but the credential is in place).
    let status = if adapter_supported {
        AvailabilityStatus::Available
    } else {
        AvailabilityStatus::Configured
    };
    Availability::unavailable(status, reasons)
        .expect("non-runnable status with at least one reason is always constructible")
}

/// `availability` with the provider's refusal of the model for the account
/// in use added (#2435): never runnable, at most `Available`, every earlier
/// reason kept.
pub fn refused_availability(availability: &Availability, reason: String) -> Availability {
    let status = availability.status().min(AvailabilityStatus::Available);
    let mut reasons = availability.reasons().to_vec();
    reasons.push(UnavailableReason::RefusedForAccount(reason));
    Availability::unavailable(status, reasons)
        .expect("a non-runnable status with at least one reason is always constructible")
}

#[cfg(test)]
#[path = "availability_tests.rs"]
mod tests;

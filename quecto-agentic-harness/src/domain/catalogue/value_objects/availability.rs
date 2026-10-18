//! Availability semantics: the `Known < Configured < Available < Runnable`
//! ladder and why a known model is not runnable.

use super::error::CatalogueDomainError;
use super::identity::TransportKind;

/// Why a known model is not runnable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    MissingCredential,
    UnsupportedTransport {
        transport: TransportKind,
    },
    InvalidConfiguration(String),
    PolicyDenied(String),
    /// The provider refused the model for the account or auth mode in use
    /// (#2435), with the provider's own words: learned from a definitive
    /// refusal, it holds until it expires or the provider serves the model.
    RefusedForAccount(String),
}

/// Availability ladder: `Known < Configured < Available < Runnable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AvailabilityStatus {
    /// The catalogue knows the entry exists.
    Known,
    /// Local configuration supplies enough non-secret shape to configure it.
    Configured,
    /// A transport adapter exists and configuration is valid.
    Available,
    /// Fully runnable right now.
    Runnable,
}

/// Availability status with structured reasons for why an entry is not
/// runnable. `Runnable` carries no reasons by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    status: AvailabilityStatus,
    reasons: Vec<UnavailableReason>,
}

impl Availability {
    pub fn runnable() -> Self {
        Self {
            status: AvailabilityStatus::Runnable,
            reasons: Vec::new(),
        }
    }

    /// A non-runnable availability. Returns an error when `status` is
    /// `Runnable` (runnable entries carry no reasons) or when `reasons` is
    /// empty (non-runnable entries must say why).
    pub fn unavailable(
        status: AvailabilityStatus,
        reasons: Vec<UnavailableReason>,
    ) -> Result<Self, CatalogueDomainError> {
        if status == AvailabilityStatus::Runnable {
            return Err(CatalogueDomainError::RunnableWithReasons);
        }
        if reasons.is_empty() {
            return Err(CatalogueDomainError::UnavailableWithoutReason);
        }
        Ok(Self { status, reasons })
    }

    pub fn status(&self) -> AvailabilityStatus {
        self.status
    }

    pub fn is_runnable(&self) -> bool {
        self.status == AvailabilityStatus::Runnable
    }

    pub fn reasons(&self) -> &[UnavailableReason] {
        &self.reasons
    }
}

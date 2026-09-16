//! Ports of a catalogue refresh (#1846): a source that can refresh itself
//! from its remote origin under a bounded, cancellable context, and the
//! redaction of credential material from the outcomes it reports. Network
//! happens only inside [`RefreshableCatalogueSource::refresh`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::application::catalogue::CatalogueSource;
use crate::application::catalogue::dto::RefreshBounds;

/// Per-run refresh context: bounds plus a cooperative cancellation flag.
#[derive(Debug, Clone, Default)]
pub struct RefreshContext {
    pub bounds: RefreshBounds,
    cancel: Arc<AtomicBool>,
}

impl RefreshContext {
    pub fn new(bounds: RefreshBounds) -> Self {
        Self {
            bounds,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// What a successful source refresh changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshChange {
    Updated {
        models: usize,
    },
    /// Nothing changed; `models` is the count the (unchanged) cache holds, so
    /// adapters can still report a meaningful total.
    Unchanged {
        models: usize,
    },
}

/// Why a source refresh did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshError {
    /// The provider has no remote listing to refresh from; the reason must be
    /// actionable (what is unsupported and what the user can do instead).
    Unsupported {
        reason: String,
    },
    Failed {
        reason: String,
    },
    Cancelled,
}

/// Application port: a catalogue source that can be asked to refresh itself
/// from its remote origin. Ordinary loads stay network-free; network happens
/// only inside [`RefreshableCatalogueSource::refresh`].
pub trait RefreshableCatalogueSource: CatalogueSource {
    fn refresh(&self, ctx: &RefreshContext) -> Result<RefreshChange, RefreshError>;
}

/// Redaction port: strips credential material from human-facing refresh text.
/// Implemented by infrastructure (which knows the secret values); the
/// application never sees the secrets themselves.
pub trait RefreshRedactionPort: Send + Sync {
    fn redact(&self, text: &str) -> String;
}

/// A redactor for contexts with no credential material in scope (rigs).
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopRedaction;

#[cfg(any(test, feature = "test-support"))]
impl RefreshRedactionPort for NoopRedaction {
    fn redact(&self, text: &str) -> String {
        text.to_string()
    }
}

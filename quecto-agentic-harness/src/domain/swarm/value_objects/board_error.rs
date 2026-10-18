//! The board's refusal (#2303).

use crate::domain::swarm::events::telemetry::RefusalKind;

/// A refusal by the board. `Display` is the exact message members see;
/// [`RefusalKind`] is what telemetry records (#2303). The fields are
/// private: every refusal is built through [`BoardError::new`] with its
/// kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardError {
    kind: RefusalKind,
    message: String,
}

impl BoardError {
    pub fn new(kind: RefusalKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> RefusalKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BoardError {}

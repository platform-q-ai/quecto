//! Domain-level catalogue errors.

/// Domain-level catalogue errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogueDomainError {
    EmptyProviderId,
    EmptyModelId,
    UnqualifiedModelRef(String),
    ProviderMismatch {
        entry_provider: String,
        model_provider: String,
    },
    ZeroLimit(String),
    RunnableWithReasons,
    UnavailableWithoutReason,
}

impl std::fmt::Display for CatalogueDomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyProviderId => f.write_str("provider id must not be empty"),
            Self::EmptyModelId => f.write_str("model id must not be empty"),
            Self::UnqualifiedModelRef(value) => write!(
                f,
                "model reference '{value}' is missing provider/model syntax"
            ),
            Self::ProviderMismatch {
                entry_provider,
                model_provider,
            } => write!(
                f,
                "entry provider '{entry_provider}' does not match model provider '{model_provider}'"
            ),
            Self::ZeroLimit(field) => write!(f, "model limit '{field}' must be non-zero"),
            Self::RunnableWithReasons => {
                f.write_str("runnable availability must not carry unavailability reasons")
            }
            Self::UnavailableWithoutReason => {
                f.write_str("non-runnable availability must carry at least one reason")
            }
        }
    }
}

impl std::error::Error for CatalogueDomainError {}

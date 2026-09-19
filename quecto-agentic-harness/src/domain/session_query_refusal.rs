//! Why a metadata search (#2010) is answered with no rows instead of being
//! searched, in the words the answer carries.

/// The longest query, in visible characters, that is searched at all.
pub const MAX_QUERY_CHARS: usize = 256;

/// Why a query is answered with no rows instead of being searched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryRefusal {
    /// More visible characters than [`MAX_QUERY_CHARS`], counted before any
    /// fold (R2-H4): a prefix would match records the whole query does not name.
    TooLong { chars: usize },
    /// A request field was not what it must be (R1-H5, R2-H10): answered —
    /// correlated — rather than dropped as undecodable.
    Malformed {
        field: &'static str,
        expected: &'static str,
    },
}

impl std::fmt::Display for QueryRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong { chars } => write!(
                f,
                "query too long: {chars} characters (at most {MAX_QUERY_CHARS} are searched)"
            ),
            Self::Malformed { field, expected } => write!(f, "{field} must be {expected}"),
        }
    }
}

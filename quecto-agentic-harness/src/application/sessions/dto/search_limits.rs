//! The two numbers of a metadata search request (#2010): the client's query
//! generation and the row limit, each a value that cannot be out of range.

/// The client's own counter of the query an answer belongs to, echoed
/// unchanged: the application attaches no meaning to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueryGeneration(pub u64);

/// How many rows an answer may carry: 1 to [`SearchLimit::MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchLimit(usize);

impl SearchLimit {
    pub const DEFAULT: usize = 200;
    pub const MAX: usize = 500;

    /// The requested limit brought into range; absent is the default.
    pub fn clamped(requested: Option<u64>) -> Self {
        let requested =
            requested.map_or(Self::DEFAULT, |n| usize::try_from(n).unwrap_or(Self::MAX));
        Self(requested.clamp(1, Self::MAX))
    }

    pub fn get(self) -> usize {
        self.0
    }
}

impl Default for SearchLimit {
    fn default() -> Self {
        Self::clamped(None)
    }
}

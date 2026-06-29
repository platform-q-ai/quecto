//! Shared domain constants that cross layer boundaries.

/// Default cap on tool output bytes (50 KiB) used by file, search, and shell tools
/// as well as agent-loop progress events. Kept in the domain layer so the
/// application layer and infrastructure layers can both depend on one value.
pub const DEFAULT_OUTPUT_CAP_BYTES: usize = 50 * 1024;

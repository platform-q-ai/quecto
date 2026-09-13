//! Audit capability: the sink port the agent turn emits engine-authored
//! events through (#1960). The event vocabulary is the domain's
//! (`domain::audit`); the append-only log adapter lives in infrastructure.

pub mod ports;

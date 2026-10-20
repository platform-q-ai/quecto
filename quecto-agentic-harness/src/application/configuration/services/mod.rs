//! Pure policy over the configuration JSON document: how a malformed one is
//! worded (#2617), overlay sections and
//! global-only keys, secret-shaped leaves, and the removed keys (#2414);
//! shared by the use cases and the repair DTO; the config schema's tests pin
//! parity with `removed_keys`.

pub mod document_parse;
pub mod overlay_policy;
pub mod redaction;
pub mod removed_keys;

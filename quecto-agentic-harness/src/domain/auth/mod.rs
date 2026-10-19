//! Provider credential lifecycle policy (#1839): what a stored credential
//! is, when it has expired, the margin every OAuth sign-in keeps before its
//! server-side expiry, and which providers a login may name. Pure: the
//! current time is always handed in.
//!
//! Flat until a role holds two files (wiki rule 10).

pub mod credential;
pub mod pasted_code;
pub mod provider;

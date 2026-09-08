//! Shared inference admission authority adapters (#1679 P3): private
//! directory, singleton lock, durable journal, framed UDS server and client.
pub mod secret;

pub use secret::RandomSecretSource;

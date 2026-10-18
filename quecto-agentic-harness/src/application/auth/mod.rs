//! Authenticate with a provider and manage its stored credentials (#1839–
//! #1842): the login use cases and the effect ports they need. Constructed
//! only by `composition::bootstrap::auth`; the interface holds handles.

pub mod dto;
pub mod ports;
pub mod use_cases;

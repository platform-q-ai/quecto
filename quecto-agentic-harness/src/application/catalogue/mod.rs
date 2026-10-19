//! Catalogue capability (epic #1193; target layout from #1845 on).
//!
//! The application layer is the authority for resolving catalogue sources into
//! one published, immutable snapshot generation. It defines the ports it
//! needs; infrastructure implements them; the use cases under `use_cases/`
//! answer the read and mutation triggers (UDS `list_models` first, #1845),
//! and every surface presents their DTOs. Only composition
//! (`composition::catalogue`) constructs a use case; interface holds handles.

pub mod dto;
pub mod ports;
pub mod services;
pub mod use_cases;

//! Canonical provider/model catalogue domain model (epic #1193, slice 1).
//!
//! These types are intentionally free of JSON/TOML schemas, filesystem paths,
//! environment variables, HTTP clients, CLI/UDS DTOs, and TUI state.
//! Infrastructure adapters translate external catalogue formats into these
//! descriptors in later slices; this slice defines identities, descriptors,
//! capabilities, availability, immutable snapshots, and the pure merge /
//! validation / override rules over them.
//!
//! # Source-layer precedence
//!
//! Catalogue entries arrive from several source layers. Resolution applies a
//! stable-identity upsert with the documented precedence order (lowest to
//! highest):
//!
//! `BuiltIn < Generated < Discovered < Extension < UserDefined < UserOverride`
//!
//! A higher-precedence entry for the same [`ModelRef`](value_objects::identity::ModelRef) replaces the
//! lower-precedence entry while keeping the earlier ordering position, matching
//! the legacy registry upsert behaviour. Precedence is a property of the layer,
//! not of the order layers are handed to [`resolve_catalogue`](services::resolution::resolve_catalogue).

pub mod entities;
pub mod services;
pub mod value_objects;

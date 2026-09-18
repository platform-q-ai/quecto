//! The standard container bundle (#2024 S4e): the embedded assets
//! (Containerfile, the official adapter's runtime scripts, the build
//! command) and their materialisation below a project, and the checkout's
//! `origin` remote. Mechanics only; where the bundle goes and what the
//! config entry says is the application's. Which runtime the bundle
//! drives is the bundle's own knowledge (its scripts and its build
//! command), never this crate's.

pub mod assets;
pub mod workspace_origin;

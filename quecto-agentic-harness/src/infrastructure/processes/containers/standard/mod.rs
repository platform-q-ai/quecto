//! The standard container bundle (#2024 S4e): the embedded assets
//! (Containerfile, the official rootless-Podman/Docker runtime scripts)
//! and their materialisation below a project, and the checkout's `origin`
//! remote. Mechanics only; where the bundle goes and what the config
//! entry says is the application's.

pub mod assets;
pub mod workspace_origin;

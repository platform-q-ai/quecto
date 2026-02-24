//! Port traits for coding job coordination.
//!
//! These define what the application layer needs from the outside world.
//! Infrastructure adapters implement these traits.

/// Port for validating repository and ref existence.
pub trait RepoValidator {
    fn repo_exists(&self, repo: &str) -> bool;
    fn ref_exists(&self, repo: &str, base_ref: &str) -> bool;
}

/// Port for resolving skills from the workspace.
pub trait SkillResolver {
    fn skill_exists(&self, name: &str) -> bool;
}

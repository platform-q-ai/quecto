//! The team a plan names for its swarm (#2481's artifact model): its
//! roles, each with a model, an effort and limits, and an optional
//! deadline, in seconds from Start, and image. A team sets no token
//! budget: a run started from it has none (owner decision, 2026-10-14).
use super::super::value_objects::schema_error::{SchemaError, distinct};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::vocabulary::Effort;
use super::task_parts::shaped;
use regex::Regex;
use std::sync::LazyLock;

pub const MAX_ROLES: usize = 16;
pub const MAX_MEMBERS: u16 = 32;
pub const MAX_MODEL_BYTES: usize = 100;
pub const MAX_IMAGE_BYTES: usize = 255;
/// The nearest deadline a run is started with: ten minutes.
pub const MIN_DEADLINE_SECONDS: u32 = 10 * 60;
/// The furthest deadline a run is started with: seven days, the most a
/// swarm takes.
pub const MAX_DEADLINE_SECONDS: u32 = 7 * 24 * 60 * 60;

/// The team a plan names for its swarm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub roles: Vec<Role>,
    /// How long the run has, in seconds from Start, at least
    /// [`MIN_DEADLINE_SECONDS`] and at most [`MAX_DEADLINE_SECONDS`]; none
    /// for the default.
    pub deadline_seconds: Option<u32>,
    /// An OCI image reference.
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub name: Slug,
    pub model: String,
    pub effort: Effort,
    pub limits: RoleLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleLimits {
    pub members: u16,
    pub turns: Option<u32>,
}

/// A model name: `/`-separated segments, each starting with a letter or
/// digit, so none is empty, `.`, `..` or an option.
static MODEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._:-]*(?:/[A-Za-z0-9][A-Za-z0-9._:-]*)*$").expect("model")
});
/// The distribution image reference grammar, `[domain/]path[:tag][@digest]`,
/// as container engines normalise it: the first component is a domain only
/// when it holds a `.` or a `:` port, is `localhost`, or is a bracketed IPv6
/// address; otherwise it is a path component, and path components are
/// lowercase.
static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    let label = r"[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?";
    let port = r"(?::[0-9]+)";
    let domain = format!(
        r"(?:localhost{port}?|{label}(?:\.{label})+{port}?|{label}{port}|\[[0-9A-Fa-f:]+\]{port}?)"
    );
    let component = r"[a-z0-9]+(?:(?:[._]|__|-+)[a-z0-9]+)*";
    let tag = r"[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}";
    let digest = r"[A-Za-z][A-Za-z0-9]*(?:[-_+.][A-Za-z][A-Za-z0-9]*)*:[0-9a-fA-F]{32,}";
    let pattern = format!(r"^(?:{domain}/)?{component}(?:/{component})*(?::{tag})?(?:@{digest})?$");
    Regex::new(&pattern).expect("image")
});
impl Team {
    pub fn validate(&self, field: &str) -> Result<(), SchemaError> {
        let names: Vec<&Slug> = self.roles.iter().map(|role| &role.name).collect();
        distinct(&format!("{field}/roles"), &names, MAX_ROLES)?;
        if self.roles.is_empty() {
            return Err(SchemaError::new(
                format!("{field}/roles"),
                "must name at least one role",
            ));
        }
        for (index, role) in self.roles.iter().enumerate() {
            role.validate(&format!("{field}/roles/{index}"))?;
        }
        let timed = self
            .deadline_seconds
            .is_none_or(|seconds| (MIN_DEADLINE_SECONDS..=MAX_DEADLINE_SECONDS).contains(&seconds));
        if !timed {
            return Err(SchemaError::new(
                format!("{field}/deadline_seconds"),
                format!("must be {MIN_DEADLINE_SECONDS} to {MAX_DEADLINE_SECONDS}"),
            ));
        }
        match &self.image {
            Some(image) => shaped(&format!("{field}/image"), image, MAX_IMAGE_BYTES, &IMAGE),
            None => Ok(()),
        }
    }
}

impl Role {
    fn validate(&self, field: &str) -> Result<(), SchemaError> {
        shaped(
            &format!("{field}/model"),
            &self.model,
            MAX_MODEL_BYTES,
            &MODEL,
        )?;
        if !(1..=MAX_MEMBERS).contains(&self.limits.members) {
            return Err(SchemaError::new(
                format!("{field}/limits/members"),
                format!("must be 1 to {MAX_MEMBERS}"),
            ));
        }
        if self.limits.turns.is_none_or(|turns| matches!(turns, 1..)) {
            Ok(())
        } else {
            Err(SchemaError::new(
                format!("{field}/limits/turns"),
                "must be at least 1",
            ))
        }
    }
}

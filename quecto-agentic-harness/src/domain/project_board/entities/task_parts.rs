//! The structured parts the board's artifacts share: the team a plan
//! names for its swarm, and (until reviews become their own artifact, M1b)
//! a task's review findings. Reviews are capped; a write past the cap is
//! refused.
use super::super::value_objects::schema_error::{SchemaError, distinct, filled_markdown, line};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{Effort, Severity, Verdict};
use super::claim::Identity;
use regex::Regex;
use std::sync::LazyLock;

pub const MAX_ITEMS: usize = 128;
/// Criteria, evidence and dependencies per item.
pub const MAX_PER_ITEM: usize = 16;
pub const MAX_ROLES: usize = 16;
pub const MAX_MEMBERS: u16 = 32;
pub const MAX_REVIEWS: usize = 128;
pub const MAX_LINE_CHARS: usize = 500;
pub const MAX_OWNER_CHARS: usize = 100;
pub const MAX_MODEL_BYTES: usize = 100;
pub const MAX_IMAGE_BYTES: usize = 255;
/// The largest integer a JSON reader in any language keeps exactly.
pub const MAX_BUDGET_TOKENS: u64 = (1 << 53) - 1;
pub const MAX_FINDING_CHARS: usize = 4_000;

/// The team a plan names for its swarm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub roles: Vec<Role>,
    pub budget: Option<Budget>,
    pub deadline: Option<Timestamp>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    pub tokens: u64,
}

/// A review finding, at `path:line` or `path:line:col` when it has a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub id: Slug,
    pub round: u8,
    pub reviewer: Identity,
    pub at: Timestamp,
    pub severity: Severity,
    pub location: Option<String>,
    /// Markdown.
    pub finding: String,
    pub verdict: Verdict,
    /// The test that proves a fix.
    pub proving_test: Option<String>,
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
/// `path:line` or `path:line:col`, numbers from 1 without leading zeros.
/// The path is relative: `/`-separated segments, none empty, `.` or `..`
/// (a segment starts with a name character, or with one `.` followed by
/// one, as in `.github`), and the first does not start with `-` (an option
/// to a tool).
static LOCATION: LazyLock<Regex> = LazyLock::new(|| {
    let rest = r"[A-Za-z0-9._@+-]*";
    let first = format!(r"(?:[A-Za-z0-9_@+]{rest}|\.[A-Za-z0-9_@+-]{rest})");
    let segment = format!(r"(?:[A-Za-z0-9_@+-]{rest}|\.[A-Za-z0-9_@+-]{rest})");
    let pattern = format!(r"^{first}(?:/{segment})*:[1-9][0-9]{{0,8}}(?::[1-9][0-9]{{0,8}})?$");
    Regex::new(&pattern).expect("location")
});

/// Distinct single lines, at most `max`.
pub(super) fn lines(field: &str, entries: &[String], max: usize) -> Result<(), SchemaError> {
    distinct(field, entries, max)?;
    let mut indexed = entries.iter().enumerate();
    indexed.try_for_each(|(index, entry)| line(&format!("{field}/{index}"), entry, MAX_LINE_CHARS))
}

fn shaped(field: &str, text: &str, max_bytes: usize, shape: &Regex) -> Result<(), SchemaError> {
    if text.len() <= max_bytes && shape.is_match(text) {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("{text:?} is not an allowed reference of at most {max_bytes} bytes"),
        ))
    }
}

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
        let funded = self
            .budget
            .as_ref()
            .is_none_or(|budget| matches!(budget.tokens, 1..=MAX_BUDGET_TOKENS));
        if !funded {
            return Err(SchemaError::new(
                format!("{field}/budget/tokens"),
                format!("must be 1 to {MAX_BUDGET_TOKENS}"),
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

impl Review {
    pub fn validate(&self, field: &str) -> Result<(), SchemaError> {
        if !matches!(self.round, 1..) {
            return Err(SchemaError::new(
                format!("{field}/round"),
                "must be at least 1",
            ));
        }
        self.reviewer.validate(&format!("{field}/reviewer"))?;
        if let Some(location) = &self.location {
            shaped(
                &format!("{field}/location"),
                location,
                MAX_LINE_CHARS,
                &LOCATION,
            )?;
        }
        filled_markdown(
            &format!("{field}/finding"),
            &self.finding,
            MAX_FINDING_CHARS,
        )?;
        match &self.proving_test {
            Some(test) => line(&format!("{field}/proving_test"), test, MAX_LINE_CHARS),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "task_parts_tests.rs"]
mod tests;

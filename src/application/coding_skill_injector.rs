//! Skill injection for the coding runtime.
//!
//! Resolves which skills to inject into a coding worker's context by
//! merging default skills, profile-specific skills, and explicitly
//! requested skills. Enforces allowlist/denylist policy, deduplicates,
//! and validates skill existence via the `SkillResolver` port.

use crate::domain::coding_command::CommandError;
use crate::domain::coding_ports::SkillResolver;

use std::collections::HashMap;

// ============================================================================
// Policy configuration
// ============================================================================

/// Full skill injection policy including defaults, profiles, and
/// enable/disable toggle.
#[derive(Debug, Clone)]
pub struct SkillPolicy {
    /// Whether skill injection is enabled at all.
    pub enabled: bool,
    /// Default skills merged into every job.
    pub defaults: Vec<String>,
    /// Global allowlist. Empty means "allow all not denied".
    pub allowlist: Vec<String>,
    /// Global denylist. Takes precedence over allowlist.
    pub denylist: Vec<String>,
    /// Profile-specific skill sets (profile name -> skills).
    pub profile_skills: HashMap<String, Vec<String>>,
    /// Profile-specific allowlist overrides.
    pub profile_allowlist: HashMap<String, Vec<String>>,
    /// Profile-specific denylist overrides.
    pub profile_denylist: HashMap<String, Vec<String>>,
}

impl Default for SkillPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            defaults: Vec::new(),
            allowlist: Vec::new(),
            denylist: Vec::new(),
            profile_skills: HashMap::new(),
            profile_allowlist: HashMap::new(),
            profile_denylist: HashMap::new(),
        }
    }
}

// ============================================================================
// Resolution result
// ============================================================================

/// Outcome of skill resolution for a single job.
#[derive(Debug, Clone)]
pub struct SkillResolution {
    /// The deduplicated set of effective skills.
    pub skills: Vec<String>,
    /// Whether injection was enabled.
    pub injection_enabled: bool,
    /// Profile name used (if any).
    pub profile: Option<String>,
}

/// A worker's skill suggestion, with policy evaluation.
#[derive(Debug, Clone)]
pub struct SkillSuggestion {
    /// Suggested skill names.
    pub skills: Vec<String>,
    /// Reason for the suggestion.
    pub reason: String,
    /// Who suggested it (e.g. "worker").
    pub by: Option<String>,
    /// Whether any suggested skill is policy-denied.
    pub policy_denied: bool,
}

// ============================================================================
// Resolution logic
// ============================================================================

/// Resolve the effective skill set for a job.
///
/// Returns `Err(CommandError::PolicyDenied)` if any requested skill
/// violates the allowlist/denylist policy.
/// Returns `Err(CommandError::SkillNotFound)` if any effective skill
/// does not exist on disk via the `SkillResolver` port.
pub fn resolve_skills<S: SkillResolver>(
    policy: &SkillPolicy,
    requested: &[String],
    profile: Option<&str>,
    resolver: &S,
) -> Result<SkillResolution, CommandError> {
    if !policy.enabled {
        return Ok(SkillResolution {
            skills: Vec::new(),
            injection_enabled: false,
            profile: profile.map(String::from),
        });
    }

    let profile_name = profile.unwrap_or("default");
    let profile_skills = policy
        .profile_skills
        .get(profile_name)
        .cloned()
        .unwrap_or_default();

    let (effective_allowlist, effective_denylist) =
        resolve_policy_lists(policy, profile_name, &profile_skills);

    // Check requested skills against policy.
    for skill in requested {
        check_policy(skill, &effective_allowlist, &effective_denylist)?;
    }

    // Merge defaults + profile + requested, deduplicate.
    let merged: Vec<String> = policy
        .defaults
        .iter()
        .cloned()
        .chain(profile_skills)
        .chain(requested.iter().cloned())
        .collect();
    let effective = dedupe(merged);

    // Validate all effective skills exist on disk.
    for skill in &effective {
        if !resolver.skill_exists(skill) {
            return Err(CommandError::SkillNotFound);
        }
    }

    Ok(SkillResolution {
        skills: effective,
        injection_enabled: true,
        profile: profile.map(String::from),
    })
}

/// Evaluate a worker's skill suggestion against policy.
pub fn evaluate_suggestion(
    policy: &SkillPolicy,
    skills: Vec<String>,
    reason: String,
    by: Option<String>,
) -> SkillSuggestion {
    let policy_denied = skills.iter().any(|s| policy.denylist.contains(s));
    SkillSuggestion {
        skills,
        reason,
        by,
        policy_denied,
    }
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Resolve effective allowlist and denylist, with profile overrides.
fn resolve_policy_lists(
    policy: &SkillPolicy,
    profile_name: &str,
    profile_skills: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut allowlist = policy
        .profile_allowlist
        .get(profile_name)
        .cloned()
        .unwrap_or_else(|| policy.allowlist.clone());

    // Profile skills are implicitly allowed when the profile has its
    // own allowlist override.
    if policy.profile_allowlist.contains_key(profile_name) {
        for skill in profile_skills {
            if !allowlist.contains(skill) {
                allowlist.push(skill.clone());
            }
        }
    }

    let denylist = policy
        .profile_denylist
        .get(profile_name)
        .cloned()
        .unwrap_or_else(|| policy.denylist.clone());

    (allowlist, denylist)
}

/// Check a single skill against effective allow/deny lists.
fn check_policy(
    skill: &str,
    allowlist: &[String],
    denylist: &[String],
) -> Result<(), CommandError> {
    if denylist.iter().any(|s| s == skill) {
        return Err(CommandError::PolicyDenied);
    }
    if !allowlist.is_empty() && !allowlist.iter().any(|s| s == skill) {
        return Err(CommandError::PolicyDenied);
    }
    Ok(())
}

/// Remove duplicates while preserving order.
fn dedupe(items: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    for item in items {
        if !seen.contains(&item) {
            seen.push(item);
        }
    }
    seen
}

#[cfg(test)]
#[path = "coding_skill_injector_tests.rs"]
mod tests;

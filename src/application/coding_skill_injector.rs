//! Skill injection for the coding runtime.
//!
//! Resolves which skills to inject into a coding worker's context by
//! merging default skills, profile-specific skills, and explicitly
//! requested skills. Enforces allowlist/denylist policy, deduplicates,
//! and validates skill existence via the `SkillResolver` port.

use crate::domain::coding_command::CommandError;
use crate::domain::coding_ports::SkillResolver;
use crate::domain::skill::is_valid_skill_name;

use std::collections::{HashMap, HashSet};

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

    // Validate skill name format for all requested skills.
    for skill in requested {
        if !is_valid_skill_name(skill) {
            return Err(CommandError::PolicyDenied);
        }
    }

    let profile_name = profile.unwrap_or("default");
    let profile_skills = policy
        .profile_skills
        .get(profile_name)
        .cloned()
        .unwrap_or_default();

    let (effective_allowlist, effective_denylist) =
        resolve_policy_lists(policy, profile_name, &profile_skills);

    let deny_set: HashSet<&str> = effective_denylist.iter().map(String::as_str).collect();
    let allow_set: HashSet<&str> = effective_allowlist.iter().map(String::as_str).collect();

    // Check requested skills against policy.
    for skill in requested {
        check_policy(skill, &allow_set, &deny_set)?;
    }

    // Merge defaults + profile + requested, deduplicate.
    let effective = dedupe(
        policy
            .defaults
            .iter()
            .cloned()
            .chain(profile_skills)
            .chain(requested.iter().cloned()),
    );

    // Check defaults and profile skills against denylist too.
    for skill in &effective {
        if deny_set.contains(skill.as_str()) {
            return Err(CommandError::PolicyDenied);
        }
    }

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

/// Input for evaluating a worker's skill suggestion.
pub struct SuggestionInput {
    /// Suggested skill names.
    pub skills: Vec<String>,
    /// Reason for the suggestion.
    pub reason: String,
    /// Who suggested it (e.g. "worker").
    pub by: Option<String>,
    /// Active profile for effective denylist resolution.
    pub profile: Option<String>,
}

/// Evaluate a worker's skill suggestion against policy.
///
/// Uses the effective denylist (profile-overridden if `profile` is
/// specified) so that profile-scoped denylists are respected.
pub fn evaluate_suggestion(policy: &SkillPolicy, input: SuggestionInput) -> SkillSuggestion {
    let profile_name = input.profile.as_deref().unwrap_or("default");
    let effective_denylist = policy
        .profile_denylist
        .get(profile_name)
        .unwrap_or(&policy.denylist);
    let deny_set: HashSet<&str> = effective_denylist.iter().map(String::as_str).collect();
    let policy_denied = input.skills.iter().any(|s| deny_set.contains(s.as_str()));
    SkillSuggestion {
        skills: input.skills,
        reason: input.reason,
        by: input.by,
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

/// Check a single skill against effective allow/deny sets.
fn check_policy(
    skill: &str,
    allow_set: &HashSet<&str>,
    deny_set: &HashSet<&str>,
) -> Result<(), CommandError> {
    if deny_set.contains(skill) {
        return Err(CommandError::PolicyDenied);
    }
    if !allow_set.is_empty() && !allow_set.contains(skill) {
        return Err(CommandError::PolicyDenied);
    }
    Ok(())
}

/// Remove duplicates while preserving insertion order.
fn dedupe(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen_set = HashSet::new();
    let mut result = Vec::new();
    for item in items {
        if seen_set.insert(item.clone()) {
            result.push(item);
        }
    }
    result
}

#[cfg(test)]
#[path = "coding_skill_injector_tests.rs"]
mod tests;

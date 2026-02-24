use super::*;

// -- Test doubles ---------------------------------------------------------

struct MockResolver {
    available: Vec<String>,
}

impl MockResolver {
    fn all_known() -> Self {
        Self {
            available: vec![
                "rust-style".into(),
                "test-first".into(),
                "security-checklist".into(),
                "api-design".into(),
                "frontend-guide".into(),
            ],
        }
    }

    fn with(names: &[&str]) -> Self {
        Self {
            available: names.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl SkillResolver for MockResolver {
    fn skill_exists(&self, name: &str) -> bool {
        self.available.iter().any(|s| s == name)
    }
}

fn default_policy() -> SkillPolicy {
    SkillPolicy {
        enabled: true,
        defaults: vec!["rust-style".into(), "test-first".into()],
        allowlist: vec![
            "rust-style".into(),
            "test-first".into(),
            "security-checklist".into(),
        ],
        denylist: vec!["forbidden-skill".into()],
        ..Default::default()
    }
}

// -- Tests ----------------------------------------------------------------

#[test]
fn test_resolve_defaults_only() {
    let policy = default_policy();
    let resolver = MockResolver::all_known();
    let result = resolve_skills(&policy, &[], None, &resolver).unwrap();
    assert_eq!(result.skills, vec!["rust-style", "test-first"]);
    assert!(result.injection_enabled);
}

#[test]
fn test_resolve_with_additional_skill() {
    let policy = default_policy();
    let resolver = MockResolver::all_known();
    let result = resolve_skills(&policy, &["security-checklist".into()], None, &resolver).unwrap();
    assert!(result.skills.contains(&"rust-style".to_string()));
    assert!(result.skills.contains(&"test-first".to_string()));
    assert!(result.skills.contains(&"security-checklist".to_string()));
}

#[test]
fn test_resolve_denylisted_skill_rejected() {
    let policy = default_policy();
    let resolver = MockResolver::all_known();
    let result = resolve_skills(&policy, &["forbidden-skill".into()], None, &resolver);
    assert_eq!(result.unwrap_err(), CommandError::PolicyDenied);
}

#[test]
fn test_resolve_unknown_skill_rejected() {
    let policy = default_policy();
    let resolver = MockResolver::all_known();
    let result = resolve_skills(&policy, &["unknown-skill".into()], None, &resolver);
    assert_eq!(result.unwrap_err(), CommandError::PolicyDenied);
}

#[test]
fn test_resolve_disabled_injection() {
    let mut policy = default_policy();
    policy.enabled = false;
    let resolver = MockResolver::all_known();
    let result = resolve_skills(&policy, &[], None, &resolver).unwrap();
    assert!(result.skills.is_empty());
    assert!(!result.injection_enabled);
}

#[test]
fn test_resolve_duplicate_skills_deduplicated() {
    let policy = default_policy();
    let resolver = MockResolver::all_known();
    let result = resolve_skills(
        &policy,
        &["rust-style".into(), "rust-style".into()],
        None,
        &resolver,
    )
    .unwrap();
    let count = result.skills.iter().filter(|s| *s == "rust-style").count();
    assert_eq!(count, 1);
}

#[test]
fn test_resolve_skill_not_found_on_disk() {
    let policy = SkillPolicy {
        enabled: true,
        defaults: vec![],
        allowlist: vec!["security-checklist".into()],
        denylist: vec![],
        ..Default::default()
    };
    let resolver = MockResolver::with(&[]); // nothing on disk
    let result = resolve_skills(&policy, &["security-checklist".into()], None, &resolver);
    assert_eq!(result.unwrap_err(), CommandError::SkillNotFound);
}

#[test]
fn test_resolve_empty_defaults_empty_request() {
    let policy = SkillPolicy {
        enabled: true,
        defaults: vec![],
        allowlist: vec![],
        denylist: vec![],
        ..Default::default()
    };
    let resolver = MockResolver::all_known();
    let result = resolve_skills(&policy, &[], None, &resolver).unwrap();
    assert!(result.skills.is_empty());
    assert!(result.injection_enabled);
}

#[test]
fn test_resolve_with_profile() {
    let mut policy = default_policy();
    policy
        .profile_skills
        .insert("backend".into(), vec!["api-design".into()]);
    policy
        .profile_allowlist
        .insert("backend".into(), vec!["api-design".into()]);
    let resolver = MockResolver::all_known();
    let result = resolve_skills(&policy, &[], Some("backend"), &resolver).unwrap();
    assert!(result.skills.contains(&"api-design".to_string()));
    assert!(result.skills.contains(&"rust-style".to_string()));
    assert_eq!(result.profile.as_deref(), Some("backend"));
}

#[test]
fn test_resolve_profile_denylist_overrides_global() {
    let mut policy = default_policy();
    policy
        .profile_denylist
        .insert("restricted".into(), vec!["security-checklist".into()]);
    let resolver = MockResolver::all_known();
    let result = resolve_skills(
        &policy,
        &["security-checklist".into()],
        Some("restricted"),
        &resolver,
    );
    assert_eq!(result.unwrap_err(), CommandError::PolicyDenied);
}

#[test]
fn test_resolve_profile_skills_merged_with_defaults() {
    let mut policy = default_policy();
    policy.profile_skills.insert(
        "fullstack".into(),
        vec![
            "rust-style".into(),
            "api-design".into(),
            "frontend-guide".into(),
        ],
    );
    policy.profile_allowlist.insert(
        "fullstack".into(),
        vec![
            "rust-style".into(),
            "api-design".into(),
            "frontend-guide".into(),
        ],
    );
    // Also add to global allowlist
    let resolver = MockResolver::all_known();
    let result = resolve_skills(&policy, &[], Some("fullstack"), &resolver).unwrap();
    // rust-style appears only once despite being in both defaults and profile
    let count = result.skills.iter().filter(|s| *s == "rust-style").count();
    assert_eq!(count, 1);
    assert!(result.skills.contains(&"api-design".to_string()));
    assert!(result.skills.contains(&"frontend-guide".to_string()));
    assert!(result.skills.contains(&"test-first".to_string()));
}

#[test]
fn test_evaluate_suggestion_allowed() {
    let policy = default_policy();
    let suggestion = evaluate_suggestion(
        &policy,
        SuggestionInput {
            skills: vec!["security-checklist".into()],
            reason: "touches auth".into(),
            by: Some("worker".into()),
            profile: None,
        },
    );
    assert!(!suggestion.policy_denied);
    assert_eq!(suggestion.by.as_deref(), Some("worker"));
}

#[test]
fn test_evaluate_suggestion_denied() {
    let policy = default_policy();
    let suggestion = evaluate_suggestion(
        &policy,
        SuggestionInput {
            skills: vec!["forbidden-skill".into()],
            reason: "wanted it".into(),
            by: None,
            profile: None,
        },
    );
    assert!(suggestion.policy_denied);
}

#[test]
fn test_evaluate_suggestion_profile_denylist() {
    let mut policy = default_policy();
    policy
        .profile_denylist
        .insert("strict".into(), vec!["security-checklist".into()]);
    let suggestion = evaluate_suggestion(
        &policy,
        SuggestionInput {
            skills: vec!["security-checklist".into()],
            reason: "reason".into(),
            by: None,
            profile: Some("strict".into()),
        },
    );
    assert!(suggestion.policy_denied);
    // Same skill allowed under default profile
    let suggestion2 = evaluate_suggestion(
        &policy,
        SuggestionInput {
            skills: vec!["security-checklist".into()],
            reason: "reason".into(),
            by: None,
            profile: None,
        },
    );
    assert!(!suggestion2.policy_denied);
}

#[test]
fn test_resolve_rejects_invalid_skill_name() {
    let policy = default_policy();
    let resolver = MockResolver::all_known();
    let result = resolve_skills(&policy, &["../traversal".into()], None, &resolver);
    assert_eq!(result.unwrap_err(), CommandError::PolicyDenied);
}

#[test]
fn test_dedupe_preserves_order() {
    let input = vec!["a".into(), "b".into(), "a".into(), "c".into(), "b".into()];
    let result = dedupe(input);
    assert_eq!(result, vec!["a", "b", "c"]);
}

#[test]
fn test_check_policy_denylist_first() {
    // Skill in both allow and deny → denied
    let allow: HashSet<&str> = ["x"].into();
    let deny: HashSet<&str> = ["x"].into();
    assert_eq!(
        check_policy("x", &allow, &deny),
        Err(CommandError::PolicyDenied)
    );
}

#[test]
fn test_check_policy_empty_allowlist_allows_all() {
    let allow: HashSet<&str> = HashSet::new();
    let deny: HashSet<&str> = HashSet::new();
    assert!(check_policy("anything", &allow, &deny).is_ok());
}

#[test]
fn test_resolve_policy_lists_profile_override() {
    let mut policy = default_policy();
    policy
        .profile_allowlist
        .insert("custom".into(), vec!["custom-skill".into()]);
    let (allow, deny) = resolve_policy_lists(&policy, "custom", &[]);
    assert!(allow.contains(&"custom-skill".to_string()));
    // Denylist falls back to global since no profile override
    assert!(deny.contains(&"forbidden-skill".to_string()));
}

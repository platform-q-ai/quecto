use super::*;

// --- CancelFlag ---

#[test]
fn cancel_flag_initially_not_cancelled() {
    let flag = CancelFlag::new();
    assert!(!flag.is_cancelled());
}

#[test]
fn cancel_flag_cancel_sets_cancelled() {
    let flag = CancelFlag::new();
    flag.cancel();
    assert!(flag.is_cancelled());
}

#[test]
fn cancel_flag_clone_shares_state() {
    let flag = CancelFlag::new();
    let clone = flag.clone();
    flag.cancel();
    assert!(clone.is_cancelled());
}

#[test]
fn cancel_flag_default_not_cancelled() {
    let flag = CancelFlag::default();
    assert!(!flag.is_cancelled());
}

// --- ThinkingLevel ---

#[test]
fn thinking_level_adaptive_is_adaptive() {
    assert!(ThinkingLevel::Adaptive.is_adaptive());
}

#[test]
fn thinking_level_non_adaptive() {
    assert!(!ThinkingLevel::Low.is_adaptive());
    assert!(!ThinkingLevel::Medium.is_adaptive());
    assert!(!ThinkingLevel::High.is_adaptive());
    assert!(!ThinkingLevel::Max.is_adaptive());
}

#[test]
fn thinking_level_budget_tokens() {
    assert_eq!(ThinkingLevel::Adaptive.budget_tokens(), None);
    assert_eq!(ThinkingLevel::Low.budget_tokens(), Some(1024));
    assert_eq!(ThinkingLevel::Medium.budget_tokens(), Some(10_000));
    assert_eq!(ThinkingLevel::High.budget_tokens(), Some(16_384));
    assert_eq!(ThinkingLevel::Max.budget_tokens(), Some(32_768));
}

// --- EffortLevel ---

#[test]
fn effort_level_as_str() {
    assert_eq!(EffortLevel::Low.as_str(), "low");
    assert_eq!(EffortLevel::Medium.as_str(), "medium");
    assert_eq!(EffortLevel::High.as_str(), "high");
    assert_eq!(EffortLevel::Max.as_str(), "max");
}

#[test]
fn effort_level_parse_valid() {
    assert_eq!(EffortLevel::parse("low"), Some(EffortLevel::Low));
    assert_eq!(EffortLevel::parse("medium"), Some(EffortLevel::Medium));
    assert_eq!(EffortLevel::parse("high"), Some(EffortLevel::High));
    assert_eq!(EffortLevel::parse("max"), Some(EffortLevel::Max));
}

/// Issue #1066: OpenAI's documented reasoning-effort scale (none, low,
/// medium, high, xhigh) must be parseable and round-trip through as_str
/// so it can be transmitted verbatim for OpenAI reasoning models.
#[test]
fn effort_level_parse_openai_documented_scale_1066() {
    assert_eq!(EffortLevel::None.as_str(), "none");
    assert_eq!(EffortLevel::XHigh.as_str(), "xhigh");
    for level in ["none", "low", "medium", "high", "xhigh"] {
        let parsed = EffortLevel::parse(level).unwrap_or_else(|| {
            panic!("OpenAI-documented effort level '{level}' must parse (#1066)")
        });
        assert_eq!(
            parsed.as_str(),
            level,
            "effort '{level}' must round-trip verbatim (#1066)"
        );
    }
}

#[test]
fn effort_level_parse_invalid() {
    assert_eq!(EffortLevel::parse(""), None);
    assert_eq!(EffortLevel::parse("ultra"), None);
    assert_eq!(EffortLevel::parse("LOW"), None);
}

// --- ToolChoice ---

#[test]
fn tool_choice_auto_eq() {
    assert_eq!(ToolChoice::Auto, ToolChoice::Auto);
    assert_ne!(ToolChoice::Auto, ToolChoice::Any);
}

#[test]
fn tool_choice_specific() {
    let tc = ToolChoice::Specific("bash".to_string());
    assert_eq!(tc, ToolChoice::Specific("bash".to_string()));
    assert_ne!(tc, ToolChoice::Specific("read".to_string()));
}

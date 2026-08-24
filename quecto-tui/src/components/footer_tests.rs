use super::*;

#[test]
fn footer_shows_model() {
    let mut f = Footer::new();
    f.set_model("claude-sonnet-4-6");
    let lines = f.render(80);
    let joined = lines.join("\n");
    let plain: String = joined
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert!(
        plain.contains("claude-sonnet-4-6"),
        "should contain model: {}",
        plain
    );
}

#[test]
fn footer_shows_git_branch() {
    let mut f = Footer::new();
    f.set_git_branch(Some("main".to_string()));
    let lines = f.render(80);
    let joined = lines.join("\n");
    let plain: String = joined
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert!(plain.contains("main"), "should contain branch: {}", plain);
}

#[test]
fn footer_respects_width() {
    let mut f = Footer::new();
    f.set_model("very-long-model-name-that-might-overflow");
    let lines = f.render(40);
    for line in &lines {
        assert!(
            visible_width(line) <= 40,
            "footer line exceeds width: {} (width={})",
            line,
            visible_width(line)
        );
    }
}

#[test]
fn format_tokens_various() {
    assert_eq!(format_tokens(0), "0");
    assert_eq!(format_tokens(500), "500");
    assert_eq!(format_tokens(1500), "1.5k");
    assert_eq!(format_tokens(15000), "15k");
    assert_eq!(format_tokens(1500000), "1.5M");
}

#[test]
fn format_tokens_edge_cases() {
    assert_eq!(format_tokens(999), "999");
    assert_eq!(format_tokens(1000), "1.0k");
    assert_eq!(format_tokens(9999), "10.0k");
    assert_eq!(format_tokens(10000), "10k");
    assert_eq!(format_tokens(999_999), "999k");
    assert_eq!(format_tokens(1_000_000), "1.0M");
}

#[test]
fn footer_shows_monetary_cost_after_context() {
    let mut f = Footer::new();
    f.update_context_usage(120_000, 200_000);
    f.set_cost(Some(0.0421));
    let lines = f.render(120);
    let joined = lines.join("\n");
    assert!(joined.contains("120k/200k"), "context first: {joined}");
    assert!(
        joined.contains("cost $0.042100"),
        "footer should show normalized monetary cost: {joined}"
    );
}

#[test]
fn footer_hides_zero_cost() {
    let mut f = Footer::new();
    f.update_context_usage(120_000, 200_000);
    f.set_cost(Some(0.0));
    let joined = f.render(120).join("\n");
    assert!(!joined.contains('$'), "zero cost hidden: {joined}");
    assert!(!joined.contains("cost"), "zero cost label hidden: {joined}");
}

#[test]
fn footer_hides_cost_when_none() {
    let mut f = Footer::new();
    f.update_context_usage(120_000, 200_000);
    let joined = f.render(120).join("\n");
    assert!(!joined.contains('$'), "no cost by default: {joined}");
}

#[test]
fn footer_shows_context_percent() {
    let mut f = Footer::new();
    f.set_context(Some(42.5), 200_000);
    let lines = f.render(80);
    let joined = lines.join("\n");
    assert!(joined.contains("42.5%"));
}

#[test]
fn footer_shows_used_tokens_and_limit() {
    let mut f = Footer::new();
    // 120k of a 200k window → 60.0%.
    f.update_context_usage(120_000, 200_000);
    let lines = f.render(80);
    let joined = lines.join("\n");
    assert!(
        joined.contains("120k/200k"),
        "should show used/limit: {joined}"
    );
    assert!(joined.contains("60.0%"), "should show percent: {joined}");
}

#[test]
fn footer_context_high_usage_warning() {
    let mut f = Footer::new();
    f.set_context(Some(75.0), 200_000);
    let lines = f.render(80);
    let joined = lines.join("");
    // Should contain warning color (yellow)
    assert!(joined.contains("75.0%"));
}

#[test]
fn footer_context_critical_usage_error() {
    let mut f = Footer::new();
    f.set_context(Some(95.0), 200_000);
    let lines = f.render(80);
    let joined = lines.join("");
    assert!(joined.contains("95.0%"));
}

#[test]
fn footer_no_context() {
    let mut f = Footer::new();
    f.set_context(None, 0);
    let lines = f.render(80);
    let joined = lines.join("");
    assert!(joined.contains("?/0"));
}

#[test]
fn footer_narrow_width_truncates() {
    let mut f = Footer::new();
    f.set_model("very-long-model-name-that-will-be-truncated");
    f.set_context(Some(50.0), 200_000);
    let lines = f.render(20);
    for line in &lines {
        assert!(visible_width(line) <= 20);
    }
}

/// The streaming flag must be reflected in `render()` (issue #760): the four
/// production callers of `set_streaming` previously toggled a write-only
/// field that nothing rendered. Streaming must show a spinner indicator.
#[test]
fn footer_renders_streaming_indicator() {
    let mut f = Footer::new();
    f.set_model("claude-sonnet-4-6");
    f.set_streaming(true);
    let joined = f.render(80).join("\n");
    assert!(
        joined.contains(theme::STREAMING_INDICATOR),
        "streaming footer should render a streaming indicator: {joined:?}"
    );
}

#[test]
fn footer_hides_streaming_indicator_when_idle() {
    let mut f = Footer::new();
    f.set_model("claude-sonnet-4-6");
    f.set_streaming(false);
    let joined = f.render(80).join("\n");
    assert!(
        !joined.contains(theme::STREAMING_INDICATOR),
        "idle footer should not render a streaming indicator: {joined:?}"
    );
}

#[test]
fn footer_invalidate_no_panic() {
    let mut f = Footer::new();
    f.invalidate(); // should not panic
    assert_eq!(f.render(80).len(), 2);
}

#[test]
fn footer_all_fields_set() {
    let mut f = Footer::new();
    f.set_model("gpt-4o");
    f.set_git_branch(Some("feature".to_string()));
    f.set_context(Some(30.0), 128_000);
    f.set_streaming(true);
    let lines = f.render(100);
    assert_eq!(lines.len(), 2);
    let joined = lines.join("\n");
    assert!(joined.contains("feature"));
}

#[test]
fn footer_extremely_narrow() {
    let mut f = Footer::new();
    f.set_model("m");
    f.set_context(Some(50.0), 100);
    let lines = f.render(5);
    for line in &lines {
        assert!(visible_width(line) <= 5);
    }
}

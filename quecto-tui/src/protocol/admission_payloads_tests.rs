use super::*;

fn plain(s: &str) -> String {
    s.to_string()
}

#[test]
fn parses_the_bounded_view_and_ignores_garbage() {
    assert_eq!(parse_admission(&serde_json::json!(null), &plain), None);
    assert_eq!(parse_admission(&serde_json::json!("x"), &plain), None);
    let view = parse_admission(
        &serde_json::json!({
            "waiting": 2, "admitted": 1, "longestWaitSeconds": 12, "revision": 9,
            "groups": [
                {"group": "anthropic", "cooldown": {"state": "until", "remainingSeconds": 30}},
                {"group": "openai"},
                {"nope": true},
                {"group": "vague", "cooldown": {"state": "unknown"}}
            ]
        }),
        &plain,
    )
    .unwrap();
    assert_eq!((view.waiting, view.admitted, view.revision), (2, 1, 9));
    assert_eq!(view.longest_wait_seconds, Some(12));
    assert_eq!(view.groups.len(), 3);
    assert_eq!(
        view.groups[0].cooldown,
        Some(AdmissionCooldown {
            state: "until".into(),
            remaining_seconds: Some(30)
        })
    );
    assert_eq!(view.groups[1].cooldown, None);
    assert_eq!(view.groups[2].cooldown.as_ref().unwrap().state, "unknown");
    let empty = parse_admission(&serde_json::json!({}), &plain).unwrap();
    assert_eq!(empty, AdmissionView::default());
}

#[test]
fn group_names_are_sanitized_and_bounded() {
    let many: Vec<_> = (0..40)
        .map(|i| serde_json::json!({"group": format!("g{i}\u{1b}[31m{}", "x".repeat(100))}))
        .collect();
    let view = parse_admission(
        &serde_json::json!({"groups": many}),
        &crate::components::ansi::sanitize_control,
    )
    .unwrap();
    assert_eq!(view.groups.len(), MAX_GROUPS);
    assert!(
        view.groups
            .iter()
            .all(|g| g.group.chars().count() <= MAX_GROUP_LABEL)
    );
    assert!(!view.groups[0].group.contains('\u{1b}'));
}

#[test]
fn status_label_prefers_waiting_then_cooldown_then_nothing() {
    let mut view = AdmissionView::default();
    assert_eq!(view.status_label(), None);
    view.admitted = 2;
    assert_eq!(view.status_label(), None, "admitted work is ordinary work");
    view.waiting = 1;
    assert_eq!(
        view.status_label().as_deref(),
        Some("waiting for admission")
    );
    view.longest_wait_seconds = Some(12);
    view.waiting = 3;
    assert_eq!(view.status_label().as_deref(), Some("12s (×3)"));
    view.groups.push(AdmissionGroupView {
        group: "anthropic".into(),
        cooldown: Some(AdmissionCooldown {
            state: "until".into(),
            remaining_seconds: Some(30),
        }),
    });
    assert_eq!(
        view.status_label().as_deref(),
        Some("12s (×3) · anthropic cooldown 30s")
    );
    view.waiting = 0;
    view.longest_wait_seconds = None;
    assert_eq!(
        view.status_label().as_deref(),
        Some("anthropic cooldown 30s")
    );
    view.groups[0].cooldown = Some(AdmissionCooldown {
        state: "until".into(),
        remaining_seconds: Some(0),
    });
    assert_eq!(
        view.status_label().as_deref(),
        Some("anthropic cooldown elapsed"),
        "local expiry stays visible without claiming authoritative availability"
    );
    view.groups[0].cooldown = Some(AdmissionCooldown {
        state: "unknown".into(),
        remaining_seconds: None,
    });
    assert_eq!(view.status_label().as_deref(), Some("anthropic throttled"));
    view.groups[0].cooldown = Some(AdmissionCooldown {
        state: "unavailable".into(),
        remaining_seconds: None,
    });
    assert_eq!(
        view.status_label().as_deref(),
        Some("anthropic unavailable")
    );
}

#[test]
fn compact_label_fits_a_panel_row() {
    let mut view = AdmissionView::default();
    assert_eq!(view.compact_label(), None);
    view.waiting = 2;
    assert_eq!(view.compact_label().as_deref(), Some("waiting"));
    view.longest_wait_seconds = Some(4);
    assert_eq!(view.compact_label().as_deref(), Some("4s"));
    view.waiting = 0;
    view.groups.push(AdmissionGroupView {
        group: "g".into(),
        cooldown: Some(AdmissionCooldown {
            state: "until".into(),
            remaining_seconds: Some(30),
        }),
    });
    assert_eq!(view.compact_label().as_deref(), Some("cooldown 30s"));
    view.groups[0].cooldown.as_mut().unwrap().state = "unknown".into();
    assert_eq!(view.compact_label().as_deref(), Some("throttled"));
    view.groups[0].cooldown.as_mut().unwrap().state = "unavailable".into();
    assert_eq!(view.compact_label().as_deref(), Some("unavailable"));
}

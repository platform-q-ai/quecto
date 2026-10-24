//! `/admission` and `/admission restart` (#2615), and the footer badge's
//! queue depth, through the headless harness: raw wire lines go through the
//! real `Event` deserializer and the footer through its real render.
use crate::shell::app::tui_harness::TuiHarness;

fn admission_line(admission: serde_json::Value) -> String {
    serde_json::json!({"type": "admission_state_changed", "admission": admission}).to_string()
}

fn response_line(success: bool, data: serde_json::Value, error: Option<&str>) -> String {
    let mut line = serde_json::json!({
        "type": "response", "id": "admission-restart",
        "command": "restart_admission", "success": success,
    });
    if success {
        line["data"] = data;
    } else {
        line["error"] = serde_json::json!(error);
    }
    line.to_string()
}

#[tokio::test]
async fn admission_restart_sends_the_restart_and_says_so() {
    let mut h = TuiHarness::new().await;
    h.submit("/admission restart");
    let commands = h.drain_commands().await;
    assert!(
        commands
            .iter()
            .any(|c| c.contains(r#""type":"restart_admission""#) && c.contains("admission-restart")),
        "{commands:?}"
    );
    assert!(
        h.notification_messages()
            .iter()
            .any(|m| m == "Restarting the admission broker…"),
        "{:?}",
        h.notification_messages()
    );
}

#[tokio::test]
async fn admission_with_anything_else_shows_the_usage_and_sends_nothing() {
    let mut h = TuiHarness::new().await;
    h.submit("/admission reboot");
    assert!(h.drain_commands().await.is_empty());
    assert_eq!(h.notification_messages(), ["Usage: /admission restart"]);
}

#[tokio::test]
async fn admission_alone_shows_the_brokers_health() {
    let mut h = TuiHarness::new().await;
    h.submit("/admission");
    assert_eq!(
        h.notification_messages(),
        ["This session has no admission broker"]
    );
    let mut h = TuiHarness::new().await;
    h.event_line(&admission_line(serde_json::json!({
        "waiting": 0, "revision": 1, "authorityStatus": "connected"
    })));
    h.submit("/admission");
    assert_eq!(
        h.notification_messages(),
        ["admission ✓ connected — Usage: /admission restart to restart the broker"]
    );
}

#[tokio::test]
async fn a_restart_response_says_how_the_broker_came_back() {
    let cases = [
        (
            serde_json::json!({"outcome": "restarted", "authorityStatus": "connected"}),
            "Admission broker restarted; this session reconnected",
        ),
        (
            serde_json::json!({"outcome": "service_restarted", "authorityStatus": "connected"}),
            "Admission broker service restarted; this session reconnected",
        ),
        (
            serde_json::json!({"outcome": "started", "authorityStatus": "reconnecting"}),
            "Admission broker started; this session is reconnecting",
        ),
    ];
    for (data, expected) in cases {
        let mut h = TuiHarness::new().await;
        h.event_line(&response_line(true, data, None));
        assert_eq!(h.notification_messages(), [expected]);
    }
    let mut h = TuiHarness::new().await;
    h.event_line(&response_line(
        false,
        serde_json::Value::Null,
        Some("an admission restart is already in progress"),
    ));
    assert_eq!(
        h.notification_messages(),
        ["Admission restart failed: an admission restart is already in progress"]
    );
}

#[tokio::test]
async fn the_badge_shows_health_and_this_sessions_queue_depth() {
    let mut h = TuiHarness::new().await;
    h.event_line(&admission_line(serde_json::json!({
        "waiting": 2, "longestWaitSeconds": 3, "revision": 1,
        "authorityStatus": "connected"
    })));
    let footer = h.master_footer_text();
    assert!(
        footer.contains("admission ✓ connected · 2 queued here"),
        "{footer}"
    );
    // An activity-only event (no status) moves the depth, keeps the health.
    h.event_line(&admission_line(
        serde_json::json!({"waiting": 0, "revision": 2}),
    ));
    let footer = h.master_footer_text();
    assert!(footer.contains("admission ✓ connected"), "{footer}");
    assert!(!footer.contains("queued"), "{footer}");
    h.event_line(&admission_line(serde_json::json!({
        "waiting": 1, "revision": 3, "authorityStatus": "unavailable"
    })));
    let footer = h.master_footer_text();
    assert!(
        footer.contains("admission ✗ unavailable · 1 queued here"),
        "{footer}"
    );
}

/// A harness whose TUI owns its agent and knows its session, model and
/// effort (from `get_state`), as a spawned TUI does.
async fn owning_harness() -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.app_mut()
        .set_child_exit_watch(crate::shell::child_watch::ChildWatch::for_tests(Some(4242)));
    h.event_line(
        &serde_json::json!({
            "type": "response", "command": "get_state", "success": true,
            "data": {"state": "idle", "model": "openai-oauth/gpt-6", "effort": "high",
                     "sessionKey": "cli:work", "generation": 1}
        })
        .to_string(),
    );
    h
}

const OFFER_RELAUNCH: &str = "The admission broker came back with a different policy; this session must restart to use it. /admission restart restarts this session and resumes this conversation.";

#[tokio::test]
async fn a_required_session_restart_is_offered_once_and_badged() {
    let mut h = owning_harness().await;
    let changed = serde_json::json!({
        "waiting": 0, "revision": 1, "authorityStatus": "unavailable", "restartRequired": true
    });
    h.event_line(&admission_line(changed.clone()));
    let footer = h.master_footer_text();
    assert!(footer.contains("admission ✗ restart required"), "{footer}");
    assert_eq!(h.notification_messages(), [OFFER_RELAUNCH]);
    // Still required: offered once, not on every event.
    let mut again = changed;
    again["revision"] = 2.into();
    h.event_line(&admission_line(again));
    assert_eq!(h.notification_messages().len(), 1);
    // An activity-only event keeps the badge.
    h.event_line(&admission_line(
        serde_json::json!({"waiting": 0, "revision": 3}),
    ));
    assert!(
        h.master_footer_text()
            .contains("admission ✗ restart required")
    );
    // A cleared view forgets it: no stale badge, and a later requirement
    // is offered again.
    h.app_mut().clear_master_admission();
    h.event_line(&admission_line(
        serde_json::json!({"waiting": 0, "revision": 4}),
    ));
    assert!(!h.master_footer_text().contains("restart required"));
    h.event_line(&admission_line(serde_json::json!({
        "waiting": 0, "revision": 5, "authorityStatus": "unavailable", "restartRequired": true
    })));
    assert_eq!(h.notification_messages().len(), 2, "offered again");
}

/// #2615 review: the offer says what this TUI will actually do.
#[tokio::test]
async fn the_offer_is_worded_for_what_this_tui_can_do() {
    let required = admission_line(serde_json::json!({
        "waiting": 0, "revision": 1, "authorityStatus": "unavailable", "restartRequired": true
    }));
    let mut not_owned = TuiHarness::new().await;
    not_owned.event_line(&required);
    assert_eq!(
        not_owned.notification_messages(),
        [
            "The admission broker came back with a different policy; this session must restart to use it. This TUI does not end its agent: restart that agent."
        ]
    );
    let mut ephemeral = owning_harness().await;
    ephemeral.app_mut().set_session_persisted(false);
    ephemeral.event_line(&required);
    assert_eq!(
        ephemeral.notification_messages(),
        [
            "The admission broker came back with a different policy; this session must restart to use it, and it is not persisted (--no-persist): restart quecto-tui yourself."
        ]
    );
}

fn session_restart_line() -> String {
    response_line(
        true,
        serde_json::json!({
            "outcome": "restarted", "authorityStatus": "unavailable",
            "sessionRestartRequired": true
        }),
        None,
    )
}

#[tokio::test]
async fn a_tui_that_owns_its_agent_restarts_the_session_resuming_it() {
    let mut h = owning_harness().await;
    h.event_line(&session_restart_line());
    assert_eq!(
        h.notification_messages(),
        [
            "Admission broker restarted; restarting this session to use its policy (resuming this conversation)…"
        ]
    );
    assert!(
        h.should_exit(),
        "the ordinary exit ends the owned agent first"
    );
    assert_eq!(
        h.app_mut().session_relaunch().cloned(),
        Some(crate::shell::app::app_ordinary_exit::SessionRelaunch {
            session_key: "cli:work".into(),
            model: Some("openai-oauth/gpt-6".into()),
            effort: Some("high".into()),
        })
    );
}

#[tokio::test]
async fn a_tui_that_cannot_resume_its_session_says_what_to_restart() {
    type Setup = fn(&mut TuiHarness);
    let cases: [(&str, Setup); 2] = [
        (
            "Admission broker restarted; its policy changed, and this TUI does not end its agent: restart that agent to use it",
            |h| h.app_mut().set_ordinary_exit_kill_owned(false),
        ),
        (
            "Admission broker restarted; its policy changed, and this session is not persisted (--no-persist), so restarting it would lose this conversation: restart quecto-tui yourself to use it",
            |h| h.app_mut().set_session_persisted(false),
        ),
    ];
    for (expected, setup) in cases {
        let mut h = owning_harness().await;
        setup(&mut h);
        h.event_line(&session_restart_line());
        assert_eq!(h.notification_messages(), [expected]);
        assert!(!h.should_exit());
        assert!(h.app_mut().session_relaunch().is_none());
    }
}

/// #2615 review: a session already told the broker serves a new policy
/// restarts itself only; the broker is not restarted again (other sessions
/// accepting the same offer would otherwise restart it in turn).
#[tokio::test]
async fn an_accepted_offer_restarts_only_this_session() {
    let mut h = owning_harness().await;
    h.event_line(&admission_line(serde_json::json!({
        "waiting": 0, "revision": 1, "authorityStatus": "unavailable", "restartRequired": true
    })));
    h.submit("/admission restart");
    assert!(
        !h.drain_commands()
            .await
            .iter()
            .any(|c| c.contains("restart_admission")),
        "the broker is not restarted"
    );
    assert_eq!(
        h.notification_messages().last().map(String::as_str),
        Some(
            "The admission broker serves a new policy; restarting this session to use its policy (resuming this conversation)…"
        )
    );
    assert!(h.should_exit());
    assert!(h.app_mut().session_relaunch().is_some());
}

/// #2615 review: a restart others share is confirmed by a second
/// `/admission restart` within a minute, and a confirmation is used once.
#[tokio::test]
async fn a_shared_restart_is_confirmed_by_running_it_again() {
    let mut h = TuiHarness::new().await;
    h.submit("/admission restart");
    let first = h.drain_commands().await;
    assert!(
        first
            .iter()
            .any(|c| c.contains("restart_admission") && !c.contains("confirm")),
        "{first:?}"
    );
    h.event_line(&response_line(
        true,
        serde_json::json!({"outcome": "unconfirmed", "sharedWith": 3}),
        None,
    ));
    assert_eq!(
        h.notification_messages().last().map(String::as_str),
        Some(
            "The admission broker is shared with 3 other session(s) or sub-agent(s). Restarting it fails their sub-agents (respawn them) and their in-flight requests once. Run /admission restart again within a minute to confirm."
        )
    );
    h.submit("/admission restart");
    let confirmed = h.drain_commands().await;
    assert!(
        confirmed
            .iter()
            .any(|c| c.contains("restart_admission") && c.contains(r#""confirm":3"#)),
        "{confirmed:?}"
    );
    h.submit("/admission restart");
    let again = h.drain_commands().await;
    assert!(
        again
            .iter()
            .any(|c| c.contains("restart_admission") && !c.contains("confirm")),
        "a confirmation is used once: {again:?}"
    );
}

/// #2615 review: a cleared admission view forgets the broker's health, so
/// a later activity-only event shows no stale badge.
#[tokio::test]
async fn a_cleared_view_forgets_the_brokers_health() {
    let mut h = TuiHarness::new().await;
    h.event_line(&admission_line(serde_json::json!({
        "waiting": 0, "revision": 1, "authorityStatus": "connected"
    })));
    assert!(h.master_footer_text().contains("admission ✓ connected"));
    h.app_mut().clear_master_admission();
    h.event_line(&admission_line(
        serde_json::json!({"waiting": 1, "revision": 2}),
    ));
    let footer = h.master_footer_text();
    assert!(!footer.contains("admission ✓"), "{footer}");
}

/// #2615 review: on a narrow line the model keeps its place and the badge
/// yields.
#[tokio::test]
async fn the_model_outranks_the_badge_on_a_narrow_line() {
    use crate::components::component::Component;
    let mut footer = crate::components::footer::Footer::new();
    footer.set_model("anthropic/claude-sonnet-4-5");
    footer.set_admission_health(Some("admission ✓ connected · 2 queued here".into()));
    let render = |footer: &mut crate::components::footer::Footer, width| {
        crate::components::ansi::strip_ansi(&footer.render(width).join("\n"))
    };
    let narrow = render(&mut footer, 60);
    assert!(narrow.contains("anthropic/claude-sonnet-4-5"), "{narrow}");
    assert!(!narrow.contains("admission ✓"), "{narrow}");
    let wide = render(&mut footer, 200);
    assert!(
        wide.contains("admission ✓ connected · 2 queued here"),
        "{wide}"
    );
    assert!(wide.contains("anthropic/claude-sonnet-4-5"), "{wide}");
}

/// #2615 review round 2: a confirmation lapses after a minute.
#[tokio::test]
async fn a_confirmation_lapses_after_a_minute() {
    let mut h = TuiHarness::new().await;
    h.event_line(&response_line(
        true,
        serde_json::json!({"outcome": "unconfirmed", "sharedWith": 2}),
        None,
    ));
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    h.submit("/admission restart");
    let sent = h.drain_commands().await;
    assert!(
        sent.iter()
            .any(|c| c.contains("restart_admission") && !c.contains("confirm")),
        "past the minute nothing is confirmed: {sent:?}"
    );
}

/// #2615 review round 2: an outcome this TUI does not know is not
/// reported as a restart.
#[tokio::test]
async fn an_unknown_outcome_is_not_reported_as_a_restart() {
    let mut h = TuiHarness::new().await;
    h.event_line(&response_line(
        true,
        serde_json::json!({"outcome": "rebooted", "authorityStatus": "connected"}),
        None,
    ));
    assert_eq!(
        h.notification_messages(),
        [
            "The admission broker answered the restart with an unknown outcome (rebooted); check /admission"
        ]
    );
}

/// #2615 review round 2: on a narrow line a required restart stays visible.
#[tokio::test]
async fn a_required_restart_stays_visible_on_a_narrow_line() {
    use crate::components::component::Component;
    let mut footer = crate::components::footer::Footer::new();
    footer.set_model("anthropic/claude-sonnet-4-5");
    footer.set_admission_health(Some("admission ✗ restart required".into()));
    footer.set_admission_alert(true);
    let narrow = crate::components::ansi::strip_ansi(&footer.render(50).join("\n"));
    assert!(narrow.contains("admission!"), "{narrow}");
    footer.set_admission_alert(false);
    let quiet = crate::components::ansi::strip_ansi(&footer.render(50).join("\n"));
    assert!(!quiet.contains("admission"), "{quiet}");
}

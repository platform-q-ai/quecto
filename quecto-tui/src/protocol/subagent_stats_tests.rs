use super::*;

fn reply(command: &str, success: bool) -> Event {
    Event::Response {
        id: None,
        command: command.into(),
        success,
        data: None,
        error: None,
    }
}

#[test]
fn the_request_is_get_session_stats_with_the_subagent_id() {
    let request = subagent_stats_request();
    let json = serde_json::to_value(&request).expect("command serializes");
    assert_eq!(json["type"], "get_session_stats");
    assert_eq!(json["id"], SUBAGENT_STATS_ID);
}

#[test]
fn every_run_end_makes_the_stats_stale() {
    let turn_end = Event::TurnEnd {
        message: serde_json::json!({}),
    };
    assert_eq!(session_stats_signal(&turn_end), SessionStatsSignal::Stale);
    assert_eq!(
        session_stats_signal(&reply("agent_error", false)),
        SessionStatsSignal::Stale
    );
    assert_eq!(
        session_stats_signal(&reply("abort", true)),
        SessionStatsSignal::Stale
    );
}

#[test]
fn any_stats_reply_answers_the_request() {
    assert_eq!(
        session_stats_signal(&reply("get_session_stats", true)),
        SessionStatsSignal::Answered
    );
    assert_eq!(
        session_stats_signal(&reply("get_session_stats", false)),
        SessionStatsSignal::Answered
    );
}

#[test]
fn other_events_leave_the_stats_alone() {
    for event in [
        reply("get_state", true),
        reply("sync", true),
        reply("set_model", false),
        Event::AgentStart,
        Event::Token { token: "x".into() },
    ] {
        assert_eq!(
            session_stats_signal(&event),
            SessionStatsSignal::Unrelated,
            "{event:?}"
        );
    }
}

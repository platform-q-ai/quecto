//! #2246: a child that has given no substantive reply yet (it crashed, or
//! is still working through its first turn) is read like any other: a
//! supervisor's first default read delivers the newest window it holds,
//! says no report was found, names every unread message it did not deliver,
//! and is acknowledged, so the next read moves on.
use super::nudge_report_tests::{TASK, serve_child};
use crate::application::tools::ports::Tool;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::agent_cmd_report::plan_default_report;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use serde_json::{Value, json};

/// A marked page of `n` messages from `first` on, none a substantive reply:
/// tool calls, their results, and blank replies.
fn answerless(first: u64, n: u64) -> Vec<Value> {
    (first..first + n)
        .map(|ordinal| match ordinal % 3 {
            0 => json!({"id":format!("t{ordinal}"),"role":"assistant","content":"",
                "toolCalls":[{"id":"c","name":"bash","arguments":"{}"}],
                "ordinal":ordinal,"turnOrigin":TASK}),
            1 => json!({"id":format!("t{ordinal}"),"role":"tool","content":"ok",
                "ordinal":ordinal,"turnOrigin":TASK}),
            _ => json!({"id":format!("t{ordinal}"),"role":"assistant","content":"  ",
                "ordinal":ordinal,"turnOrigin":TASK}),
        })
        .collect()
}

/// Two default reads, each delivered, of a child serving `newest` then
/// `older` (for any `before`): each read's result, the pages it asked for,
/// and the watermark after it.
async fn two_reads(newest: Value, older: Value) -> [(Value, usize, Option<u64>); 2] {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let seen = serve_child(&sock, vec![(None, newest), (Some("p"), older)], vec![]);
    let registry = new_registry();
    let mut entry = SubagentEntry::new(sock.clone(), 0);
    entry.persisted_liveness = crate::domain::sessions::entities::session::SubagentLiveness::Live;
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let args = r#"{"agent_id":"w1","command":"get_messages"}"#;
    let mut reads = Vec::new();
    for _ in 0..2 {
        seen.lock().unwrap().clear();
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), tool.execute(args))
            .await
            .expect("the mock child answers")
            .unwrap();
        assert!(!result.is_error, "{}", result.content);
        tool.result_delivered(args, &result);
        let pages = seen.lock().unwrap().len();
        let watermark = registry.lock().unwrap()["w1"].delivered_message_ordinal;
        reads.push((
            serde_json::from_str::<Value>(&result.content).unwrap(),
            pages,
            watermark,
        ));
    }
    reads.try_into().unwrap()
}

fn ordinals(read: &Value) -> Vec<u64> {
    read["data"]["messages"]
        .as_array()
        .expect("the window is delivered")
        .iter()
        .filter_map(|m| m["ordinal"].as_u64())
        .collect()
}

/// #2246 cold review M1: a current child names `report: null` on every
/// page while it has no substantive reply. Its first read is not paged
/// back, and delivers the newest page, acknowledged, instead of
/// `unchanged` on every read.
#[tokio::test]
async fn a_first_read_of_a_child_naming_no_report_delivers_its_window() {
    let newest = json!({"before":"p","hasMoreBefore":true,"messages":answerless(1000, 3),
        "report":null});
    let older = json!({"before":"p","hasMoreBefore":true,"messages":answerless(500, 3),
        "report":null});
    let [(first, pages, watermark), (second, again, _)] = two_reads(newest, older).await;
    assert_eq!(pages, 1, "no paging back: {first}");
    assert!(first["data"].get("reportIncomplete").is_none(), "{first}");
    assert_eq!(first["data"]["reportFound"], false, "{first}");
    assert_eq!(ordinals(&first), [1000, 1001, 1002]);
    assert_eq!(
        first["data"]["olderUnreadSkipped"],
        json!({"ranges": [{"fromOrdinal": 1, "toOrdinal": 999}], "before": "p"}),
        "{first}"
    );
    assert_eq!(watermark, Some(1002), "what it delivered is acknowledged");
    assert_eq!(again, 1, "no paging back: {second}");
    assert_eq!(second["data"], json!({"unchanged": true}), "{second}");
}

/// #2246 review finding 1, from a page that marks its messages but names
/// no report at all (no `report` key): the first read pages back to the
/// 16-page cap, then completes with the window it holds, names every unread
/// ordinal it did not deliver, and is acknowledged; the next read moves on.
#[tokio::test]
async fn a_first_read_capped_without_an_answer_completes_and_moves_on() {
    let newest = json!({"before":"p","hasMoreBefore":true,"messages":answerless(1000, 3)});
    let older = json!({"before":"p","hasMoreBefore":true,"messages":answerless(500, 3)});
    let [(first, pages, watermark), (second, again, _)] = two_reads(newest, older).await;
    assert_eq!(pages, 17, "the cap bounds the paging");
    assert!(first["data"].get("reportIncomplete").is_none(), "{first}");
    assert_eq!(first["data"]["reportFound"], false, "{first}");
    assert_eq!(ordinals(&first).iter().max(), Some(&1002), "{first}");
    assert_eq!(
        first["data"]["olderUnreadSkipped"],
        json!({"ranges": [{"fromOrdinal": 1, "toOrdinal": 499},
            {"fromOrdinal": 503, "toOrdinal": 999}], "before": "p"}),
        "{first}"
    );
    assert_eq!(watermark, Some(1002), "what it delivered is acknowledged");
    assert_eq!(again, 1, "no paging back: {second}");
    assert_eq!(second["data"], json!({"unchanged": true}), "{second}");
}

/// #2246 cold review L3: a window over the report budget is cut to its
/// newest messages; the older messages it held but did not deliver are
/// named as skipped along with the history it never held.
#[test]
fn a_window_cut_to_the_budget_names_every_message_it_did_not_deliver() {
    let messages: Vec<Value> = (5..=10)
        .map(|ordinal| {
            json!({"id":format!("t{ordinal}"),"role":"tool","content":"x".repeat(1000),
                "ordinal":ordinal,"turnOrigin":TASK})
        })
        .collect();
    let response = json!({"success":true,"data":{"messages":messages,"before":"c",
        "hasMoreBefore":true,"reportFound":false}});
    let plan = plan_default_report(&response.to_string(), 0);
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    let delivered = ordinals(&report);
    assert!(!delivered.is_empty() && delivered.len() < 6, "{report}");
    let first_delivered = delivered[0];
    assert_eq!(delivered, (first_delivered..=10).collect::<Vec<_>>());
    assert_eq!(
        report["data"]["olderUnreadSkipped"],
        json!({"ranges": [{"fromOrdinal": 1, "toOrdinal": first_delivered - 1}], "before": "c"}),
        "{report}"
    );
    assert_eq!(plan.pending.expect("acknowledgeable").ordinal, 10);
}

/// #2246 cold review M1: only a first read looks for a report; a later read
/// of a child still naming none delivers its unread messages as any later
/// read does, without saying no report was found.
#[tokio::test]
async fn a_later_read_of_a_child_naming_no_report_reads_as_before() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let newest = json!({"messages":answerless(1000, 3),"report":null});
    let _seen = serve_child(&sock, vec![(None, newest)], vec![]);
    let tool = super::nudge_report_tests::tool_delivered(&sock, Some(1000));
    let response = json!({"success":true,"data":{"messages":answerless(1000, 3),"report":null}});
    let expanded = tool
        .expand_default_get_messages_response(&sock, None, &response.to_string(), "w1")
        .await;
    let plan = plan_default_report(&expanded, 1000);
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    assert_eq!(ordinals(&report), [1001, 1002]);
    assert!(report["data"].get("reportFound").is_none(), "{report}");
    assert!(
        report["data"].get("olderUnreadSkipped").is_none(),
        "{report}"
    );
}

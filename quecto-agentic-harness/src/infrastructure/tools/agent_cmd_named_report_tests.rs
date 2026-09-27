//! #2226 review 3: the default read's named report at the backfill cap
//! (owed, later reads), its ranks, and its single read by id. Shares the
//! fixtures of `agent_cmd_nudge_report_tests.rs`.
use super::nudge_report_tests::{
    NUDGE, TASK, contents, first_read, fixture, planned, serve_child, tool_delivered, tool_over,
};
use crate::infrastructure::tools::agent_cmd_report::{
    needs_default_report_backfill, plan_default_report,
};
use serde_json::{Value, json};

/// Nudge progress at ordinals from `first`, as more pages than the cap.
fn progress_from(first: u64) -> Vec<Value> {
    let mut progress = fixture()[2..].to_vec();
    for (i, message) in progress.iter_mut().enumerate() {
        message["ordinal"] = json!(first + i as u64);
    }
    progress
}

/// Review 3 probe P1 (M1): a later read (watermark 2) after a new task
/// answered at ordinal 100, then more nudge progress than the cap pages.
/// The named answer is read by id and delivered, and the skipped unread
/// history is named precisely so it can be paged.
#[tokio::test]
async fn r3_p1_a_later_read_at_the_cap_delivers_the_named_report() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let older = json!({"before":"m3","hasMoreBefore":true,"messages":progress_from(500)[..2]});
    let seen = serve_child(&sock, vec![(None, older)], vec![("a2", "ANSWER2")]);
    let tool = tool_delivered(&sock, Some(2));
    let expanded = first_read(
        &tool,
        &sock,
        json!({"before":"m3","hasMoreBefore":true,"messages":progress_from(1000),
            "report":{"id":"a2","ordinal":100,"turnOrigin":TASK,"contentLength":7}}),
    )
    .await;
    let (report, pending) = planned(&expanded, 2);
    assert!(
        contents(&report).contains(&"ANSWER2".to_string()),
        "{report}"
    );
    assert!(report["data"].get("reportIncomplete").is_none(), "{report}");
    assert_eq!(
        report["data"]["olderUnreadSkipped"],
        json!({"ranges": [{"fromOrdinal": 3, "toOrdinal": 99}, {"fromOrdinal": 101, "toOrdinal": 499}],
            "before": "m3"}),
        "the delivered report is not among the skipped"
    );
    assert_eq!(pending, Some(1003));
    let reads = seen.lock().unwrap();
    assert_eq!(
        reads.iter().filter(|c| c["type"] == "get_message").count(),
        1
    );
}

/// Review 3 probe P1 (M1): a later read at the cap whose named report
/// cannot be read is incomplete: it never acknowledges past the report.
#[tokio::test]
async fn a_later_read_at_the_cap_that_cannot_read_the_named_report_is_incomplete() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let older = json!({"before":"m3","hasMoreBefore":true,"messages":progress_from(500)[..2]});
    let _seen = serve_child(&sock, vec![(None, older)], vec![]);
    let tool = tool_delivered(&sock, Some(2));
    let expanded = first_read(
        &tool,
        &sock,
        json!({"before":"m3","hasMoreBefore":true,"messages":progress_from(1000),
            "report":{"id":"a2","ordinal":100,"turnOrigin":TASK,"contentLength":7}}),
    )
    .await;
    let (report, pending) = planned(&expanded, 2);
    assert_eq!(report["data"]["reportIncomplete"], true, "{report}");
    assert_eq!(pending, None);
}

/// Review 3 probe P2 (M2): a first read whose named report read fails, then
/// whose fallback paging hits the cap on nudge progress, acknowledges
/// nothing: the report is still owed, and the next read retries.
#[tokio::test]
async fn r3_p2_a_failed_named_read_then_the_cap_acknowledges_nothing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let older = json!({"before":"m3","hasMoreBefore":true,"messages":progress_from(500)[..2]});
    let _seen = serve_child(&sock, vec![(None, older)], vec![]);
    let tool = tool_over(&sock);
    let expanded = first_read(
        &tool,
        &sock,
        json!({"before":"m3","hasMoreBefore":true,"messages":progress_from(1000),
            "report":{"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}}),
    )
    .await;
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(report["data"]["reportIncomplete"], true, "{report}");
    assert_eq!(pending, None, "never acknowledged past the named report");
}

/// Review 3 probe P3 (L1): after the report is read by id, the progress
/// after it is counted by ordinal, including what the page does not reach.
#[tokio::test]
async fn r3_p3_later_progress_counts_by_ordinal_after_a_named_read() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let _seen = serve_child(&sock, vec![(None, json!({}))], vec![("m2", "REPORT")]);
    let tool = tool_over(&sock);
    let expanded = first_read(
        &tool,
        &sock,
        json!({"before":"m3","hasMoreBefore":true,"messages":progress_from(1000),
            "report":{"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}}),
    )
    .await;
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(contents(&report), ["REPORT"]);
    assert_eq!(report["data"]["laterProgress"], 1001);
    assert_eq!(pending, Some(2));
}

/// Review 3 probe P4 (L2): a reply of an origin this build does not know
/// ranks below a nudge reply.
#[test]
fn r3_p4_an_unrecognised_origin_ranks_below_a_nudge_reply() {
    let msgs = json!([
        {"id":"x1","role":"assistant","content":"future","ordinal":1,"turnOrigin":"someNewProgressKind"},
        {"id":"x2","role":"assistant","content":"nudge","ordinal":2,"turnOrigin":NUDGE}
    ]);
    let plan = plan_default_report(
        &json!({"success":true,"data":{"messages":msgs}}).to_string(),
        0,
    );
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    assert_eq!(contents(&report), ["nudge"]);
    // Nor does it end the search for an answer (L3).
    let mut page = vec![msgs[0].clone()];
    page[0]["ordinal"] = json!(5);
    assert!(needs_default_report_backfill(&page, 0, true));
}

/// Review 3 L5: a report over the final-report budget read by id is read
/// once and cut once, with its notice.
#[tokio::test]
async fn a_long_named_report_is_read_once_and_cut_once() {
    use crate::infrastructure::tools::agent_cmd_report::FINAL_REPORT_BUDGET_BYTES;
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let long: &'static str = Box::leak("L".repeat(FINAL_REPORT_BUDGET_BYTES * 2).into_boxed_str());
    let seen = serve_child(&sock, vec![(None, json!({}))], vec![("m2", long)]);
    let tool = tool_over(&sock);
    let expanded = first_read(
        &tool,
        &sock,
        json!({"before":"m3","hasMoreBefore":true,"messages":fixture()[2..],
            "report":{"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":long.len()}}),
    )
    .await;
    let (report, pending) = planned(&expanded, 0);
    let delivered = &report["data"]["messages"][0];
    assert!(delivered["content"].as_str().unwrap().len() <= FINAL_REPORT_BUDGET_BYTES);
    assert_eq!(delivered["truncated"], true);
    assert!(delivered["contentNotice"].is_string());
    assert_eq!(pending, Some(2));
    assert_eq!(seen.lock().unwrap().len(), 1, "read once");
}

/// Review 3 M3: a page holding its report only as a collapsed recall stub
/// reads it whole by id, in place of the stub.
#[tokio::test]
async fn a_report_held_as_a_stub_is_read_whole_by_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let seen = serve_child(&sock, vec![(None, json!({}))], vec![("m2", "REPORT")]);
    let tool = tool_over(&sock);
    let mut page = fixture()[1..].to_vec();
    page[0]["content"] = json!("[recall(turn1:msg:assistant)]");
    page[0]["collapsed"] = json!(true);
    let named = json!({"hasMoreBefore":true,"before":"m2","messages":page,
        "report":{"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}});
    let expanded = first_read(&tool, &sock, named).await;
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(contents(&report), ["REPORT"]);
    assert_eq!(pending, Some(2));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// Review 3 M1: a later read at the cap never reads again, or delivers
/// again, a named report the supervisor already read.
#[tokio::test]
async fn a_later_read_at_the_cap_never_rereads_an_acknowledged_report() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let older = json!({"before":"m3","hasMoreBefore":true,"messages":progress_from(500)[..2]});
    let seen = serve_child(&sock, vec![(None, older)], vec![("m2", "REPORT")]);
    let tool = tool_delivered(&sock, Some(2));
    let expanded = first_read(
        &tool,
        &sock,
        json!({"before":"m3","hasMoreBefore":true,"messages":progress_from(1000),
            "report":{"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}}),
    )
    .await;
    let (report, pending) = planned(&expanded, 2);
    assert!(
        !contents(&report).contains(&"REPORT".to_string()),
        "{report}"
    );
    assert_eq!(pending, Some(1003));
    let reads = seen.lock().unwrap();
    assert_eq!(
        reads.iter().filter(|c| c["type"] == "get_message").count(),
        0
    );
}

/// A later read's page (watermark 2, the whole transcript) holding the
/// child's new answer at ordinal 8 only as a context-collapsed stub.
fn stubbed_answer_page() -> Value {
    let mut page = progress_from(3);
    page.push(json!({"id":"u9","role":"user","content":"follow up","ordinal":7,"turnOrigin":TASK}));
    page.push(json!({"id":"a2","role":"assistant","content":"[collapsed assistant message: recall(turn9:msg:assistant)]",
        "collapsed":true,"ordinal":8,"turnOrigin":TASK}));
    page.extend(progress_from(9));
    json!({"hasMoreBefore":false,"messages":page,
        "report":{"id":"a2","ordinal":8,"turnOrigin":TASK,"contentLength":7}})
}

/// Review 4 probe D1 (H1): a later read whose page holds the report only as
/// a stub reads its text by id; the stub is never delivered.
#[tokio::test]
async fn r4_d1_a_later_read_delivers_a_stubbed_report_read_by_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let seen = serve_child(&sock, vec![(None, json!({}))], vec![("a2", "ANSWER2")]);
    let tool = tool_delivered(&sock, Some(2));
    let expanded = first_read(&tool, &sock, stubbed_answer_page()).await;
    let (report, pending) = planned(&expanded, 2);
    let delivered = contents(&report);
    assert!(delivered.contains(&"ANSWER2".to_string()), "{report}");
    assert!(delivered.iter().all(|c| !c.contains("recall(")), "{report}");
    let answer = report["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "a2")
        .unwrap();
    assert!(answer.get("collapsed").is_none(), "{answer}");
    assert_eq!(pending, Some(12));
    let reads = seen.lock().unwrap();
    assert_eq!(
        reads.iter().filter(|c| c["type"] == "get_message").count(),
        1
    );
}

/// Review 4 H1: when the stubbed report cannot be read, the read is
/// incomplete, acknowledges nothing, and never delivers the stub.
#[tokio::test]
async fn a_stubbed_report_that_cannot_be_read_is_withheld_and_incomplete() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let _seen = serve_child(&sock, vec![(None, json!({}))], vec![]);
    let tool = tool_delivered(&sock, Some(2));
    let expanded = first_read(&tool, &sock, stubbed_answer_page()).await;
    let (report, pending) = planned(&expanded, 2);
    assert_eq!(report["data"]["reportIncomplete"], true, "{report}");
    assert_eq!(pending, None);
    assert!(!report.to_string().contains("recall("), "{report}");
}

/// Review 4 probe D2 (M1): a first read whose named-report read fails, then
/// whose paging reaches the report only as a stub, is incomplete: the owed
/// report is never delivered as its stub, and nothing is acknowledged.
#[tokio::test]
async fn r4_d2_a_failed_named_read_then_a_paged_stub_is_incomplete() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let mut older_msgs = fixture()[..2].to_vec();
    older_msgs[1]["content"] = json!("[collapsed assistant message: recall(turn1:msg:assistant)]");
    older_msgs[1]["collapsed"] = json!(true);
    let older = json!({"hasMoreBefore":false,"messages":older_msgs,
        "report":{"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}});
    let _seen = serve_child(
        &sock,
        vec![(Some("m3"), older.clone()), (None, older)],
        vec![],
    );
    let tool = tool_over(&sock);
    let expanded = first_read(
        &tool,
        &sock,
        json!({"before":"m3","hasMoreBefore":true,"messages":fixture()[2..],
            "report":{"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}}),
    )
    .await;
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(report["data"]["reportIncomplete"], true, "{report}");
    assert_eq!(pending, None);
    assert!(!report.to_string().contains("recall("), "{report}");
}

/// Review 4 M1: a first read whose named-report read fails, and whose page
/// holds only an unmarked reply newer than the report (so paging stops at
/// it), never acknowledges past the owed report: it is incomplete.
#[tokio::test]
async fn an_owed_report_the_read_never_reached_is_never_acknowledged_past() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let seen = serve_child(&sock, vec![(None, json!({"messages": []}))], vec![]);
    let tool = tool_over(&sock);
    let unmarked = json!({"id":"u50","role":"assistant","content":"unmarked reply","ordinal":50});
    let expanded = first_read(
        &tool,
        &sock,
        json!({"before":"u50","hasMoreBefore":true,"messages":[unmarked],
            "report":{"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}}),
    )
    .await;
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(report["data"]["reportIncomplete"], true, "{report}");
    assert_eq!(
        pending, None,
        "nothing past the owed report is acknowledged"
    );
    let kinds: Vec<Value> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|c| c["type"].clone())
        .collect();
    assert_eq!(
        kinds,
        [json!("get_message")],
        "the page alone stops the paging"
    );
}

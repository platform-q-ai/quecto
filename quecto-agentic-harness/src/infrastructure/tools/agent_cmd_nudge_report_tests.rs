//! #2226: a workflow child's replies to its engine's progress nudges never
//! replace its report. The child stamps every message with its turn's
//! origin (`turnOrigin`) and names its report on every page; the
//! supervisor's default report, its handoff budget, its backfill and its
//! collapsed-report expansion all take the latest answer to an instruction
//! as the report.
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::agent_cmd_report::{
    PageReport, REPORT_BUDGET_BYTES, bounded_report_messages, needs_default_report_backfill,
    page_report, plan_default_report,
};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use serde_json::{Value, json};
use std::io::Write;
use std::sync::{Arc, Mutex};

pub(super) const TASK: &str = "instruction";
pub(super) const NUDGE: &str = "progressNudge";

/// A task answered with `answer`, then two nudge turns replying `status`.
fn answered_then_nudged(answer: &str) -> Value {
    json!([
        {"id":"m1","role":"user","content":"task","ordinal":1,"turnOrigin":TASK},
        {"id":"m2","role":"assistant","content":answer,"ordinal":2,"turnOrigin":TASK},
        {"id":"m3","role":"user","content":"Workflow incomplete.","ordinal":3,"turnOrigin":NUDGE},
        {"id":"m4","role":"assistant","content":"status one","ordinal":4,"turnOrigin":NUDGE},
        {"id":"m5","role":"user","content":"Your last reply did not advance.","ordinal":5,"turnOrigin":NUDGE},
        {"id":"m6","role":"assistant","content":"status two","ordinal":6,"turnOrigin":NUDGE}
    ])
}

pub(super) fn fixture() -> Vec<Value> {
    answered_then_nudged("REPORT").as_array().unwrap().clone()
}

pub(super) fn contents(report: &Value) -> Vec<String> {
    report["data"]["messages"]
        .as_array()
        .expect("a report carries messages")
        .iter()
        .map(|m| m["content"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn a_first_read_reports_the_answer_and_leaves_the_nudge_turns_unread() {
    let response = json!({"success": true, "data": {"messages": fixture()}});
    let plan = plan_default_report(&response.to_string(), 0);
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    assert_eq!(contents(&report), ["REPORT"]);
    assert_eq!(report["data"]["hasMoreMessages"], true);
    assert_eq!(
        report["data"]["laterProgress"], 4,
        "the four nudge-turn messages after the report are named"
    );
    let pending = plan.pending.expect("a report is delivered");
    assert_eq!(
        pending.ordinal, 2,
        "the watermark stops at the report, so the next read brings the progress"
    );
    // A report that is the newest message names no later progress.
    let alone = json!({"success": true, "data": {"messages": &fixture()[..2]}});
    let plan = plan_default_report(&alone.to_string(), 0);
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    assert!(report["data"].get("laterProgress").is_none(), "{report}");
    // A later read covers every unread message and names none either.
    let later = json!({"success": true, "data": {"messages": fixture()}});
    let plan = plan_default_report(&later.to_string(), 2);
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    assert!(report["data"].get("laterProgress").is_none(), "{report}");
}

#[test]
fn a_later_read_gives_the_answer_the_handoff_budget_over_a_nudge_reply() {
    let answer = "A".repeat(REPORT_BUDGET_BYTES * 2);
    let candidates: Vec<Value> = answered_then_nudged(&answer).as_array().unwrap().clone();
    let bounded = bounded_report_messages(candidates, 6);
    let delivered_answer = bounded
        .messages
        .iter()
        .find(|m| m["id"] == "m2")
        .expect("the answer is delivered");
    assert_eq!(
        delivered_answer["content"].as_str().unwrap().len(),
        answer.len(),
        "the answer is the handoff, delivered whole"
    );
    assert!(delivered_answer.get("truncated").is_none());
}

#[test]
fn a_first_read_of_a_page_holding_only_nudge_turns_pages_back_for_the_answer() {
    let nudge_only = fixture()[2..].to_vec();
    assert!(needs_default_report_backfill(&nudge_only, 0, true));
    assert!(!needs_default_report_backfill(&fixture()[1..], 0, true));
    assert!(
        !needs_default_report_backfill(&nudge_only, 0, false),
        "a page with nothing older is the whole transcript"
    );
}

/// Review probe P3: an instruction turn's tool step (no text) is not the
/// answer, so a page holding only that and nudge replies still pages back.
#[test]
fn a_tool_step_of_an_instruction_turn_is_not_an_answer_to_stop_at() {
    let mut page = vec![json!({"id":"m2","role":"assistant","content":"",
        "toolCalls":[{"id":"c","name":"bash"}],"ordinal":2,"turnOrigin":TASK})];
    page.extend(fixture()[2..].iter().cloned());
    assert!(needs_default_report_backfill(&page, 0, true));
}

#[test]
fn a_page_names_its_report_only_with_a_string_id() {
    let named = json!({"report": {"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}});
    let PageReport::OffPage(report) = page_report(&fixture()[2..], &named) else {
        panic!("the report is off the page");
    };
    assert_eq!(report.id, "m2");
    let delivered = report.with_text("REPORT".into(), 6);
    assert_eq!(
        (
            &delivered["id"],
            &delivered["ordinal"],
            &delivered["turnOrigin"]
        ),
        (&json!("m2"), &json!(2), &json!(TASK))
    );
    assert_eq!(delivered["content"], "REPORT");
    assert!(delivered.get("truncated").is_none());
    assert!(report.is_unread(0) && report.is_unread(1) && !report.is_unread(2));
    assert_eq!(page_report(&fixture(), &named), PageReport::OnPage);
    assert_eq!(
        page_report(&fixture(), &json!({"report": null})),
        PageReport::NamesNone
    );
    assert_eq!(page_report(&fixture(), &json!({})), PageReport::Unnamed);
    for bad in [
        json!({"report": {"id": 2}}),
        json!({"report": {}}),
        json!({"report": "m2"}),
    ] {
        assert_eq!(page_report(&fixture(), &bad), PageReport::Unnamed, "{bad}");
    }
}

/// A mock child serving `get_messages` pages from `pages` (by `before`
/// cursor; the first page when none matches) and `get_message` reads of
/// `texts` by id (refusing any other id as not found), until the test drops
/// it. Each command it read is recorded.
pub(super) fn serve_child(
    sock_path: &std::path::Path,
    pages: Vec<(Option<&'static str>, Value)>,
    texts: Vec<(&'static str, &'static str)>,
) -> Arc<Mutex<Vec<Value>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = std::os::unix::net::UnixListener::bind(sock_path).unwrap();
    let recorded = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            while let Some(line) = crate::infrastructure::test_support::read_framed_command(&stream)
            {
                let cmd: Value = serde_json::from_str(&line).unwrap();
                recorded.lock().unwrap().push(cmd.clone());
                let data = match cmd["type"].as_str() {
                    Some("get_message") => {
                        let Some((_, text)) = texts.iter().find(|(id, _)| cmd["messageId"] == *id)
                        else {
                            let refusal = json!({"type":"response","id":cmd["id"],
                                "success":false,"error":format!("message not found: {}", cmd["messageId"])});
                            stream.write_all(format!("{refusal}\n").as_bytes()).unwrap();
                            continue;
                        };
                        let offset = cmd["offset"].as_u64().unwrap_or(0) as usize;
                        let limit = cmd["limit"].as_u64().map_or(text.len(), |l| l as usize);
                        let end = text.len().min(offset + limit);
                        json!({"offset":offset,"content":&text[offset..end],
                            "contentLength":text.len(),"hasMoreContent":end < text.len()})
                    }
                    _ => pages
                        .iter()
                        .find(|(before, _)| cmd["before"].as_str() == *before)
                        .unwrap_or(&pages[0])
                        .1
                        .clone(),
                };
                let reply = json!({"type":"response","id":cmd["id"],"success":true,"data":data});
                stream
                    .write_all(format!("{reply}\n").as_bytes())
                    .expect("the mock child writes its whole reply");
            }
        }
    });
    seen
}

pub(super) async fn first_read(
    tool: &AgentCmdTool,
    sock: &std::path::Path,
    newest: Value,
) -> String {
    let newest = json!({"success": true, "data": newest}).to_string();
    let expanded = tool
        .expand_default_get_messages_response(sock, None, &newest, "w1")
        .await;
    tool.expand_collapsed_final_report(sock, None, expanded, "w1")
        .await
}

pub(super) fn tool_over(sock_path: &std::path::Path) -> AgentCmdTool {
    tool_delivered(sock_path, None)
}

/// A tool whose child `w1` was last delivered up to `delivered`.
pub(super) fn tool_delivered(sock_path: &std::path::Path, delivered: Option<u64>) -> AgentCmdTool {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(sock_path.to_path_buf(), 0);
    entry.delivered_message_ordinal = delivered;
    registry.lock().unwrap().insert("w1".to_string(), entry);
    AgentCmdTool::new(registry)
}

/// A mock child refusing every command with `error`.
fn serve_refusing_child(
    sock_path: &std::path::Path,
    error: &'static str,
) -> Arc<Mutex<Vec<Value>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = std::os::unix::net::UnixListener::bind(sock_path).unwrap();
    let recorded = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            while let Some(line) = crate::infrastructure::test_support::read_framed_command(&stream)
            {
                let cmd: Value = serde_json::from_str(&line).unwrap();
                recorded.lock().unwrap().push(cmd.clone());
                let reply = json!({"type":"response","id":cmd["id"],"success":false,"error":error});
                stream
                    .write_all(format!("{reply}\n").as_bytes())
                    .expect("the mock child writes its whole reply");
            }
        }
    });
    seen
}

fn named_page() -> Value {
    json!({"before": "m3", "hasMoreBefore": true, "messages": fixture()[2..],
        "report": {"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}})
}

/// The planned read of `expanded`: never a stand-in for the report's text.
pub(super) fn planned(expanded: &str, delivered: u64) -> (Value, Option<u64>) {
    let plan = plan_default_report(expanded, delivered);
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    for message in report["data"]["messages"].as_array().into_iter().flatten() {
        assert!(
            message["content"]
                .as_str()
                .is_some_and(|text| !text.contains("read from the child")),
            "no stand-in text reaches the parent: {report}"
        );
    }
    (report, plan.pending.map(|pending| pending.ordinal))
}

#[tokio::test]
async fn a_default_read_finds_an_answer_older_than_the_newest_page() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let all = fixture();
    let seen = serve_child(
        &sock,
        vec![(
            Some("m3"),
            json!({"messages": all[..2], "hasMoreBefore": false}),
        )],
        vec![],
    );
    let tool = tool_over(&sock);
    let newest = json!({"before": "m3", "hasMoreBefore": true, "messages": all[2..]});
    let expanded = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        first_read(&tool, &sock, newest),
    )
    .await
    .expect("the mock child answers");
    let plan = plan_default_report(&expanded, 0);
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    assert_eq!(contents(&report), ["REPORT"]);
    assert!(report["data"].get("reportIncomplete").is_none());
    assert_eq!(seen.lock().unwrap()[0]["before"], "m3");
}

/// Review probe P4 (#2226), with the child naming its report: an answer
/// far older than the newest page is read by id, never paged back to.
#[tokio::test]
async fn a_named_report_is_read_by_id_without_paging() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let seen = serve_child(&sock, vec![(None, json!({}))], vec![("m2", "REPORT")]);
    let tool = tool_over(&sock);
    let newest = json!({"before": "m3", "hasMoreBefore": true, "messages": fixture()[2..],
        "report": {"id":"m2","ordinal":2,"turnOrigin":TASK,"contentLength":6}});
    let expanded = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        first_read(&tool, &sock, newest),
    )
    .await
    .expect("the mock child answers");
    let plan = plan_default_report(&expanded, 0);
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    assert_eq!(contents(&report), ["REPORT"]);
    assert!(report["data"].get("reportIncomplete").is_none(), "{report}");
    assert_eq!(plan.pending.expect("acknowledgeable").ordinal, 2);
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().all(|cmd| cmd["type"] == "get_message"),
        "no page is read back: {seen:?}"
    );
    assert_eq!(seen.len(), 1);
}

/// Review probe P4 (#2226), from a child that names no report: when the
/// backfill cap is reached still looking for an answer, the latest reply
/// held is reported whole and acknowledgeable, not incomplete forever.
#[tokio::test]
async fn a_backfill_cap_reached_looking_for_an_answer_reports_the_latest_reply() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    // Every older page is more nudge turns, always with older history.
    let older = json!({"before":"m3","hasMoreBefore":true,"messages":fixture()[2..4]});
    let seen = serve_child(&sock, vec![(None, older)], vec![]);
    let tool = tool_over(&sock);
    let mut newest = fixture()[2..].to_vec();
    for (i, message) in newest.iter_mut().enumerate() {
        message["ordinal"] = json!(1000 + i);
    }
    let expanded = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        first_read(
            &tool,
            &sock,
            json!({"before":"m3","hasMoreBefore":true,"messages":newest}),
        ),
    )
    .await
    .expect("the mock child answers");
    assert_eq!(seen.lock().unwrap().len(), 16, "the cap bounds the paging");
    let plan = plan_default_report(&expanded, 0);
    let report: Value = serde_json::from_str(&plan.content).unwrap();
    assert!(report["data"].get("reportIncomplete").is_none(), "{report}");
    assert_eq!(contents(&report), ["status two"]);
    assert!(plan.pending.is_some(), "the report is acknowledgeable");
}

#[tokio::test]
async fn the_collapsed_report_expanded_is_the_answer_not_a_nudge_reply() {
    let tmp = tempfile::TempDir::new().unwrap();
    // No child listens: any read attempt would mark the message.
    let sock_path = tmp.path().join("absent.sock");
    let tool = tool_over(&sock_path);
    let mut messages = answered_then_nudged("REPORT");
    messages[5]["truncated"] = json!(true);
    messages[5]["collapsed"] = json!(true);
    let response = json!({"success": true, "data": {"messages": messages}}).to_string();
    let expanded = tool
        .expand_collapsed_final_report(&sock_path, None, response.clone(), "w1")
        .await;
    assert_eq!(
        expanded, response,
        "the answer is whole, so no nudge reply is read in its place"
    );
}

/// #2226: a child that names no report (`report: null`) has none to page
/// back for, so its first read never pages, even over nudge turns alone.
#[tokio::test]
async fn a_page_naming_no_report_is_never_paged_back_from() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let seen = serve_child(&sock, vec![(None, json!({"messages": []}))], vec![]);
    let tool = tool_over(&sock);
    let mut replies = fixture()[2..].to_vec();
    for message in &mut replies {
        message["content"] = json!("");
    }
    let newest = json!({"before":"m3","hasMoreBefore":true,"messages":replies,"report":null});
    let expanded = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        first_read(&tool, &sock, newest),
    )
    .await
    .expect("no child read is awaited");
    assert!(seen.lock().unwrap().is_empty(), "no page is read back");
    assert!(!expanded.contains("reportIncomplete"), "{expanded}");
}

/// Review 2 probe P1 (#2226): the child refuses the named report's read (it
/// was cleared, or the child restarted). The first read falls back to the
/// page path and finds the report by paging back; no stand-in is delivered.
#[tokio::test]
async fn a_refused_named_report_falls_back_to_paging_for_the_answer() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let all = fixture();
    let seen = serve_child(
        &sock,
        vec![(
            Some("m3"),
            json!({"messages": all[..2], "hasMoreBefore": false}),
        )],
        vec![],
    );
    let tool = tool_over(&sock);
    let expanded = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        first_read(&tool, &sock, named_page()),
    )
    .await
    .expect("the mock child answers");
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(contents(&report), ["REPORT"]);
    assert!(report["data"].get("reportIncomplete").is_none(), "{report}");
    assert_eq!(pending, Some(2));
    let kinds: Vec<Value> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|c| c["type"].clone())
        .collect();
    assert_eq!(kinds, [json!("get_message"), json!("get_messages")]);
}

/// Review 2 probe P1 (#2226): when the fallback cannot page either, the
/// read is incomplete and acknowledges nothing; no stand-in is delivered.
#[tokio::test]
async fn a_refused_named_report_that_cannot_page_is_incomplete_and_unacknowledged() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let _seen = serve_refusing_child(&sock, "message not found: m2");
    let tool = tool_over(&sock);
    let expanded = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        first_read(&tool, &sock, named_page()),
    )
    .await
    .expect("the mock child answers");
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(report["data"]["reportIncomplete"], true, "{report}");
    assert_eq!(pending, None, "nothing is acknowledged");
    assert!(!expanded.contains("REPORT\""), "{expanded}");
}

/// Review 2 probe P2 (#2226): a historical grandchild's ancestor cannot
/// route the read ("no live inspection route"). The first read falls back
/// to paging, which the same route refuses too, so the report stays owed:
/// the read is incomplete and acknowledges nothing.
#[tokio::test]
async fn an_unroutable_named_report_falls_back_to_paging() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dead = tmp.path().join("dead.sock");
    let _refusing = serve_refusing_child(
        &dead,
        "subagent 'g1' is dead and has no live inspection route: it has ended",
    );
    let tool = tool_over(&dead);
    let expanded = first_read(&tool, &dead, named_page()).await;
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(report["data"]["reportIncomplete"], true);
    assert_eq!(pending, None);
}

/// Review 2 probe P3 (#2226): a child older than #2226 names no report and
/// marks nothing; its newest page holds its latest reply, so a first read
/// is not paged back at all.
#[tokio::test]
async fn an_old_childs_first_read_is_not_paged_back() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let strip = |mut m: Value| {
        m.as_object_mut().unwrap().remove("turnOrigin");
        m
    };
    let seen = serve_child(&sock, vec![(None, json!({"messages": []}))], vec![]);
    let tool = tool_over(&sock);
    let mut newest: Vec<Value> = fixture()[2..].iter().cloned().map(strip).collect();
    for (i, message) in newest.iter_mut().enumerate() {
        message["ordinal"] = json!(1000 + i);
    }
    let expanded = first_read(
        &tool,
        &sock,
        json!({"before":"m3","hasMoreBefore":true,"messages":newest}),
    )
    .await;
    assert!(seen.lock().unwrap().is_empty(), "no page is read back");
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(contents(&report), ["status two"]);
    assert_eq!(pending, Some(1003));
}

/// #2226 review 2: a later read whose unread progress runs past the backfill
/// cap delivers the newest window it holds, acknowledgeable, and says older
/// unread history was skipped; it is never incomplete forever.
#[tokio::test]
async fn a_later_read_past_the_cap_delivers_the_newest_window() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    // Every older page is more progress, far above the watermark.
    let mut progress = fixture()[2..4].to_vec();
    for (i, message) in progress.iter_mut().enumerate() {
        message["ordinal"] = json!(500 + i);
    }
    let older = json!({"before":"m3","hasMoreBefore":true,"messages":progress});
    let seen = serve_child(&sock, vec![(None, older)], vec![]);
    let tool = tool_delivered(&sock, Some(2));
    let mut newest = fixture()[2..].to_vec();
    for (i, message) in newest.iter_mut().enumerate() {
        message["ordinal"] = json!(1000 + i);
    }
    let expanded = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        first_read(
            &tool,
            &sock,
            json!({"before":"m3","hasMoreBefore":true,"messages":newest}),
        ),
    )
    .await
    .expect("the mock child answers");
    assert_eq!(seen.lock().unwrap().len(), 16, "the cap bounds the paging");
    let (report, pending) = planned(&expanded, 2);
    assert!(report["data"].get("reportIncomplete").is_none(), "{report}");
    assert_eq!(
        report["data"]["olderUnreadSkipped"],
        json!({"ranges": [{"fromOrdinal": 3, "toOrdinal": 499}], "before": "m3"}),
        "{report}"
    );
    assert_eq!(
        pending,
        Some(1003),
        "the watermark moves to what was delivered"
    );
}

/// #2226 review 2: a page that holds the report it names is read alone,
/// even when that report is a nudge reply (the child never answered an
/// instruction), so nothing older is paged back for.
#[tokio::test]
async fn a_page_holding_its_named_report_is_never_paged_back_from() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let seen = serve_child(&sock, vec![(None, json!({"messages": []}))], vec![]);
    let tool = tool_over(&sock);
    let newest = json!({"before":"m3","hasMoreBefore":true,"messages":fixture()[2..],
        "report": {"id":"m6","ordinal":6,"turnOrigin":NUDGE,"contentLength":10}});
    let expanded = first_read(&tool, &sock, newest).await;
    assert!(seen.lock().unwrap().is_empty(), "no page is read back");
    let (report, pending) = planned(&expanded, 0);
    assert_eq!(contents(&report), ["status two"]);
    assert_eq!(pending, Some(6));
}

#[test]
fn trial_busy_report_without_final_answer_keeps_latest_progress() {
    let messages = (2..100)
        .map(|ordinal| {
            serde_json::json!({
                "id":format!("m{ordinal}"), "role":"tool", "ordinal":ordinal,
                "content":format!("progress {ordinal}: {}", "x".repeat(200))
            })
        })
        .collect();
    let report =
        crate::infrastructure::tools::agent_cmd_report::bounded_report_messages(messages, 99);
    assert!(
        report.messages.iter().any(|m| m["ordinal"] == 99),
        "bounded report falls behind current progress"
    );
}

#[test]
fn large_incomplete_board_history_still_exposes_latest_acknowledgment() {
    let mut messages: Vec<_> = (2..101)
        .map(|ordinal| {
            serde_json::json!({
                "id":format!("m{ordinal}"), "role":"tool", "ordinal":ordinal,
                "content":"historical board snapshot ".repeat(1400),
            })
        })
        .collect();
    messages.push(
        serde_json::json!({"id":"m101", "role":"assistant", "ordinal":101,
        "content":"Acknowledged CI permission at current timestamp"}),
    );
    let response = serde_json::json!({"success":true, "data":{
        "messages":messages,"reportIncomplete":true}})
    .to_string();
    let plan = crate::infrastructure::tools::agent_cmd_report::plan_default_report(&response, 1);
    let report: serde_json::Value = serde_json::from_str(&plan.content).unwrap();
    assert!(
        report["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["ordinal"] == 101
                && m["content"].as_str().unwrap().contains("Acknowledged CI"))
    );
    assert_eq!(report["data"]["reportIncomplete"], true);
    assert!(
        plan.pending.is_none(),
        "gaps must not silently advance the unread cursor"
    );
    assert!(plan.content.len() < 4000, "report must stay bounded");
}

use super::{LONG_REQUEST_ID_REGRESSION_LEN, message_to_json_range_for_response};
use crate::domain::message::Message;
use crate::interface::cli::protocol::AgentEvent;

/// #1103 review: range fitting must include the actual response id, not a fixed
/// envelope reserve. A long id should still produce a success frame under the
/// shared protocol cap by shrinking the returned content page.
#[test]
fn ranged_get_message_accounts_for_long_request_id() {
    let body = "x".repeat(crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET);
    let msg = Message::assistant(body, vec![]);
    let request_id = "r".repeat(LONG_REQUEST_ID_REGRESSION_LEN);

    let data = message_to_json_range_for_response(&msg, Some(0), None, Some(&request_id));
    let line = AgentEvent::ok(Some(&request_id), "get_message", Some(data.clone())).to_json_line();

    assert!(
        line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET,
        "ranged response exceeded frame budget with long request id: {} > {}",
        line.len(),
        crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET
    );
    assert!(
        data["nextOffset"].as_u64().unwrap() > 0,
        "long request id should shrink the page, not return an empty page for simple content"
    );
    assert_eq!(data["hasMoreContent"].as_bool(), Some(true));
}

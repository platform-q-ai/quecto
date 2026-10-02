//! #2404 review L3: a watermark cut's archive stub resumes as a notice.
use super::*;
use crate::protocol::session_payloads::parse_resumed_messages;

#[test]
fn an_archive_stub_resumes_as_a_status_notice() {
    let messages = parse_resumed_messages(&serde_json::json!({
        "messages": [
            {"role": "user", "content": "[Context archive] 4 earlier messages of this session were archived", "id": "m2", "userKind": "archiveStub"},
            {"role": "assistant", "content": "done", "id": "m3"}
        ]
    }))
    .expect("payload should parse");
    let entries = App::resumed_chat_entries(messages);
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert!(
        matches!(&entries[0], ChatEntry::Status { text } if text.starts_with("[Context archive] 4 earlier")),
        "{entries:?}"
    );
}

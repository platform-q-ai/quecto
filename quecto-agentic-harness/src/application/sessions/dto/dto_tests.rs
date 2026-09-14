use super::*;

#[test]
fn all_has_no_prefix_and_existing_prefix_exposes_it() {
    assert_eq!(SessionListQuery::All.key_prefix(), None);
    let prefix = SessionKeyPrefix::new("chat-").unwrap();
    let query = SessionListQuery::ExistingKeyPrefix(prefix.clone());
    assert_eq!(query.key_prefix(), Some(&prefix));
    assert_ne!(query, SessionListQuery::All);
}

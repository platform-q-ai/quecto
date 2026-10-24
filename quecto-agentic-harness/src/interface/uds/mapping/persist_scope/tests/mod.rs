use super::*;

#[test]
fn absent_means_session_only() {
    assert_eq!(parse_persist_scope(None), Ok(None));
}

#[test]
fn the_two_scopes_parse() {
    assert_eq!(
        parse_persist_scope(Some("local")),
        Ok(Some(DefaultScope::Local))
    );
    assert_eq!(
        parse_persist_scope(Some("global")),
        Ok(Some(DefaultScope::Global))
    );
}

#[test]
fn anything_else_is_refused_naming_the_choices() {
    let error = parse_persist_scope(Some("everywhere")).unwrap_err();
    assert!(error.contains("\"local\" or \"global\""), "{error}");
    assert!(error.contains("everywhere"), "{error}");
    assert!(parse_persist_scope(Some("Local")).is_err());
    assert!(parse_persist_scope(Some("")).is_err());
}

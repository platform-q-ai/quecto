//! Wire parsing of the `persist` field `set_model` and `set_effort` accept
//! (#2024 S2): absent means the session only; `"local"` and `"global"` name
//! the configuration layer the default is recorded in; anything else is a
//! delivery error before any use case runs.

use crate::application::catalogue::ports::DefaultScope;

pub fn parse_persist_scope(persist: Option<&str>) -> Result<Option<DefaultScope>, String> {
    match persist {
        None => Ok(None),
        Some("local") => Ok(Some(DefaultScope::Local)),
        Some("global") => Ok(Some(DefaultScope::Global)),
        Some(other) => Err(format!(
            "persist must be \"local\" or \"global\" (got {other:?}); omit it to change the session only"
        )),
    }
}

#[cfg(test)]
mod tests {
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
}

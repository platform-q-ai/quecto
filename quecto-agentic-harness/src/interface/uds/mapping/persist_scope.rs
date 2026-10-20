//! Wire parsing of the `persist` field `set_model` and `set_effort` accept
//! (#2024 S2): absent means the session only; `"local"` and `"global"` name
//! the configuration layer the default is recorded in; anything else is a
//! delivery error before any use case runs.

use crate::application::catalogue::dto::DefaultScope;

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
#[path = "persist_scope_tests.rs"]
mod tests;

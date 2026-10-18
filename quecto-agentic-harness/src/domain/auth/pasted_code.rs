//! An authorization code pasted by hand when the browser callback cannot
//! be used (#1839, PR #1087): what a paste must carry to be trusted.

/// Why a pasted authorization code was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteRefusal {
    /// A redirect URL whose `state` is not the login's.
    StateMismatch,
    /// A redirect URL (or query) carrying no `state`.
    StateMissing,
    /// Nothing that could be a code.
    NoCode,
}

impl PasteRefusal {
    pub fn reason(self) -> &'static str {
        match self {
            PasteRefusal::StateMismatch => "state mismatch in pasted redirect URL",
            PasteRefusal::StateMissing => "pasted redirect URL is missing the state parameter",
            PasteRefusal::NoCode => "could not extract authorization code",
        }
    }
}

/// The authorization code in `input`, a pasted redirect URL, query or bare
/// code, for a login that expects `expected_state`.
///
/// URL- or query-shaped input (anything with a `?`) must carry a non-empty
/// `state` equal to `expected_state`. A bare code cannot carry state and is
/// accepted as-is, because some providers display a bare code.
pub fn code_from_paste(input: &str, expected_state: &str) -> Result<String, PasteRefusal> {
    debug_assert!(!expected_state.is_empty(), "a login always expects a state");
    let input = input.trim();
    let carries_a_query = input.contains('?');
    if carries_a_query {
        match query_param(input, "state") {
            Some(state) if state == expected_state => {}
            Some(_) => return Err(PasteRefusal::StateMismatch),
            None => return Err(PasteRefusal::StateMissing),
        }
    }
    if let Some(code) = query_param(input, "code") {
        return Ok(code);
    }
    match input.is_empty() {
        false => Ok(input.to_string()),
        true => Err(PasteRefusal::NoCode),
    }
}

/// The non-empty, URL-decoded value of `name` in `input`'s query (the text
/// after its first `?`, or all of it); an undecodable value is kept raw.
fn query_param(input: &str, name: &str) -> Option<String> {
    let query = input.split('?').nth(1).unwrap_or(input);
    let prefix = format!("{name}=");
    query
        .split('&')
        .filter_map(|param| param.strip_prefix(prefix.as_str()))
        .find(|value| !value.is_empty())
        .map(|value| {
            urlencoding::decode(value)
                .map(|decoded| decoded.into_owned())
                .unwrap_or_else(|_| value.to_string())
        })
}

#[cfg(test)]
#[path = "pasted_code_tests.rs"]
mod tests;

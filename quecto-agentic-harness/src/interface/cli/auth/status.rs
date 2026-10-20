//! `auth status` (#1842): lists each stored credential and whether it is
//! usable now, through the `InspectAuthStatus` use case.

use crate::application::auth::dto::AuthStatusError;
use crate::interface::cli::CliContext;

pub(super) fn cmd_auth_status(ctx: &CliContext, stdout: &mut String, stderr: &mut String) -> i32 {
    let Some(handles) = super::handles::status_handles(ctx, stderr) else {
        return 1;
    };
    match handles.status.execute() {
        Ok(statuses) => {
            if statuses.is_empty() {
                stdout.push_str("no credentials stored\n");
            } else {
                stdout.push_str("Credentials:\n");
                for status in &statuses {
                    let entry = credential_entry(
                        &status.provider,
                        status.method.as_str(),
                        status.state.as_str(),
                    );
                    stdout.push_str(&format!("  {entry}\n"));
                }
            }
            0
        }
        // Reported on stdout, as `auth status` always has.
        Err(AuthStatusError::Unreadable(why)) => {
            stdout.push_str(&format!("failed to read credentials: {}\n", why));
            1
        }
    }
}

/// Longest name shown whole; a longer one is cut and ends in `…`.
const SHOWN_NAME_CHARS: usize = 64;

/// A stored credential as `auth status` and `quecto status` list it:
/// "openai (oauth) — active". Every part is shown through [`shown_name`].
pub(in crate::interface::cli) fn credential_entry(
    provider: &str,
    method: &str,
    state: &str,
) -> String {
    format!(
        "{} ({}) — {}",
        shown_name(provider),
        shown_name(method),
        shown_name(state)
    )
}

/// A name read from a hand-editable file, as a terminal line shows it:
/// escaped, so a newline cannot forge a line nor an ESC sequence reach the
/// terminal, and capped at [`SHOWN_NAME_CHARS`].
pub(in crate::interface::cli) fn shown_name(name: &str) -> String {
    let mut chars = name.chars();
    let head: String = chars.by_ref().take(SHOWN_NAME_CHARS).collect();
    let cut = chars.next().is_some();
    let mut shown = shown_text(&head);
    if cut {
        shown.push('…');
    }
    shown
}

/// Text read from a hand-editable file, escaped as [`shown_name`] escapes
/// it but never cut: a long workspace path is still worth reading whole.
pub(in crate::interface::cli) fn shown_text(text: &str) -> String {
    let shown = text.escape_debug().to_string();
    debug_assert!(
        !shown.chars().any(char::is_control),
        "an escaped text holds no control character"
    );
    shown
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;

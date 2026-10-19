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
                    stdout.push_str(&format!(
                        "  {} ({}) — {}\n",
                        status.provider,
                        status.method.as_str(),
                        status.state.as_str()
                    ));
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

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;

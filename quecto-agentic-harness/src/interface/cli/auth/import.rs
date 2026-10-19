//! `auth login --import-external` (#1840): takes the OAuth credentials
//! another tool stored, through the `ImportExternalCredentials` use case,
//! and reports each one as it always has.

use super::{Output, brand};
use crate::application::auth::dto::{ImportError, ImportReport, RefreshFailure};
use crate::interface::cli::CliContext;

/// Import credentials from an external auth.json file.
pub(super) fn cmd_auth_import_external(ctx: &CliContext, out: &mut Output<'_>) -> i32 {
    let Some(handles) = super::handles::login_handles(ctx, out.stderr) else {
        return 1;
    };
    match handles.import.execute() {
        Ok(report) => present(&report, out),
        Err(error) => {
            out.stderr.push_str(&refusal(&error));
            1
        }
    }
}

/// The report's lines, in the order the import took each credential.
fn present(report: &ImportReport, out: &mut Output<'_>) -> i32 {
    for entry in &report.entries {
        let name = brand(entry.provider);
        if entry.refreshed {
            out.stdout
                .push_str(&format!("{name} token expired, refreshing...\n"));
        }
        match &entry.stored {
            Ok(()) => out
                .stdout
                .push_str(&format!("Imported {name} OAuth credential\n")),
            Err(why) => out.stderr.push_str(&format!(
                "auth login: failed to store {name} credential: {why}\n"
            )),
        }
    }
    if let Some((provider, failure)) = &report.aborted {
        let name = brand(*provider);
        out.stdout
            .push_str(&format!("{name} token expired, refreshing...\n"));
        out.stderr.push_str(&match failure {
            RefreshFailure::Rejected(why) => {
                format!("auth login: failed to refresh {name} token: {why}\n")
            }
            RefreshFailure::Unavailable(why) => {
                format!("auth login: failed to create runtime: {why}\n")
            }
            RefreshFailure::InsideAsyncRuntime => {
                "auth login: cannot start a login from inside an async runtime\n".to_string()
            }
        });
        return 1;
    }
    match report.imported() {
        0 => {
            out.stderr
                .push_str("auth login: no OAuth credentials found in auth.json\n");
            1
        }
        imported => {
            out.stdout
                .push_str(&format!("Imported {imported} credential(s)\n"));
            0
        }
    }
}

fn refusal(error: &ImportError) -> String {
    match error {
        ImportError::NotFound { path } => format!("auth login: auth.json not found at {path}\n"),
        ImportError::Unreadable { path, reason } => {
            format!("auth login: failed to read {path}: {reason}\n")
        }
        ImportError::Malformed(reason) => {
            format!("auth login: failed to parse auth.json: {reason}\n")
        }
    }
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;

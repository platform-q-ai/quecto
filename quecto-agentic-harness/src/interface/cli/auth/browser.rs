//! `auth login` through a browser (#1839): shows the authorization URL,
//! waits for the callback or reads a pasted code, and presents the
//! outcome. What is trusted and stored is the `LogInWithOAuth` use case's.

use super::{Output, flush_stdout, login_refusal, read_line, skip_browser_callback};
use crate::application::auth::dto::{
    AuthorizationReturn, CallbackFailure, LoginError, OAuthLoginAttempt,
};
use crate::domain::auth::provider::LoginProvider;
use crate::interface::cli::CliContext;

/// Why a REPL command or supplied input reads a paste at once, worded as
/// a provider failure like every other reason the callback brought no code.
fn callback_skipped() -> String {
    crate::domain::shared::error::DomainError::Provider(
        "browser callback skipped for supplied input or non-interactive REPL".to_string(),
    )
    .to_string()
}

/// OAuth browser-based login flow.
pub(super) fn cmd_auth_login_oauth(
    ctx: &CliContext,
    provider: &str,
    out: &mut Output<'_>,
    reader: &mut dyn std::io::BufRead,
) -> i32 {
    let Some(handles) = super::handles::login_handles(ctx, out.stderr) else {
        return 1;
    };
    let login = &handles.oauth_login;
    let attempt = match login.begin(provider) {
        Ok(attempt) => attempt,
        Err(error) => return refuse(out, &error),
    };
    out.stdout.push_str(&authorization_prompt(&attempt));
    flush_stdout(ctx, out);
    let accepted = match attempt.returns() {
        AuthorizationReturn::PastedCode => match read_line(reader) {
            Ok(line) => login.accept_pasted(&attempt, &line),
            Err(why) => return read_failure(out, &why),
        },
        AuthorizationReturn::Loopback { .. } => {
            // Supplied input and non-interactive REPL commands paste at once.
            let failed = match skip_browser_callback(ctx) {
                true => callback_skipped(),
                false => match login.await_callback(&attempt) {
                    Ok(code) => return complete(login, attempt, code, out),
                    Err(CallbackFailure::PasteInstead(why)) => why,
                    Err(CallbackFailure::Refused(error)) => return refuse(out, &error),
                },
            };
            out.stdout.push_str(&format!(
                "\nCallback failed ({failed}). Paste the authorization code or redirect URL:\n"
            ));
            flush_stdout(ctx, out);
            match read_line(reader) {
                Ok(line) => login.accept_pasted(&attempt, &line),
                Err(why) => return read_failure(out, &why),
            }
        }
    };
    match accepted {
        Ok(code) => complete(login, attempt, code, out),
        Err(error) => refuse(out, &error),
    }
}

fn complete(
    login: &crate::application::auth::use_cases::LogInWithOAuth,
    attempt: OAuthLoginAttempt,
    code: crate::application::auth::dto::AuthorizationCode,
    out: &mut Output<'_>,
) -> i32 {
    match login.complete(attempt, code) {
        Ok(stored) => {
            if stored.account_id_missing {
                out.stderr
                    .push_str("auth login: warning — could not extract account ID from token\n");
            }
            out.stdout.push_str(&format!(
                "{} OAuth credential stored successfully\n",
                capitalize(stored.provider.id())
            ));
            0
        }
        Err(error) => refuse(out, &error),
    }
}

fn refuse(out: &mut Output<'_>, error: &LoginError) -> i32 {
    out.stderr.push_str(&login_refusal(error));
    1
}

fn read_failure(out: &mut Output<'_>, why: &str) -> i32 {
    out.stderr.push_str(&format!("auth login: {why}\n"));
    1
}

/// What a browser login shows before it waits: where to authorize and how
/// the code comes back.
fn authorization_prompt(attempt: &OAuthLoginAttempt) -> String {
    let url = attempt.authorization_url();
    match (attempt.provider(), attempt.returns()) {
        (_, AuthorizationReturn::PastedCode) => format!(
            "Open this URL in your browser to authenticate with {}:\n\n{url}\n\n\
             After authorizing, you'll be redirected to a page with a URL containing\n\
             an authorization code. Copy the FULL URL or code and paste it below.\n\n\
             Paste the authorization code:\n",
            brand(attempt.provider())
        ),
        (provider, AuthorizationReturn::Loopback { waiting_on }) => format!(
            "Open this URL in your browser to authenticate with {}:\n\n{url}\n\n\
             Waiting for browser callback on {waiting_on} ...\n\
             (If the browser doesn't open, copy the URL above and paste it manually)\n",
            brand(provider)
        ),
    }
}

fn brand(provider: LoginProvider) -> &'static str {
    match provider {
        LoginProvider::OpenAi => "OpenAI",
        LoginProvider::Anthropic => "Anthropic",
        LoginProvider::Xai => "xAI (SuperGrok / X Premium+)",
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;

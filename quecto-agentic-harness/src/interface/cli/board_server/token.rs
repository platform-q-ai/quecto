//! `quecto board-server token add|revoke|list` (epic #2705 B5): a running
//! server's device tokens, administered over its Unix socket only (the
//! server refuses its admin routes over TCP). `add` prints the token once,
//! alone on stdout; `list` gives names and dates, never a token.
use std::path::PathBuf;

use serde_json::Value;

use crate::infrastructure::http::board_service::listener::ServiceMethod;
use crate::infrastructure::http::board_service::local_client::local_request;

/// A token command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenAction {
    Add(String),
    Revoke(String),
    List,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenOptions {
    pub action: TokenAction,
    pub socket: PathBuf,
}

/// `args` (after `token`) as a token command.
/// # Errors
/// Another action, a missing or extra name, or a missing `--socket`.
pub fn parse(args: &[String]) -> Result<TokenOptions, String> {
    let refused = |why: &str| Err(format!("board-server: {why}"));
    let (action, rest) = match args {
        [action, name, rest @ ..] if action == "add" && name_shaped(name) => {
            (TokenAction::Add(name.clone()), rest)
        }
        [action, name, rest @ ..] if action == "revoke" && name_shaped(name) => {
            (TokenAction::Revoke(name.clone()), rest)
        }
        [action, rest @ ..] if action == "list" => (TokenAction::List, rest),
        _ => {
            return refused(
                "token takes add <name>, revoke <name> or list \
                 (a name is 1 to 64 of a-z, 0-9 and '-', starting with a letter or digit)",
            );
        }
    };
    match rest {
        [option, socket] if option == "--socket" => Ok(TokenOptions {
            action,
            socket: PathBuf::from(socket),
        }),
        _ => refused("token needs --socket <path> (and nothing else)"),
    }
}

/// Whether `name` is shaped as a token name, an allowlist: 1 to 64 of
/// `a-z`, `0-9` and `-`, the first a letter or digit (the server judges
/// the rest of a slug's rules).
fn name_shaped(name: &str) -> bool {
    let bytes = name.as_bytes();
    let body = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-';
    match bytes.split_first() {
        Some((first, rest)) => {
            bytes.len() <= 64
                && (first.is_ascii_lowercase() || first.is_ascii_digit())
                && rest.iter().all(body)
        }
        None => false,
    }
}

/// Runs `options` against the server on its socket: 0 when it did, else 1.
pub fn run(options: &TokenOptions, stdout: &mut String, stderr: &mut String) -> i32 {
    let (method, path, body) = match &options.action {
        TokenAction::Add(name) => (
            ServiceMethod::Post,
            "/v1/admin/tokens",
            serde_json::json!({ "name": name }).to_string(),
        ),
        TokenAction::Revoke(name) => (
            ServiceMethod::Post,
            "/v1/admin/tokens/revoke",
            serde_json::json!({ "name": name }).to_string(),
        ),
        TokenAction::List => (ServiceMethod::Get, "/v1/admin/tokens", String::new()),
    };
    let answer = match local_request(&options.socket, method, path, body.as_bytes()) {
        Ok(answer) => answer,
        Err(error) => {
            let socket = options.socket.display();
            stderr.push_str(&format!(
                "board-server: no answer from a server on {socket}: {error}\n"
            ));
            return 1;
        }
    };
    let reply: Value = serde_json::from_slice(&answer.body).unwrap_or(Value::Null);
    let text = |key: &str| reply[key].as_str().unwrap_or_default().to_string();
    match (&options.action, answer.status) {
        (TokenAction::Add(_), 201) => {
            stdout.push_str(&format!("{}\n", text("token")));
            stderr.push_str(&format!(
                "board-server: token '{}' added; it is shown only this once\n",
                text("name")
            ));
            0
        }
        (TokenAction::Revoke(_), 200) => {
            stdout.push_str(&format!("board-server: token '{}' revoked\n", text("name")));
            0
        }
        (TokenAction::List, 200) => {
            let tokens = reply["tokens"].as_array().cloned().unwrap_or_default();
            for token in &tokens {
                let field = |key: &str| token[key].as_str().unwrap_or_default().to_string();
                stdout.push_str(&format!("{}\t{}\n", field("name"), field("created")));
            }
            if tokens.is_empty() {
                stderr.push_str("board-server: no tokens\n");
            }
            0
        }
        (_, status) => {
            stderr.push_str(&format!(
                "board-server: {} ({status} {})\n",
                text("error"),
                text("code")
            ));
            1
        }
    }
}

#[cfg(test)]
#[path = "token_tests.rs"]
mod tests;

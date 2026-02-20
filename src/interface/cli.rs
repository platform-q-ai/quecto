use std::collections::HashMap;
use std::path::PathBuf;

use super::gateway::Gateway;
use crate::application::onboard;
use crate::domain::session::Session;
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
use crate::infrastructure::config::Config;

/// Result of a CLI invocation, capturing stdout, stderr, and exit code.
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Runtime context for CLI commands, allowing override of paths for testing.
#[derive(Debug, Clone, Default)]
pub struct CliContext {
    /// Override for the base directory (default: ~/.quecto).
    pub base_dir: Option<PathBuf>,
}

impl CliContext {
    /// Resolve the base directory: use override if set, otherwise default.
    fn base_dir(&self) -> PathBuf {
        self.base_dir
            .clone()
            .or_else(onboard::default_base_dir)
            .unwrap_or_else(|| PathBuf::from(".quecto"))
    }
}

/// Run the CLI with the given args, printing to real stdout/stderr.
/// Returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    let ctx = CliContext::default();

    // Handle gateway specially — it's a long-running async process
    if args.len() >= 2 && args[1] == "gateway" {
        return cmd_gateway_run(&ctx);
    }

    let output = run_with_output(args, &ctx);
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    output.exit_code
}

/// Run the CLI with the given args and context, capturing all output for testing.
pub fn run_with_output(args: Vec<String>, ctx: &CliContext) -> CliOutput {
    let mut stdout = String::new();
    let mut stderr = String::new();

    let exit_code = if args.len() < 2 {
        help_text(&mut stdout);
        1
    } else {
        match args[1].as_str() {
            "onboard" => cmd_onboard(ctx, &mut stdout, &mut stderr),
            "agent" => cmd_agent(&args[2..], &mut stdout, &mut stderr),
            "gateway" => {
                stdout.push_str("Use 'quecto gateway' to start the gateway service\n");
                0
            }
            "status" => cmd_status(ctx, &mut stdout, &mut stderr),
            "auth" => cmd_auth(ctx, &args[2..], &mut stdout, &mut stderr),
            "cron" => {
                stderr.push_str("cron: not yet implemented\n");
                1
            }
            "skills" => cmd_skills(ctx, &args[2..], &mut stdout, &mut stderr),
            "version" | "--version" | "-v" => {
                version_text(&mut stdout);
                0
            }
            other => {
                stderr.push_str(&format!("Unknown command: {other}\n"));
                help_text(&mut stdout);
                1
            }
        }
    };

    CliOutput {
        stdout,
        stderr,
        exit_code,
    }
}

fn cmd_agent(args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let mut session_name: Option<String> = None;
    let mut message: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-s" | "--session" => {
                if i + 1 < args.len() {
                    session_name = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: -s requires a session name\n");
                    return 1;
                }
            }
            "-m" | "--message" => {
                if i + 1 < args.len() {
                    message = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: -m requires a message\n");
                    return 1;
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    let session_id = session_name.as_deref().unwrap_or("default");
    let session_key = Session::build_key("cli", session_id);

    stdout.push_str(&format!("session: {}\n", session_key));

    if let Some(msg) = message {
        // One-shot mode: would process the message through agent loop
        // For now, report the session and message
        stdout.push_str(&format!("message: {}\n", msg));
        stderr.push_str("agent: LLM chat not yet implemented\n");
        1
    } else {
        // Interactive mode placeholder
        stderr.push_str("agent: interactive mode not yet implemented\n");
        1
    }
}

fn cmd_auth(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let base = ctx.base_dir();

    if args.is_empty() {
        stderr.push_str("auth: missing subcommand (login, logout, status)\n");
        return 1;
    }

    match args[0].as_str() {
        "login" => cmd_auth_login(&base, &args[1..], stdout, stderr),
        "logout" => cmd_auth_logout(&base, &args[1..], stdout, stderr),
        "status" => cmd_auth_status(&base, stdout),
        other => {
            stderr.push_str(&format!("auth: unknown subcommand '{}'\n", other));
            1
        }
    }
}

/// Known provider names accepted by the auth commands.
const KNOWN_PROVIDERS: &[&str] = &["openai", "anthropic"];

fn cmd_auth_login(
    base: &std::path::Path,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let mut provider: Option<String> = None;
    let mut token: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--provider" => {
                if i + 1 < args.len() {
                    provider = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth login: --provider requires a value\n");
                    return 1;
                }
            }
            "--token" => {
                if i + 1 < args.len() {
                    token = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth login: --token requires a value\n");
                    return 1;
                }
            }
            other if other.starts_with("--") => {
                stderr.push_str(&format!("auth login: unknown flag '{}'\n", other));
                return 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let Some(provider) = provider else {
        stderr.push_str("auth login: --provider is required\n");
        return 1;
    };

    if !KNOWN_PROVIDERS.contains(&provider.as_str()) {
        stderr.push_str(&format!(
            "auth login: unknown provider '{}'. Known: {}\n",
            provider,
            KNOWN_PROVIDERS.join(", ")
        ));
        return 1;
    }

    let Some(token) = token else {
        stderr.push_str("auth login: --token is required (interactive login not yet supported)\n");
        return 1;
    };

    let token = token.trim().to_string();
    if token.is_empty() {
        stderr.push_str("auth login: --token value must not be empty\n");
        return 1;
    }

    let store = CredentialStore::new(base);
    match store.store(Credential {
        provider: provider.clone(),
        token,
        method: AuthMethod::Token,
        expires_at: None,
    }) {
        Ok(()) => {
            stdout.push_str(&format!("Credential stored for {}\n", provider));
            0
        }
        Err(e) => {
            stderr.push_str(&format!("auth login: failed to store credential: {}\n", e));
            1
        }
    }
}

fn cmd_auth_logout(
    base: &std::path::Path,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let mut provider: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--provider" => {
                if i + 1 < args.len() {
                    provider = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth logout: --provider requires a value\n");
                    return 1;
                }
            }
            other if other.starts_with("--") => {
                stderr.push_str(&format!("auth logout: unknown flag '{}'\n", other));
                return 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let Some(provider) = provider else {
        stderr.push_str("auth logout: --provider is required\n");
        return 1;
    };

    let store = CredentialStore::new(base);
    match store.remove(&provider) {
        Ok(true) => {
            stdout.push_str(&format!("Credential removed for {}\n", provider));
            0
        }
        Ok(false) => {
            stdout.push_str(&format!("no credential found for {}\n", provider));
            0
        }
        Err(e) => {
            stderr.push_str(&format!(
                "auth logout: failed to remove credential: {}\n",
                e
            ));
            1
        }
    }
}

fn cmd_auth_status(base: &std::path::Path, stdout: &mut String) -> i32 {
    let store = CredentialStore::new(base);
    match store.status_summary() {
        Ok(statuses) => {
            if statuses.is_empty() {
                stdout.push_str("no credentials stored\n");
            } else {
                stdout.push_str("Credentials:\n");
                for s in &statuses {
                    stdout.push_str(&format!("  {} ({}) — {}\n", s.provider, s.method, s.status));
                }
            }
            0
        }
        Err(e) => {
            stdout.push_str(&format!("failed to read credentials: {}\n", e));
            1
        }
    }
}

fn cmd_skills(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let base = ctx.base_dir();
    let ws_skills = base.join("workspace").join("skills");

    if args.is_empty() {
        stderr.push_str("skills: missing subcommand (list, remove, install)\n");
        return 1;
    }

    match args[0].as_str() {
        "list" => {
            if !ws_skills.is_dir() {
                stdout.push_str("No skills installed\n");
                return 0;
            }
            let mut found = false;
            if let Ok(entries) = std::fs::read_dir(&ws_skills) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        stdout.push_str(&format!("  {}\n", name));
                        found = true;
                    }
                }
            }
            if !found {
                stdout.push_str("No skills installed\n");
            }
            0
        }
        "remove" => {
            if args.len() < 2 {
                stderr.push_str("skills remove: missing skill name\n");
                return 1;
            }
            let name = &args[1];
            let skill_dir = ws_skills.join(name);
            if !skill_dir.is_dir() {
                stderr.push_str(&format!("skill '{}' not found\n", name));
                return 1;
            }
            match std::fs::remove_dir_all(&skill_dir) {
                Ok(_) => {
                    stdout.push_str(&format!("'{}' removed successfully\n", name));
                    0
                }
                Err(e) => {
                    stderr.push_str(&format!("failed to remove skill '{}': {}\n", name, e));
                    1
                }
            }
        }
        "install" => {
            stderr.push_str("skills install: not yet implemented\n");
            1
        }
        other => {
            stderr.push_str(&format!("skills: unknown subcommand '{}'\n", other));
            1
        }
    }
}

fn cmd_status(ctx: &CliContext, stdout: &mut String, stderr: &mut String) -> i32 {
    let base = ctx.base_dir();
    let config_path = base.join("config.json");

    stdout.push_str("quecto Status\n");
    stdout.push_str(&format!("  Config:    {}\n", config_path.display()));

    let config = if config_path.exists() {
        match Config::load(config_path.to_str().unwrap_or("")) {
            Ok(c) => c,
            Err(e) => {
                stderr.push_str(&format!("failed to load config: {}\n", e));
                return 1;
            }
        }
    } else {
        stderr.push_str("config not found; run 'quecto onboard' first\n");
        return 1;
    };

    let ws = config.workspace_path();
    stdout.push_str(&format!("  Workspace: {}\n", ws));
    stdout.push_str(&format!("  Model:     {}\n", config.agents.defaults.model));

    // Provider availability
    let openai_status = if config.providers.openai.api_key.is_empty() {
        "not set".to_string()
    } else {
        "configured".to_string()
    };
    let anthropic_status = if config.providers.anthropic.api_key.is_empty() {
        "not set".to_string()
    } else {
        "configured".to_string()
    };
    stdout.push_str(&format!("  OpenAI API:    {}\n", openai_status));
    stdout.push_str(&format!("  Anthropic API: {}\n", anthropic_status));

    // Telegram status
    let telegram_status = if config.channels.telegram.enabled {
        "enabled"
    } else {
        "disabled"
    };
    stdout.push_str(&format!("  Telegram:      {}\n", telegram_status));

    // Heartbeat status
    let heartbeat_status = if config.heartbeat.enabled {
        format!("enabled ({}s)", config.heartbeat.interval)
    } else {
        "disabled".to_string()
    };
    stdout.push_str(&format!("  Heartbeat:     {}\n", heartbeat_status));

    0
}

/// Run the gateway as a long-running async service.
/// This creates a tokio runtime and blocks until shutdown.
fn cmd_gateway_run(ctx: &CliContext) -> i32 {
    let base_dir = ctx.base_dir();
    let config_path = base_dir.join("config.json");

    if !config_path.exists() {
        eprintln!("config not found at {}", config_path.display());
        eprintln!("run 'quecto onboard' first");
        return 1;
    }

    // Load config with env overrides
    let env_overrides: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();

    let config = match Config::load_with_env(config_path.to_str().unwrap_or(""), &env_overrides) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to load config: {}", e);
            return 1;
        }
    };

    let gateway = Gateway::new(config, base_dir);

    let rt = tokio::runtime::Runtime::new().unwrap();
    match rt.block_on(gateway.run()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("gateway error: {}", e);
            1
        }
    }
}

fn cmd_onboard(ctx: &CliContext, stdout: &mut String, stderr: &mut String) -> i32 {
    let base_dir = ctx.base_dir();
    match onboard::run_onboard(&base_dir) {
        Ok(result) => {
            if result.already_existed {
                stdout.push_str("Config already exists\n");
                stdout.push_str(&format!("  path: {}\n", result.config_path.display()));
            } else {
                stdout.push_str("quecto is ready!\n");
                stdout.push_str(&format!("  config:    {}\n", result.config_path.display()));
                stdout.push_str(&format!(
                    "  workspace: {}\n",
                    result.workspace_path.display()
                ));
            }
            0
        }
        Err(e) => {
            stderr.push_str(&format!("onboard failed: {e}\n"));
            1
        }
    }
}

fn version_text(out: &mut String) {
    out.push_str(&format!("quecto {}\n", env!("CARGO_PKG_VERSION")));
}

fn help_text(out: &mut String) {
    out.push_str(&format!(
        "quecto - Personal AI Assistant v{}\n",
        env!("CARGO_PKG_VERSION")
    ));
    out.push_str("\nUsage: quecto <command>\n");
    out.push_str("\nCommands:\n");
    out.push_str("  onboard     Initialize configuration and workspace\n");
    out.push_str("  agent       Interact with the agent directly\n");
    out.push_str("  auth        Manage authentication (login, logout, status)\n");
    out.push_str("  gateway     Start the Telegram gateway\n");
    out.push_str("  status      Show status\n");
    out.push_str("  cron        Manage scheduled tasks\n");
    out.push_str("  skills      Manage skills (install, list, remove)\n");
    out.push_str("  version     Show version information\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        let mut v = vec!["quecto".to_string()];
        if !s.is_empty() {
            v.extend(s.split_whitespace().map(String::from));
        }
        v
    }

    fn default_ctx() -> CliContext {
        CliContext::default()
    }

    fn assert_contains_all(haystack: &str, needles: &[&str]) {
        for needle in needles {
            assert!(
                haystack.contains(needle),
                "expected output to contain '{needle}', got:\n{haystack}"
            );
        }
    }

    #[test]
    fn test_no_args_shows_help() {
        let out = run_with_output(vec!["quecto".to_string()], &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert_contains_all(
            &out.stdout,
            &[
                "Usage: quecto <command>",
                "onboard",
                "agent",
                "gateway",
                "status",
                "auth",
                "cron",
                "skills",
                "version",
            ],
        );
    }

    #[test]
    fn test_version_command() {
        let out = run_with_output(args("version"), &default_ctx());
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("quecto"));
        assert!(out.stdout.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn test_version_flag() {
        let out = run_with_output(args("--version"), &default_ctx());
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("quecto"));
    }

    #[test]
    fn test_version_short_flag() {
        let out = run_with_output(args("-v"), &default_ctx());
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("quecto"));
    }

    #[test]
    fn test_unknown_command() {
        let out = run_with_output(args("foobar"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("Unknown command: foobar"));
        assert!(out.stdout.contains("Usage: quecto <command>"));
    }

    #[test]
    fn test_onboard_creates_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("onboard"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("quecto is ready"));
        assert!(tmp.path().join("config.json").exists());
        assert!(tmp.path().join("workspace").exists());
    }

    #[test]
    fn test_onboard_existing_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("onboard"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Config already exists"));
    }

    #[test]
    fn test_status_shows_summary() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_json = r#"{
            "agents": { "defaults": { "model": "gpt-4o" } },
            "providers": {
                "openai": { "api_key": "sk-test" },
                "anthropic": { "api_key": "" }
            }
        }"#;
        std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert_contains_all(
            &out.stdout,
            &[
                "quecto Status",
                "Config:",
                "Workspace:",
                "Model:",
                "gpt-4o",
                "OpenAI API:",
                "configured",
                "Anthropic API:",
                "not set",
            ],
        );
    }

    #[test]
    fn test_status_no_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("config not found"));
    }

    #[test]
    fn test_status_redacts_api_keys() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_json = r#"{
            "providers": {
                "openai": { "api_key": "sk-super-secret-12345" }
            }
        }"#;
        std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.contains("sk-super-secret-12345"));
        assert!(out.stdout.contains("configured"));
    }

    #[test]
    fn test_auth_missing_subcommand() {
        let out = run_with_output(args("auth"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("missing subcommand"));
    }

    #[test]
    fn test_cron_not_implemented() {
        let out = run_with_output(args("cron"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("not yet implemented"));
    }

    #[test]
    fn test_gateway_subcommand_hint() {
        let out = run_with_output(args("gateway"), &default_ctx());
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("gateway"));
    }

    #[test]
    fn test_agent_no_args() {
        let out = run_with_output(args("agent"), &default_ctx());
        assert!(out.stdout.contains("session: cli:default"));
        assert!(out.stderr.contains("interactive mode not yet implemented"));
    }

    #[test]
    fn test_agent_with_session_and_message() {
        let out = run_with_output(args("agent -s test-session -m Hello"), &default_ctx());
        assert!(out.stdout.contains("session: cli:test-session"));
        assert!(out.stdout.contains("message: Hello"));
    }

    #[test]
    fn test_agent_session_flag_missing_value() {
        let out = run_with_output(args("agent -s"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("-s requires a session name"));
    }

    #[test]
    fn test_agent_message_flag_missing_value() {
        let out = run_with_output(args("agent -m"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("-m requires a message"));
    }

    #[test]
    fn test_agent_long_flags() {
        let out = run_with_output(args("agent --session my-sess --message Hi"), &default_ctx());
        assert!(out.stdout.contains("session: cli:my-sess"));
        assert!(out.stdout.contains("message: Hi"));
    }

    #[test]
    fn test_skills_no_subcommand() {
        let out = run_with_output(args("skills"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("missing subcommand"));
    }

    #[test]
    fn test_skills_unknown_subcommand() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("skills foobar"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown subcommand"));
    }

    #[test]
    fn test_skills_list_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("skills list"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("No skills installed"));
    }

    #[test]
    fn test_skills_list_with_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("workspace").join("skills").join("weather");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("skills list"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("weather"));
    }

    #[test]
    fn test_skills_remove_missing_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("skills remove"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("missing skill name"));
    }

    #[test]
    fn test_skills_remove_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("skills remove nonexistent"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("not found"));
    }

    #[test]
    fn test_skills_install_not_implemented() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("skills install"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("not yet implemented"));
    }

    #[test]
    fn test_status_shows_telegram_and_heartbeat() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_json = r#"{
            "providers": { "openai": { "api_key": "sk-test" } },
            "channels": { "telegram": { "enabled": true, "token": "123:ABC" } },
            "heartbeat": { "enabled": true, "interval": 300 }
        }"#;
        std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Telegram:"));
        assert!(out.stdout.contains("enabled"));
        assert!(out.stdout.contains("Heartbeat:"));
        assert!(out.stdout.contains("300s"));
    }

    #[test]
    fn test_status_disabled_telegram_and_heartbeat() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_json = r#"{
            "providers": { "openai": { "api_key": "sk-test" } },
            "channels": { "telegram": { "enabled": false } },
            "heartbeat": { "enabled": false }
        }"#;
        std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("disabled"));
    }

    #[test]
    fn test_gateway_no_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        // cmd_gateway_run uses eprintln directly, so we test through run_with_output
        // (which routes "gateway" to a hint message since the real gateway path
        // goes through run() -> cmd_gateway_run)
        let out = run_with_output(args("gateway"), &ctx);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_help_text_includes_all_commands() {
        let mut out = String::new();
        help_text(&mut out);
        assert_contains_all(
            &out,
            &[
                "onboard", "agent", "auth", "gateway", "status", "cron", "skills", "version",
            ],
        );
    }

    #[test]
    fn test_version_text_includes_semver() {
        let mut out = String::new();
        version_text(&mut out);
        assert!(out.starts_with("quecto "));
        // Should match semver pattern
        let version_part = out.trim().strip_prefix("quecto ").unwrap();
        let parts: Vec<&str> = version_part.split('.').collect();
        assert_eq!(parts.len(), 3, "expected semver, got: {}", version_part);
    }

    #[test]
    fn test_cli_context_default_base_dir() {
        let ctx = CliContext::default();
        let base = ctx.base_dir();
        // Should end with .quecto (either from home dir or fallback)
        assert!(
            base.to_string_lossy().contains(".quecto") || base.to_string_lossy().contains("quecto"),
            "base dir should contain 'quecto': {}",
            base.display()
        );
    }

    #[test]
    fn test_cli_context_override_base_dir() {
        let ctx = CliContext {
            base_dir: Some(PathBuf::from("/tmp/test-quecto")),
        };
        assert_eq!(ctx.base_dir(), PathBuf::from("/tmp/test-quecto"));
    }

    // --- Auth CLI tests ---

    #[test]
    fn test_auth_login_stores_token_openai() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(
            args("auth login --provider openai --token sk-test-openai"),
            &ctx,
        );
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("stored"));

        let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
        assert!(store.exists("openai").unwrap());
        let cred = store.get("openai").unwrap().unwrap();
        assert_eq!(cred.token, "sk-test-openai");
    }

    #[test]
    fn test_auth_login_stores_token_anthropic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(
            args("auth login --provider anthropic --token sk-ant-test"),
            &ctx,
        );
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("stored"));
    }

    #[test]
    fn test_auth_login_missing_provider() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth login --token sk-test"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("--provider"));
    }

    #[test]
    fn test_auth_login_missing_token() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth login --provider openai"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("--token"));
    }

    #[test]
    fn test_auth_logout_removes_credential() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        // First store a credential
        let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
        store
            .store(crate::infrastructure::auth::credential_store::Credential {
                provider: "openai".to_string(),
                token: "sk-test".to_string(),
                method: crate::infrastructure::auth::credential_store::AuthMethod::Token,
                expires_at: None,
            })
            .unwrap();

        let out = run_with_output(args("auth logout --provider openai"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("removed"));
        assert!(!store.exists("openai").unwrap());
    }

    #[test]
    fn test_auth_logout_nonexistent_is_noop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth logout --provider openai"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("no credential"));
    }

    #[test]
    fn test_auth_status_shows_credentials() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
        store
            .store(crate::infrastructure::auth::credential_store::Credential {
                provider: "openai".to_string(),
                token: "sk-test".to_string(),
                method: crate::infrastructure::auth::credential_store::AuthMethod::Token,
                expires_at: None,
            })
            .unwrap();

        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("openai"));
        assert!(out.stdout.contains("active"));
    }

    #[test]
    fn test_auth_status_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("no credentials"));
    }

    #[test]
    fn test_auth_status_expired() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
        store
            .store(crate::infrastructure::auth::credential_store::Credential {
                provider: "anthropic".to_string(),
                token: "expired-tok".to_string(),
                method: crate::infrastructure::auth::credential_store::AuthMethod::Token,
                expires_at: Some(0),
            })
            .unwrap();

        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("anthropic"));
        assert!(out.stdout.contains("expired"));
    }

    #[test]
    fn test_auth_no_subcommand() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("missing subcommand"));
    }

    #[test]
    fn test_auth_unknown_subcommand() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth foobar"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown subcommand"));
    }

    #[test]
    fn test_auth_login_unknown_provider_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth login --provider groq --token sk-test"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown provider"));
        assert!(out.stderr.contains("groq"));
    }

    #[test]
    fn test_auth_login_empty_token_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        // We pass a token that's all whitespace
        let v = vec![
            "quecto".to_string(),
            "auth".to_string(),
            "login".to_string(),
            "--provider".to_string(),
            "openai".to_string(),
            "--token".to_string(),
            "   ".to_string(),
        ];
        let out = run_with_output(v, &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("must not be empty"));
    }

    #[test]
    fn test_auth_login_unknown_flag_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth login --provider openai --tokn sk-test"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown flag"));
        assert!(out.stderr.contains("--tokn"));
    }

    #[test]
    fn test_auth_logout_unknown_flag_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
        };
        let out = run_with_output(args("auth logout --provder openai"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown flag"));
    }
}

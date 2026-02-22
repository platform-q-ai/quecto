use std::collections::HashMap;

use super::CliContext;
use crate::application::onboard;
use crate::infrastructure::config::Config;

pub(crate) fn cmd_status(ctx: &CliContext, stdout: &mut String, stderr: &mut String) -> i32 {
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
pub(crate) fn cmd_gateway_run(ctx: &CliContext) -> i32 {
    use super::super::gateway::Gateway;

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

pub(crate) fn cmd_onboard(ctx: &CliContext, stdout: &mut String, stderr: &mut String) -> i32 {
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

pub(crate) fn cmd_skills(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let base = ctx.base_dir();
    let ws_skills = base.join("workspace").join("skills");

    if args.is_empty() {
        stderr.push_str("skills: missing subcommand (list, remove, install)\n");
        return 1;
    }

    match args[0].as_str() {
        "list" => cmd_skills_list(&base, stdout),
        "remove" => {
            if args.len() < 2 {
                stderr.push_str("skills remove: missing skill name\n");
                return 1;
            }
            let name = &args[1];
            if !crate::domain::skill::is_valid_skill_name(name) {
                stderr.push_str(&format!("skill '{}' not found\n", name));
                return 1;
            }
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

/// List installed skills with their descriptions from SKILL.md frontmatter.
fn cmd_skills_list(base: &std::path::Path, stdout: &mut String) -> i32 {
    use crate::domain::skill::SkillLoader;
    use crate::infrastructure::persistence::skill_loader::FileSkillLoader;

    let workspace = base.join("workspace");
    let loader = FileSkillLoader::new(&workspace);
    let skills = match loader.list() {
        Ok(s) => s,
        Err(_) => {
            stdout.push_str("No skills installed\n");
            return 0;
        }
    };
    if skills.is_empty() {
        stdout.push_str("No skills installed\n");
    } else {
        for skill in &skills {
            stdout.push_str(&format!("  {} — {}\n", skill.name, skill.description));
        }
    }
    0
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

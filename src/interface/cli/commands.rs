use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use super::CliContext;
use crate::application::onboard;
use crate::infrastructure::config::Config;
use crate::infrastructure::persistence::workspace_store::FileOnboardStore;

const DEFAULT_GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com";
const SKILL_DOWNLOAD_TIMEOUT_SECS: u64 = 10;
const MAX_SKILL_MD_BYTES: usize = 256 * 1024;

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

    0
}

pub(crate) fn cmd_onboard(ctx: &CliContext, stdout: &mut String, stderr: &mut String) -> i32 {
    let base_dir = ctx.base_dir();
    let store = FileOnboardStore::new(&base_dir);
    match onboard::run_onboard(&store) {
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
    let workspace = resolve_workspace_for_skills(&base);
    let ws_skills = workspace.join("skills");
    let github_raw_base = ctx
        .github_raw_base_url
        .as_deref()
        .unwrap_or(DEFAULT_GITHUB_RAW_BASE);

    if args.is_empty() {
        stderr.push_str("skills: missing subcommand (list, remove, install)\n");
        return 1;
    }

    match args[0].as_str() {
        "list" => cmd_skills_list(&workspace, stdout),
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
        "install" => match cmd_skills_install(&ws_skills, args, github_raw_base) {
            Ok(message) => {
                stdout.push_str(&message);
                0
            }
            Err(message) => {
                stderr.push_str(&message);
                1
            }
        },
        other => {
            stderr.push_str(&format!("skills: unknown subcommand '{}'\n", other));
            1
        }
    }
}

fn cmd_skills_install(
    ws_skills: &std::path::Path,
    args: &[String],
    github_raw_base: &str,
) -> Result<String, String> {
    if args.len() < 2 {
        return Err("skills install: missing skill path\n".to_string());
    }

    let skill_path = &args[1];
    let Some((owner, repo, name)) = parse_github_skill_path(skill_path) else {
        return Err("skills install: invalid skill path\n".to_string());
    };

    let skill_dir = ws_skills.join(name);
    if skill_dir.exists() {
        return Err(format!("skill '{}' already exists\n", name));
    }

    let skill_md = match download_skill_markdown(github_raw_base, owner, repo, name) {
        Ok(content) => content,
        Err(e) => {
            return Err(format!("skills install: failed to download skill: {}\n", e));
        }
    };

    let (yaml, _) = crate::domain::skill::split_skill_md(&skill_md)
        .ok_or_else(|| "skills install: invalid SKILL.md frontmatter\n".to_string())?;
    let parsed = crate::domain::skill::SkillFrontmatter::parse(yaml)
        .ok_or_else(|| "skills install: invalid SKILL.md frontmatter\n".to_string())?;
    if !crate::domain::skill::validate_frontmatter(&parsed) {
        return Err("skills install: invalid SKILL.md frontmatter\n".to_string());
    }
    if parsed.name != name {
        return Err(format!(
            "skills install: invalid SKILL.md name '{}' (expected '{}')\n",
            parsed.name, name
        ));
    }

    if let Err(e) = std::fs::create_dir_all(ws_skills) {
        return Err(format!(
            "failed to create skills directory '{}': {}\n",
            ws_skills.display(),
            e
        ));
    }

    if let Err(e) = std::fs::create_dir(&skill_dir) {
        return Err(format!(
            "failed to create skill directory '{}': {}\n",
            name, e
        ));
    }

    match std::fs::symlink_metadata(&skill_dir) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            let _ = std::fs::remove_dir_all(&skill_dir);
            return Err(format!(
                "invalid skill directory '{}'\n",
                skill_dir.display()
            ));
        }
        Ok(_) => {}
        Err(e) => {
            return Err(format!(
                "failed to validate skill directory '{}': {}\n",
                skill_dir.display(),
                e
            ));
        }
    }

    let skill_md_path = skill_dir.join("SKILL.md");
    let write_result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&skill_md_path)
            .map_err(|e| format!("failed to open SKILL.md for '{}': {}", name, e))?;
        file.write_all(skill_md.as_bytes())
            .map_err(|e| format!("failed to write SKILL.md for '{}': {}", name, e))?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_dir_all(&skill_dir);
        return Err(format!("{}\n", e));
    }

    Ok(format!("'{}' installed\n", name))
}

fn parse_github_skill_path(path: &str) -> Option<(&str, &str, &str)> {
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let skill = parts.next()?;

    if parts.next().is_some() {
        return None;
    }

    if !is_valid_github_slug(owner)
        || !is_valid_github_slug(repo)
        || !crate::domain::skill::is_valid_skill_name(skill)
    {
        return None;
    }

    Some((owner, repo, skill))
}

fn is_valid_github_slug(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.starts_with('.')
        && !value.ends_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn download_skill_markdown(
    github_raw_base: &str,
    owner: &str,
    repo: &str,
    skill: &str,
) -> Result<String, String> {
    let base = github_raw_base.trim_end_matches('/');
    let paths = ["main", "master"]
        .iter()
        .map(|branch| format!("{base}/{owner}/{repo}/{branch}/{skill}/SKILL.md"))
        .collect::<Vec<_>>();

    if tokio::runtime::Handle::try_current().is_ok() {
        return Err("cannot run skills install from within an async runtime".to_string());
    }

    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("runtime init failed: {e}"))?;
    rt.block_on(download_skill_markdown_async(paths))
}

async fn download_skill_markdown_async(paths: Vec<String>) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(SKILL_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("client init failed: {e}"))?;
    let mut last_error: Option<String> = None;

    for url in paths {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let mut body = Vec::new();
                let mut resp = resp;
                while let Some(chunk) = resp
                    .chunk()
                    .await
                    .map_err(|e| format!("failed to read response body: {e}"))?
                {
                    body.extend_from_slice(&chunk);
                    if body.len() > MAX_SKILL_MD_BYTES {
                        return Err(format!(
                            "response body too large (>{MAX_SKILL_MD_BYTES} bytes)"
                        ));
                    }
                }

                return String::from_utf8(body).map_err(|e| format!("invalid UTF-8 body: {e}"));
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
                continue;
            }
            Ok(resp) => {
                last_error = Some(format!("HTTP {} from {url}", resp.status()));
            }
            Err(e) => {
                last_error = Some(format!("request error for {url}: {e}"));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "skill not found in repository".to_string()))
}

/// List installed skills with their descriptions from SKILL.md frontmatter.
fn cmd_skills_list(workspace: &std::path::Path, stdout: &mut String) -> i32 {
    use crate::domain::skill::SkillLoader;
    use crate::infrastructure::persistence::skill_loader::FileSkillLoader;

    let loader = FileSkillLoader::new(workspace);
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

fn resolve_workspace_for_skills(base: &std::path::Path) -> std::path::PathBuf {
    let fallback = base.join("workspace");
    let config_path = base.join("config.json");
    if !config_path.exists() {
        return fallback;
    }

    let env_overrides: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();

    let Some(config_path_str) = config_path.to_str() else {
        return fallback;
    };

    match Config::load_with_env(config_path_str, &env_overrides) {
        Ok(config) => {
            let workspace = std::path::PathBuf::from(config.workspace_path());
            if workspace.is_absolute() {
                workspace
            } else {
                base.join(workspace)
            }
        }
        Err(_) => fallback,
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

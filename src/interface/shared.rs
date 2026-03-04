//! Shared utility functions used by CLI, REPL, and gateway modules.

use std::collections::HashMap;
use std::path::Path;

use chrono::Local;

use crate::domain::skill::SkillLoader;
use crate::infrastructure::auth::credential_store::Credential;
use crate::infrastructure::persistence::skill_loader::FileSkillLoader;

/// Load all workspace skills and concatenate their non-empty body content.
///
/// Skills without valid YAML frontmatter are silently skipped.
pub fn load_skill_prompt(base_dir: &Path) -> String {
    let workspace = base_dir.join("workspace");
    let loader = FileSkillLoader::new(&workspace);
    let skills = match loader.list() {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    skills
        .iter()
        .filter(|s| !s.content.is_empty())
        .map(|s| s.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Merge skill content with an optional user-provided system prompt.
pub fn merge_prompts(skill_prompt: &str, user_prompt: &Option<String>) -> String {
    match user_prompt {
        Some(up) if !up.is_empty() => format!("{}\n\n{}", skill_prompt, up),
        _ => skill_prompt.to_string(),
    }
}

/// Generate a preamble with the current local date, time, and timezone.
///
/// Example: `"Current date and time: Saturday, March 1, 2026 at 10:30:15 AM GMT+1"`
///
/// This preamble is intentionally richer than any date metadata injected by
/// LLM providers (e.g. Anthropic's `"Current date: 2026-03-01"`). It includes:
/// - **Day-of-week** (e.g. "Saturday") — useful for scheduling context
/// - **Full time with seconds** — critical for cron scheduling precision
/// - **Timezone identifier** — essential for timezone-aware tasks
///
/// The resulting duplication with provider-side date metadata is expected and
/// harmless; this preamble is authoritative for time-aware agent operations.
/// See issue #104 for discussion.
pub fn datetime_preamble() -> String {
    let now = Local::now();
    // Format: "Saturday, March 1, 2026 at 10:30:15 AM GMT+1"
    let date_str = now.format("%A, %B %-d, %Y at %I:%M:%S %p %Z").to_string();
    format!("Current date and time: {}", date_str)
}

/// Build a complete system prompt with datetime preamble, skills, and user prompt.
///
/// Always prepends the current date/time/timezone so the agent knows
/// what "today" and "now" mean — critical for cron scheduling and
/// time-aware tasks. Combines:
/// 1. Datetime preamble (always present, see [`datetime_preamble`])
/// 2. Skill content (if any skills are loaded)
/// 3. User-provided system prompt (if any)
///
/// Note: Some LLM providers inject their own date metadata (e.g. "Current
/// date: 2026-03-01"). The quecto preamble is richer (day-of-week, full
/// time, timezone) and takes precedence for time-aware operations.
/// This duplication is intentional — see issue #104.
pub fn build_system_prompt(skill_prompt: &str, user_prompt: &Option<String>) -> String {
    let preamble = datetime_preamble();
    let merged = merge_prompts(skill_prompt, user_prompt);
    if merged.is_empty() {
        preamble
    } else {
        format!("{}\n\n{}", preamble, merged)
    }
}

/// Resolve an API key for a provider from a credential snapshot.
///
/// The credential store snapshot takes priority over the config-file key.
/// Expired credentials are ignored (falls back to config key).
/// Operates on a pre-loaded snapshot to avoid redundant file I/O.
pub fn resolve_api_key(
    config_key: &str,
    creds: &HashMap<String, Credential>,
    provider: &str,
) -> String {
    if let Some(cred) = creds.get(provider) {
        if !cred.is_expired() {
            return cred.token.clone();
        }
    }
    config_key.to_string()
}

/// Resolve an API key for a provider, automatically refreshing expired OAuth tokens.
///
/// If the credential is expired and has a refresh token, attempts to refresh it
/// and update the credential store. Falls back to config key on failure.
pub fn resolve_api_key_with_refresh(
    config_key: &str,
    store: &crate::infrastructure::auth::credential_store::CredentialStore,
    provider: &str,
    rt: &tokio::runtime::Runtime,
) -> String {
    let creds = store.load_snapshot().unwrap_or_default();

    if let Some(cred) = creds.get(provider) {
        if !cred.is_expired() {
            return cred.token.clone();
        }

        // Token is expired — try to refresh if we have a refresh token
        if cred.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth {
            if let Some(ref refresh_token) = cred.refresh_token {
                if let Some(oauth_config) =
                    crate::infrastructure::auth::oauth::OAuthConfig::for_provider(provider)
                {
                    tracing::info!("refreshing expired OAuth token for {}", provider);

                    // Dispatch to the correct refresh function based on provider
                    let refresh_result = match provider {
                        "openai" => {
                            rt.block_on(crate::infrastructure::auth::oauth::refresh_openai_token(
                                &oauth_config,
                                refresh_token,
                            ))
                        }
                        _ => rt.block_on(
                            crate::infrastructure::auth::oauth::refresh_anthropic_token(
                                &oauth_config,
                                refresh_token,
                            ),
                        ),
                    };

                    match refresh_result {
                        Ok(token_resp) => {
                            let expires_at =
                                chrono::Utc::now().timestamp() + token_resp.expires_in as i64 - 300;
                            let account_id = if provider == "openai" {
                                crate::infrastructure::auth::oauth::extract_openai_account_id(
                                    &token_resp.access_token,
                                )
                            } else {
                                None
                            };
                            let new_cred = Credential {
                                provider: provider.to_string(),
                                token: token_resp.access_token.clone(),
                                method:
                                    crate::infrastructure::auth::credential_store::AuthMethod::OAuth,
                                expires_at: Some(expires_at),
                                refresh_token: Some(token_resp.refresh_token),
                                account_id,
                            };
                            if let Err(e) = store.store(new_cred) {
                                tracing::warn!(
                                    "failed to persist refreshed token for {}: {}",
                                    provider,
                                    e
                                );
                            }
                            return token_resp.access_token;
                        }
                        Err(e) => {
                            tracing::warn!("failed to refresh OAuth token for {}: {}", provider, e);
                        }
                    }
                }
            }
        }
    }

    config_key.to_string()
}

/// Check which providers have expired credentials and need re-authentication.
///
/// Operates on a pre-loaded snapshot to avoid redundant file I/O.
/// Resolve the effective workspace directory for an agent or REPL invocation.
///
/// When `no_sandbox` is `true`, the agent should operate from the **process's
/// current working directory** rather than the configured workspace path. This
/// lets users run `quecto --no-sandbox` from any directory and have the agent
/// see that directory as its root, matching how every other CLI tool behaves.
///
/// When sandbox is enabled (the default), the configured workspace path is used.
///
/// # Arguments
///
/// * `config_workspace` — the resolved workspace path from config (already `~`-expanded)
/// * `no_sandbox` — whether the `--no-sandbox` flag was passed
pub fn resolve_agent_workspace(config_workspace: &str, no_sandbox: bool) -> std::path::PathBuf {
    if no_sandbox {
        match std::env::current_dir() {
            Ok(cwd) => cwd,
            Err(e) => {
                // CWD is unavailable (e.g. deleted directory). Fall back to the
                // config workspace and emit a warning so the user is not silently
                // misled about which directory the agent is operating from.
                tracing::warn!(
                    error = %e,
                    fallback = config_workspace,
                    "--no-sandbox: current_dir() failed, falling back to config workspace"
                );
                std::path::PathBuf::from(config_workspace)
            }
        }
    } else {
        std::path::PathBuf::from(config_workspace)
    }
}

pub fn check_provider_readiness(creds: &HashMap<String, Credential>) -> Vec<String> {
    creds
        .values()
        .filter(|c| c.is_expired())
        .map(|c| c.provider.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frontmatter(name: &str, desc: &str, body: &str) -> String {
        format!("---\nname: {}\ndescription: {}\n---\n{}", name, desc, body)
    }

    #[test]
    fn test_load_skill_prompt_with_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("workspace").join("skills").join("weather");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            frontmatter("weather", "Weather forecasts", "Fetch weather data"),
        )
        .unwrap();
        let prompt = load_skill_prompt(tmp.path());
        assert_eq!(prompt, "Fetch weather data");
    }

    #[test]
    fn test_load_skill_prompt_empty_when_no_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prompt = load_skill_prompt(tmp.path());
        assert!(prompt.is_empty());
    }

    #[test]
    fn test_load_skill_prompt_skips_invalid_frontmatter() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("workspace").join("skills").join("bad");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "No frontmatter").unwrap();
        let prompt = load_skill_prompt(tmp.path());
        assert!(prompt.is_empty());
    }

    #[test]
    fn test_merge_prompts_skill_only() {
        let result = merge_prompts("Skill content", &None);
        assert_eq!(result, "Skill content");
    }

    #[test]
    fn test_merge_prompts_skill_and_user() {
        let result = merge_prompts("Skill content", &Some("User prompt".to_string()));
        assert_eq!(result, "Skill content\n\nUser prompt");
    }

    #[test]
    fn test_merge_prompts_skill_with_empty_user() {
        let result = merge_prompts("Skill content", &Some(String::new()));
        assert_eq!(result, "Skill content");
    }

    #[test]
    fn test_datetime_preamble_contains_current_date() {
        let preamble = datetime_preamble();
        assert!(
            preamble.starts_with("Current date and time:"),
            "expected preamble to start with 'Current date and time:', got: {}",
            preamble
        );
        // Should contain a year (4 digits)
        let year = chrono::Local::now().format("%Y").to_string();
        assert!(
            preamble.contains(&year),
            "expected preamble to contain current year {}, got: {}",
            year,
            preamble
        );
    }

    #[test]
    fn test_build_system_prompt_datetime_only() {
        let result = build_system_prompt("", &None);
        assert!(result.starts_with("Current date and time:"));
        // No trailing skills/user content
        assert!(!result.contains("\n\n"));
    }

    #[test]
    fn test_build_system_prompt_with_skills() {
        let result = build_system_prompt("Skill content", &None);
        assert!(result.starts_with("Current date and time:"));
        assert!(result.contains("Skill content"));
    }

    #[test]
    fn test_build_system_prompt_with_skills_and_user() {
        let result = build_system_prompt("Skill content", &Some("Be helpful".to_string()));
        assert!(result.starts_with("Current date and time:"));
        assert!(result.contains("Skill content"));
        assert!(result.contains("Be helpful"));
    }

    #[test]
    fn test_build_system_prompt_with_user_only() {
        let result = build_system_prompt("", &Some("Be helpful".to_string()));
        assert!(result.starts_with("Current date and time:"));
        assert!(result.contains("Be helpful"));
    }

    /// Issue #104: The quecto datetime preamble is intentionally richer than
    /// provider-injected "Current date:" metadata. It includes day-of-week,
    /// full time with seconds, and timezone — critical for cron scheduling
    /// and time-aware tasks.
    #[test]
    fn test_datetime_preamble_includes_day_of_week_time_and_timezone() {
        let preamble = datetime_preamble();

        // Must include a day-of-week name
        let days = [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ];
        assert!(
            days.iter().any(|d| preamble.contains(d)),
            "preamble should include day-of-week, got: {}",
            preamble
        );

        // Must include AM/PM time with seconds (e.g. "06:55:58 PM")
        assert!(
            preamble.contains("AM") || preamble.contains("PM"),
            "preamble should include AM/PM time, got: {}",
            preamble
        );

        // Must include colons in the time portion (HH:MM:SS)
        let colon_count = preamble.chars().filter(|c| *c == ':').count();
        assert!(
            colon_count >= 2,
            "preamble should include HH:MM:SS (at least 2 colons), got: {}",
            preamble
        );

        // After AM/PM, there should be a timezone identifier
        let ampm_pos = preamble.find("AM").or_else(|| preamble.find("PM"));
        if let Some(pos) = ampm_pos {
            let after = &preamble[pos + 2..];
            assert!(
                !after.trim().is_empty(),
                "preamble should have timezone after AM/PM, got: {}",
                preamble
            );
        }
    }
}

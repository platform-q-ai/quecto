use super::{CliContext, selected_config_missing};
use crate::infrastructure::config::Config;

pub(crate) fn cmd_status(ctx: &CliContext, stdout: &mut String, stderr: &mut String) -> i32 {
    let selection = match ctx.config_selection() {
        Ok(selection) => selection,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    let config_path = selection.path();

    stdout.push_str("quecto Status\n");
    stdout.push_str(&format!("  Config:    {}\n", config_path.display()));

    if let Some(msg) = selected_config_missing(config_path, selection.must_exist()) {
        stderr.push_str(&format!("{msg}\n"));
        return 1;
    }

    // Missing global config is not an error: quecto is zero-config (defaults apply).
    let config = match Config::load(config_path.to_str().unwrap_or("")) {
        Ok(c) => c,
        Err(e) => {
            stderr.push_str(&format!(
                "failed to load config {}: {}\n",
                config_path.display(),
                e
            ));
            return 1;
        }
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

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

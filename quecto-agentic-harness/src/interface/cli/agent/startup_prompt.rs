//! Agent startup prompt assembly from initialization-time sources.

use crate::interface::cli::CliContext;

pub(super) fn initialization_directory(ctx: &CliContext) -> Result<std::path::PathBuf, String> {
    if let Some(cwd) = &ctx.cwd {
        return Ok(cwd.clone());
    }
    std::env::current_dir()
        .map_err(|error| format!("failed to determine agent initialization directory: {error}"))
}

pub(super) fn load_agents_instructions(
    ctx: &CliContext,
    stderr: &mut String,
) -> Option<Option<String>> {
    let initialization_dir = match initialization_directory(ctx) {
        Ok(directory) => directory,
        Err(error) => {
            stderr.push_str(&error);
            stderr.push('\n');
            return None;
        }
    };
    match crate::infrastructure::agents_instructions::load_agents_instructions(&initialization_dir)
    {
        Ok(instructions) => Some(instructions),
        Err(error) => {
            stderr.push_str(&error);
            stderr.push('\n');
            None
        }
    }
}

pub(super) fn compose(
    agents_instructions: Option<&str>,
    explicit_system_prompt: Option<&str>,
    spawned: bool,
    extension_prompt_snippets: &str,
) -> String {
    crate::interface::shared::build_agent_system_prompt(
        agents_instructions,
        explicit_system_prompt,
        spawned,
        extension_prompt_snippets,
    )
}

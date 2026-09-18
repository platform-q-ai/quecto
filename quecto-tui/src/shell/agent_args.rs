//! The `quecto agent` argument line `quecto-tui` spawns from its own flags:
//! every forwarded flag in one place, so a new pass-through (`--model`,
//! `--effort`, #2024 S2) is a line here and a row in the README table.

use super::cli::CliFlags;

pub(crate) fn build_agent_args(flags: &CliFlags) -> Vec<String> {
    let mut args = vec!["agent".to_string(), "--mode".to_string(), "uds".to_string()];
    if flags.persist {
        args.push("--persist".to_string());
    }
    if flags.workflow {
        args.push("--workflow".to_string());
    }
    if flags.workflow_disabled {
        args.push("--no-workflow".to_string());
    }
    if flags.workflow_guards {
        args.push("--workflow-guards".to_string());
    }
    if let Some(ref path) = flags.config_path {
        args.push("--config".to_string());
        args.push(path.to_string_lossy().to_string());
    }
    if let Some(ref prompt) = flags.system_prompt {
        args.push("--system".to_string());
        args.push(prompt.clone());
    }
    for tool in &flags.disable_tools {
        args.push("--disable-tool".to_string());
        args.push(tool.clone());
    }
    if let Some(ref model) = flags.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    if let Some(ref effort) = flags.effort {
        args.push("--effort".to_string());
        args.push(effort.clone());
    }
    args
}

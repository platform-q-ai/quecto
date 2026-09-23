use super::config_loading::{layer_diagnostics, load_selected_config, overlay_summary};
use super::{CliContext, selected_config_missing};

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

    // Missing global config is not an error: quecto is zero-config (defaults
    // apply). Status never prompts for overlay trust; it reports it.
    let Some(build_configuration) = ctx.configuration else {
        stderr.push_str("configuration capability not composed\n");
        return 1;
    };
    let loaded = match load_selected_config(
        build_configuration,
        &ctx.base_dir(),
        &selection,
        false,
        &Default::default(),
        false,
    ) {
        Ok(loaded) => loaded,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    stdout.push_str(&format!(
        "  Overlay:   {}\n",
        overlay_summary(&loaded.sources)
    ));
    for line in layer_diagnostics(&loaded.sources) {
        stderr.push_str(&line);
        stderr.push('\n');
    }

    let config = loaded.config;
    let ws = config.workspace_path();
    stdout.push_str(&format!("  Workspace: {}\n", ws));
    stdout.push_str(&format!("  Model:     {}\n", config.agents.defaults.model));
    stdout.push_str(&format!(
        "  Effort:    {}\n",
        config
            .agents
            .defaults
            .effort
            .as_deref()
            .unwrap_or("default")
    ));

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

    // Status is config-only when no runtime was composed in this process.
    // Never infer broker health from a file or absent published snapshot.
    if let Some(runtime) =
        crate::infrastructure::catalogue_registry::runtime_store_for(&ctx.base_dir()).current()
    {
        for slot in &runtime.admission_binding_diagnostic.unbound_slots {
            stderr.push_str(&format!(
                "warning: {}\n",
                crate::domain::state_snapshot::AdmissionBindingWarning::new(slot).message
            ));
        }
    }

    0
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

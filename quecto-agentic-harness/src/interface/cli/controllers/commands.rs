use crate::application::configuration::dto::ConfigurationInspection;
use crate::application::configuration::ports::StoredCredentialsUnavailable;
use crate::interface::cli::auth::{credential_entry, shown_name, shown_text};
use crate::interface::cli::config_loading::{layer_diagnostics, overlay_summary};
use crate::interface::cli::{CliContext, selected_config_missing};

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
    let handles = match ctx.configuration_handles(false) {
        Ok(handles) => handles,
        Err(why) => {
            stderr.push_str(&format!("{why}\n"));
            return 1;
        }
    };
    let inspection = match handles.inspect.execute(&selection) {
        Ok(inspection) => inspection,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    present(&inspection, stdout, stderr);

    // Status is config-only when no runtime was composed in this process.
    // Never infer broker health from a file or absent published snapshot.
    // Admission warnings are emitted by agent startup, where composition has
    // validated effective bindings and credentials.

    0
}

/// The inspection as `quecto status` prints it, after its `Config:` line.
fn present(inspection: &ConfigurationInspection, stdout: &mut String, stderr: &mut String) {
    stdout.push_str(&format!(
        "  Overlay:   {}\n",
        overlay_summary(&inspection.sources)
    ));
    for line in layer_diagnostics(&inspection.sources) {
        stderr.push_str(&line);
        stderr.push('\n');
    }
    let settings = &inspection.settings;
    stdout.push_str(&format!(
        "  Workspace: {}\n",
        shown_text(&settings.workspace)
    ));
    stdout.push_str(&format!("  Model:     {}\n", shown_text(&settings.model)));
    stdout.push_str(&format!(
        "  Effort:    {}\n",
        shown_text(settings.effort.as_deref().unwrap_or("default"))
    ));
    let presence = |set: bool| if set { "configured" } else { "not set" };
    stdout.push_str(&format!(
        "  OpenAI API:    {}\n",
        presence(settings.openai_key)
    ));
    stdout.push_str(&format!(
        "  Anthropic API: {}\n",
        presence(settings.anthropic_key)
    ));
    if !settings.endpoint_keys.is_empty() {
        stdout.push_str(&format!(
            "  Endpoint keys: {}\n",
            settings
                .endpoint_keys
                .iter()
                .map(|prefix| shown_name(prefix))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let credentials = match &inspection.stored_credentials {
        Ok(stored) if stored.is_empty() => "none stored".to_string(),
        Ok(stored) => stored
            .iter()
            .map(|credential| {
                credential_entry(&credential.provider, &credential.method, &credential.state)
            })
            .collect::<Vec<_>>()
            .join(", "),
        Err(StoredCredentialsUnavailable::Malformed) => "unreadable (malformed store)".to_string(),
        Err(StoredCredentialsUnavailable::PermissionDenied) => {
            "unreadable (permission denied)".to_string()
        }
        Err(StoredCredentialsUnavailable::Unreadable) => "unreadable".to_string(),
    };
    stdout.push_str(&format!("  Credentials:   {credentials}\n"));
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "commands_inspect_tests.rs"]
mod inspect_tests;

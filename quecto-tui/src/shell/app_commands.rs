//! Built-in slash-command registry for the TUI (#1067 factored out of
//! `app.rs`): the single source of truth `handle_submit` validates against.

use std::sync::LazyLock;

use crate::components::autocomplete::SlashCommand;

/// Built-in slash commands.
static BUILTIN_COMMANDS: LazyLock<Vec<SlashCommand>> = LazyLock::new(|| {
    vec![
        SlashCommand {
            name: "clear".into(),
            description: "Clear conversation history".into(),
        },
        SlashCommand {
            name: "quit".into(),
            description: "Exit TUI".into(),
        },
        SlashCommand {
            name: "exit".into(),
            description: "Exit TUI".into(),
        },
        SlashCommand {
            name: "help".into(),
            description: "Show keyboard shortcuts".into(),
        },
        SlashCommand {
            name: "hotkeys".into(),
            description: "Show keyboard shortcuts".into(),
        },
        SlashCommand {
            name: "new".into(),
            description: "Start a new session".into(),
        },
        SlashCommand {
            name: "tab-new".into(),
            description: "Open a new connecting tab".into(),
        },
        SlashCommand {
            name: "tab-close".into(),
            description: "Close the active tab (detach agent)".into(),
        },
        SlashCommand {
            name: "tab-next".into(),
            description: "Switch to the next tab".into(),
        },
        SlashCommand {
            name: "tab-prev".into(),
            description: "Switch to the previous tab".into(),
        },
        SlashCommand {
            name: "session".into(),
            description: "Show session info".into(),
        },
        SlashCommand {
            name: "refresh-tui".into(),
            description: "Refresh the TUI view".into(),
        },
        SlashCommand {
            name: "delete-all-subagents".into(),
            description: "Terminate and remove all subagents".into(),
        },
        SlashCommand {
            name: "resume".into(),
            description: "Resume a persisted CLI session".into(),
        },
        SlashCommand {
            name: "model".into(),
            description: "Switch model".into(),
        },
        SlashCommand {
            name: "effort".into(),
            description: "Switch reasoning effort".into(),
        },
        SlashCommand {
            name: "workflow".into(),
            description: "Show workflow status and hotkeys".into(),
        },
        SlashCommand {
            name: "workflow-auto".into(),
            description: "Toggle workflow auto-continue".into(),
        },
        SlashCommand {
            name: "workflow-nudge".into(),
            description: "Toggle workflow completion nudge".into(),
        },
    ]
});

pub(super) fn builtin_commands() -> &'static [SlashCommand] {
    &BUILTIN_COMMANDS
}

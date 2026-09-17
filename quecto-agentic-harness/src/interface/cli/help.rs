//! `quecto help` and `quecto version` text.

pub(super) fn version_text(out: &mut String) {
    out.push_str(&format!("quecto {}\n", env!("CARGO_PKG_VERSION")));
}

pub(super) fn help_text(out: &mut String) {
    out.push_str(&format!(
        "quecto - Personal AI Assistant v{}\n",
        env!("CARGO_PKG_VERSION")
    ));
    out.push_str("\nUsage: quecto [command]\n");
    out.push_str("\nWhen run with no arguments, quecto enters the setup and configuration REPL.\n");
    out.push_str("  Use `quecto agent` or `quecto-tui` for agent operation.\n");
    out.push_str("\nGlobal options:\n");
    out.push_str(
        "  --config <path>  Load exactly this config file (default: <base_dir>/config.json\n",
    );
    out.push_str("                   with ./.quecto/config.json merged over it when trusted)\n");
    out.push_str("\nCommands:\n");
    out.push_str("  admission-broker run|status|reset\n");
    out.push_str("              Shared inference admission authority (requires an `admission` config section)\n");
    out.push_str("  agent       Run a one-shot agent session (-m required)\n");
    out.push_str("              Options: -s <name>  Named session (default: \"default\")\n");
    out.push_str("                       --no-session  Ephemeral mode — nothing saved or loaded\n");
    out.push_str("                       --model <m>   Override model\n");
    out.push_str("                       --system <p>  System prompt\n");
    out.push_str("                       --max-iterations <n>  Max tool iterations\n");
    out.push_str("                       --max-time <s>  Wall-clock timeout in seconds\n");
    out.push_str(
        "                       --mode uds    framed JSON agent mode via Unix domain socket\n",
    );
    out.push_str(
        "                       --socket <path>  Socket path for --mode uds (default: auto in tmpdir)\n",
    );
    out.push_str(
        "                       --persist     Keep a top-level UDS agent alive after its last client disconnects; SIGTERM/SIGINT or a protocol shutdown then tears its subagents down over the protocol before it exits (harness-spawned subagents are lifetime-bound to their launcher instead)\n",
    );
    out.push_str(
        "                       --effort <level>  Effort level for 4.6 models (low/medium/high/max)\n",
    );
    out.push_str(
        "                       --disable-tool <name>  Disable/hide a tool and deny re-registration (repeatable)\n",
    );
    out.push_str("  auth        Manage authentication (login, logout, status)\n");
    out.push_str("  config      Read and write configuration\n");
    out.push_str(
        "              get [<dotted.path>] [--effective|--global|--local] [--show-secrets]\n",
    );
    out.push_str(
        "                  (API keys, tokens and passwords print as \"<redacted>\" by default)\n",
    );
    out.push_str(
        "              set <dotted.path> <json-value> [--global|--local]  (default: --local,\n",
    );
    out.push_str("                  the repo-local ./.quecto/config.json overlay)\n");
    out.push_str(
        "              trust [--path <file>]  Approve the repo-local overlay's current content\n",
    );
    out.push_str("  models      Manage runtime model registry (discover)\n");
    out.push_str("  status      Show status\n");
    out.push_str("  help        Show this help\n");
    out.push_str("  version     Show version information\n");
}

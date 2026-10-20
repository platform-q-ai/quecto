//! `quecto help` and `quecto version` text.

pub(in crate::interface::cli) fn version_text(out: &mut String) {
    out.push_str(&format!("quecto {}\n", env!("CARGO_PKG_VERSION")));
}

pub(in crate::interface::cli) fn help_text(out: &mut String) {
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
    out.push_str("  admission-broker run|status|reset|install-service|uninstall-service\n");
    out.push_str("              Shared inference admission authority (requires an `admission` config section)\n");
    out.push_str("              [--config <file>|--directory <dir>] on every action; run [--accept-missing-ledger];\n");
    out.push_str("              install-service|uninstall-service [--dry-run]\n");
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
        "              unset <dotted.path> [--global|--local]  Remove a key (same layers)\n",
    );
    out.push_str(
        "              trust [--path <file>]  Approve the repo-local overlay's current content\n",
    );
    out.push_str("  board       The project board on the repository's quecto/board branch\n");
    out.push_str(
        "              <op> [--<field> <value>]... [--input <json>|-] [--repo <path>] [--remote <name>]\n",
    );
    out.push_str("                  [--json]\n");
    out.push_str(
        "                  ops: bootstrap list list-ready list-takeable show create update-draft\n",
    );
    out.push_str(
        "                  move renew take-over link-pr unlink-pr complete clear-set-aside\n",
    );
    out.push_str(
        "                  prints one JSON answer; a write needs git's user.name and user.email\n",
    );
    out.push_str(
        "                  (GIT_AUTHOR_* and GIT_COMMITTER_* are not honoured). A user.name or\n",
    );
    out.push_str(
        "                  user.email set in git's command scope (GIT_CONFIG_COUNT with\n",
    );
    out.push_str(
        "                  GIT_CONFIG_KEY_<n> and GIT_CONFIG_VALUE_<n>) outranks the clone's, key\n",
    );
    out.push_str(
        "                  by key; the scope's other keys are skipped. A write warns on stderr of\n",
    );
    out.push_str(
        "                  a malformed set, which is dropped, or of one naming half an identity.\n",
    );
    out.push_str(
        "                  The board is cooperative: branch protection or a server hook is the\n",
    );
    out.push_str("                  real control\n");
    out.push_str("  container   Container runtimes for subagents\n");
    out.push_str(
        "              init [--project <abs dir>] [--repo <url>] [--image <tag>] [--refresh] [--dry-run]\n",
    );
    out.push_str(
        "                  Materialise the standard Containerfile and container runtime\n",
    );
    out.push_str(
        "                  scripts under <project>/.quecto/containers/standard and bind the\n",
    );
    out.push_str(
        "                  project to them (container_configs.standard in the trusted overlay)\n",
    );
    out.push_str(
        "              status [--project <abs dir>]  Assets, entry, trust and image standing\n",
    );
    out.push_str(
        "              doctor [--name <config>]  Run the effective container config's create\n",
    );
    out.push_str(
        "                  preflight (runtime, jq, git, gh, image, --repo, state dir) without\n",
    );
    out.push_str("                  creating an environment; exit 1 when any check fails\n");
    out.push_str(
        "              ls [--all] [--json]  Recorded environments (--all: stopped too; --json:\n",
    );
    out.push_str(
        "                  one line {\"environments\":[...],\"hidden_stopped\":N} for scripts)\n",
    );
    out.push_str(
        "              kill <ref|name>  End one environment (exit 4 when there is nothing to kill)\n",
    );
    out.push_str(
        "              kill <ref> --created-by <key>  End it only if its CREATED-BY is exactly <key>\n",
    );
    out.push_str(
        "                  (exit 3, nothing killed, when another key created it; exit 4 when there is nothing to kill)\n",
    );
    out.push_str(
        "              gc [--dry-run] [--name <config>] [--abandoned | --abandoned-after <duration>]\n",
    );
    out.push_str("                  Remove orphaned environments of a container config\n");
    out.push_str("  models      Manage runtime model registry (discover)\n");
    out.push_str("  status      Show status\n");
    out.push_str("  help        Show this help\n");
    out.push_str("  version     Show version information\n");
}

//! `/setup` walkthrough prompts for `quecto-tui` (#2024 S6).
//!
//! `/setup` is a prompt template, not a wizard: the TUI stays policy-free and
//! only composes text. The parent agent reads the `docs` pages the prompt
//! names, checks the current state with its own tools, proposes the runbook
//! commands and asks before it writes. Nothing here touches the filesystem,
//! the UDS protocol or harness policy — pure text plus a tiny argument parser,
//! so `shell::app_submit` needs one match arm.
//!
//! Every prompt is safe by construction: it carries the same affirmative rules
//! the S5 runbooks state (ask before every write/install, never
//! `--show-secrets`, `auth login` always `--token`, dry-run the service
//! install, never `admission-broker run` from a tool call).

#[cfg(test)]
mod tests;

/// Which part of the setup decision tree a `/setup` invocation targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupArea {
    /// Bare `/setup`: the whole decision tree, top to bottom.
    All,
    /// `/setup model <model-id>`: pin this repo's default model.
    Model(String),
    /// `/setup admission`: enable the one host-wide admission broker.
    Admission,
    /// `/setup podman` (alias `container`): this repo's standard container.
    Container,
    /// `/setup auth`: first-install credential.
    Auth,
}

/// What a `/setup …` argument string asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupCommand {
    /// Submit the walkthrough prompt for this area.
    Walkthrough(SetupArea),
    /// Unknown or malformed arguments: show [`SETUP_USAGE`], submit nothing.
    Usage,
}

/// The usage toast for an unknown or malformed `/setup` variant.
pub const SETUP_USAGE: &str =
    "Usage: /setup | /setup model <model-id> | /setup admission | /setup podman | /setup auth";

impl SetupCommand {
    /// Parse the text after `/setup`. Variants are case-sensitive single
    /// words; `model` takes exactly one whitespace-free model id (a
    /// `provider/model` token as `models` lists them) — quotes and backticks
    /// are refused so the id cannot break out of the prompt's own quoting.
    pub fn parse(args: &str) -> Self {
        let mut words = args.split_whitespace();
        let command = match (words.next(), words.next(), words.next()) {
            (None, _, _) => Self::Walkthrough(SetupArea::All),
            (Some("model"), Some(model), None) if model_id_is_plain(model) => {
                Self::Walkthrough(SetupArea::Model(model.to_string()))
            }
            (Some("admission"), None, _) => Self::Walkthrough(SetupArea::Admission),
            (Some("podman") | Some("container"), None, _) => {
                Self::Walkthrough(SetupArea::Container)
            }
            (Some("auth"), None, _) => Self::Walkthrough(SetupArea::Auth),
            _ => Self::Usage,
        };
        command
    }
}

fn model_id_is_plain(model: &str) -> bool {
    !model.is_empty()
        && !model
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '\'' | '`' | '\\'))
}

/// The affirmative rules every walkthrough carries — the same wording the
/// `setup`, `models` and `admission-broker` runbooks state, so the agent sees
/// no contradiction between the prompt and the page it reads next.
const SAFETY_RULES: &str = "\
Rules for this walkthrough: ASK before writing or installing anything and wait for my answer — \
propose the exact command first, run it only after I say yes. Read before you write \
(`quecto status`, `quecto config get --effective`); secrets print as `\"<redacted>\"` — \
never pass `--show-secrets`, never `cat` `credentials.json`, never echo a key I paste. \
`quecto auth login` always with `--token` (without it the browser OAuth flow blocks the call) — \
ask me for the key, do not write it into any file yourself. \
`quecto admission-broker install-service --dry-run` first and show me the plan; \
never run `quecto admission-broker run` from a tool call (it stays in the foreground and dies with the tool). \
One key per `quecto config set`, run the verify command after each, and say which file changed. \
Prefer the repo overlay (`<repo>/.quecto/config.json`); write the global file only when the key is global-only \
(`providers`, `admission`) or I ask for every repo. A pinned model or an enabled broker applies to agents \
started after the change, not to this session — say so instead of claiming this session changed.";

/// Compose the walkthrough prompt the TUI submits as the user's turn.
pub fn setup_walkthrough_prompt(area: &SetupArea) -> String {
    let goal = match area {
        SetupArea::All => "Set up quecto for this folder/machine. First read the docs page `setup` \
(`docs {\"name\":\"setup\"}`) and follow its decision tree top to bottom for THIS situation: \
check first install (`quecto auth status`), this repo's overlay (`quecto status`), the default model, \
the admission broker, and this repo's container config. For each area: report the current state in one line, \
then propose the exact commands from the runbook, and ASK before writing or installing anything. \
Never print secrets. Finish with a summary of what changed and how to roll each change back."
            .to_string(),
        SetupArea::Model(model) => format!(
            "Pin `{model}` as the default model for this repo. First read the docs page `models` \
(`docs {{\"name\":\"models\"}}`) and the \"Pin the default model\" row of the docs page `setup` \
(`docs {{\"name\":\"setup\"}}`). Check the current state (`quecto status`, \
`quecto config get --effective agents.defaults.model`, and whether `{model}` is a model the \
catalogue lists) and report it in one line; then propose the exact `quecto config set \
agents.defaults.model` command for the repo overlay and ASK before writing or installing anything. \
After my yes, run the verify command from the runbook. Never print secrets. Finish with a summary \
of what changed and how to roll it back (`quecto config unset agents.defaults.model`)."
        ),
        SetupArea::Admission => "Enable the admission broker for this host (one broker per host, \
never per repo). First read the docs page `admission-broker` (`docs {\"name\":\"admission-broker\"}`) \
and the \"Enable the admission broker\" row of the docs page `setup` (`docs {\"name\":\"setup\"}`). \
Check the current state (`quecto admission-broker status`, `quecto config get --effective admission`) \
and report it in one line; then propose the exact commands from the runbook — the `quecto config set \
--global admission …` JSON for my providers, then `quecto admission-broker install-service` — and ASK \
before writing or installing anything. Never print secrets. Finish with a summary of what changed, \
which sessions must restart to be bounded, and how to roll it back."
            .to_string(),
        SetupArea::Container => "Set up the standard podman container for this repo. First read the \
docs page `container-runtime` (`docs {\"name\":\"container-runtime\"}`) and the \"A podman/docker \
container for this app\" row of the docs page `setup` (`docs {\"name\":\"setup\"}`). Check the current \
state (`quecto container status`, `quecto container doctor`) and report it in one line; then propose \
the exact commands from the runbook — `quecto container init` (`--dry-run` first), then the `podman \
build …` line it prints — and ASK before writing or installing anything. Never print secrets. Finish \
with a summary of what changed, how to verify it with one `spawn {\"container\":true}` probe, and how \
to roll it back."
            .to_string(),
        SetupArea::Auth => "Set up the credential quecto needs on this machine. First read the docs \
page `models` (`docs {\"name\":\"models\"}`) and the \"First install\" row of the docs page `setup` \
(`docs {\"name\":\"setup\"}`). Check the current state (`quecto auth status`) and report it in one \
line; then propose the exact `quecto auth login --provider <openai|anthropic> --token <key>` command \
and ASK before writing or installing anything — ask me which provider and for the key, and never \
echo the key back. Never print secrets. Finish with a summary of what changed and how to roll it \
back (`quecto auth logout --provider <provider>`)."
            .to_string(),
    };
    format!("{goal}\n\n{SAFETY_RULES}")
}

//! A stand-in `claude` for tests (#2286): CI has no credentials and no test
//! ever runs the real CLI.
//!
//! [`write_mock_claude`] writes a small shell script named `claude`
//! (through [`write_executable`], so it can be exec'd at once from any
//! thread). It replays a stream-json scenario, one turn per user line it
//! reads on stdin, and exits 0 at stdin EOF, as `claude -p
//! --input-format stream-json` does.
//!
//! - `QUECTO_MOCK_CLAUDE_SCRIPT` is the scenario: an NDJSON capture whose
//!   Nth turn is the lines up to and including its Nth `result` line.
//!   Directives, test-only, are never emitted as they are:
//!   - `@stderr <text>` writes `<text>` to stderr and `@stderr-fill <n>`
//!     writes `n` bytes of `x` to stderr, in their turn;
//!   - `@stall-input <secs>` (anywhere) makes the mock sleep that long
//!     before it reads its first input line, so its stdin pipe fills;
//!   - `@stubborn` (anywhere) makes the mock ignore TERM, start a
//!     background `sleep` grandchild (which inherits the ignored TERM) and
//!     keep running after stdin EOF: only KILL to its group ends it.
//! - `QUECTO_MOCK_CLAUDE_ARGS_OUT`, when not empty, is where the mock
//!   writes, at start: its working directory (`cwd=`), pid (`pid=`),
//!   process group (`pgid=`), the `ls -ld` mode of `$HOME` (`home_mode=`)
//!   and of `$CLAUDE_CONFIG_DIR` (`config_mode=`), a stubborn mock's
//!   grandchild pid (`grandchild=`), argv (`arg=`) and environment
//!   (`env=`); then it appends each input line it reads (`input=`).
//!
//! Both are assigned in the script itself: the launcher's environment
//! allowlist drops every `QUECTO_*` variable, so they could not reach the
//! mock through its environment.

use std::path::{Path, PathBuf};

use super::executable::write_executable;

/// The program name the mock is written under.
pub const MOCK_CLAUDE_PROGRAM: &str = "claude";

/// A written mock: the directory to put on `PATH` and its args file.
#[derive(Debug, Clone)]
pub struct MockClaude {
    /// The directory holding the `claude` script.
    pub bin_dir: PathBuf,
    /// Where the mock records its cwd, argv, environment and input.
    pub args_out: PathBuf,
}

impl MockClaude {
    /// The recorded `key=value` lines of one kind (`arg`, `env`, `input`,
    /// `cwd`), in order; empty before the mock has written its record.
    pub fn recorded(&self, kind: &str) -> Vec<String> {
        let prefix = format!("{kind}=");
        std::fs::read_to_string(&self.args_out)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix))
            .map(str::to_string)
            .collect()
    }
}

/// Write the mock into `bin_dir` (created if missing), replaying
/// `scenario`.
pub fn write_mock_claude(bin_dir: &Path, scenario: &Path) -> MockClaude {
    std::fs::create_dir_all(bin_dir)
        .unwrap_or_else(|error| panic!("create {}: {error}", bin_dir.display()));
    let args_out = bin_dir.join("claude.args");
    let script = mock_script(scenario, &args_out);
    write_executable(&bin_dir.join(MOCK_CLAUDE_PROGRAM), script);
    MockClaude {
        bin_dir: bin_dir.to_path_buf(),
        args_out,
    }
}

/// A shell single-quoted literal of `path`.
fn quoted(path: &Path) -> String {
    let text = path
        .to_str()
        .unwrap_or_else(|| panic!("{} is not UTF-8", path.display()));
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn mock_script(scenario: &Path, args_out: &Path) -> String {
    format!(
        r#"#!/bin/sh
# A stand-in for `claude -p` stream-json, written by quecto's test support.
QUECTO_MOCK_CLAUDE_SCRIPT={scenario}
QUECTO_MOCK_CLAUDE_ARGS_OUT={args_out}
stubborn=
grandchild=
if grep -q '^@stubborn$' "$QUECTO_MOCK_CLAUDE_SCRIPT"; then
  stubborn=1
  trap '' TERM
  sleep 3600 &
  grandchild=$!
fi
stall=$(sed -n 's/^@stall-input \([0-9][0-9]*\)$/\1/p' "$QUECTO_MOCK_CLAUDE_SCRIPT" | head -n 1)
if [ -n "$QUECTO_MOCK_CLAUDE_ARGS_OUT" ]; then
  {{
    printf 'cwd=%s\n' "$(pwd)"
    printf 'pid=%s\n' "$$"
    printf 'pgid=%s\n' "$(ps -o pgid= -p $$ | tr -d ' ')"
    printf 'home_mode=%s\n' "$(ls -ld "$HOME" 2>/dev/null | cut -c1-10)"
    printf 'config_mode=%s\n' "$(ls -ld "$CLAUDE_CONFIG_DIR" 2>/dev/null | cut -c1-10)"
    if [ -n "$grandchild" ]; then printf 'grandchild=%s\n' "$grandchild"; fi
    for arg in "$@"; do printf 'arg=%s\n' "$arg"; done
    env | sed 's/^/env=/'
  }} > "$QUECTO_MOCK_CLAUDE_ARGS_OUT.tmp"
  mv "$QUECTO_MOCK_CLAUDE_ARGS_OUT.tmp" "$QUECTO_MOCK_CLAUDE_ARGS_OUT"
fi
if [ -n "$stall" ]; then sleep "$stall"; fi
turn=0
while IFS= read -r line; do
  turn=$((turn + 1))
  if [ -n "$QUECTO_MOCK_CLAUDE_ARGS_OUT" ]; then
    printf 'input=%s\n' "$line" >> "$QUECTO_MOCK_CLAUDE_ARGS_OUT"
  fi
  awk -v want="$turn" '
    BEGIN {{ current = 1; block = ""; for (i = 0; i < 1024; i++) block = block "x" }}
    /^@stubborn$/ || /^@stall-input [0-9]+$/ {{ next }}
    current == want && /^@stderr-fill [0-9]+$/ {{
      n = $2 + 0
      while (n >= 1024) {{ printf "%s", block > "/dev/stderr"; n -= 1024 }}
      if (n > 0) printf "%s", substr(block, 1, n) > "/dev/stderr"
      next
    }}
    current == want && /^@stderr / {{ print substr($0, 9) > "/dev/stderr"; next }}
    current == want {{ print }}
    /"type": *"result"/ {{ current++ }}
  ' "$QUECTO_MOCK_CLAUDE_SCRIPT"
done
if [ -n "$stubborn" ]; then
  while :; do sleep 1; done
fi
exit 0
"#,
        scenario = quoted(scenario),
        args_out = quoted(args_out),
    )
}

#[cfg(test)]
#[path = "mock_claude_tests.rs"]
mod tests;

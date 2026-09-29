//! A stand-in `claude` for tests (#2286): CI has no credentials and no test
//! ever runs the real CLI.
//!
//! [`write_mock_claude`] writes a small shell script named `claude`
//! (through [`write_executable`], so it can be exec'd at once from any
//! thread). It replays a stream-json scenario, one turn per user line it
//! reads on stdin, and exits 0 at stdin EOF, as `claude -p
//! --input-format stream-json` does.
//!
//! It follows claude 2.1.280 (#2287 review round 2):
//! - `@UUID@` in a turn's lines is the `uuid` of the user line that
//!   started it (`user_message_uuid`); `@UUIDS@` is the quoted,
//!   comma-separated `uuid`s of every user line the turn consumed, its own
//!   first and then each folded into it, at most 64 of them
//!   (`user_message_uuids`);
//! - an interrupt line (`"type":"control_request"`) starts no turn: it is
//!   answered at once with a success `control_response`. With
//!   `"cancel_queued":true` the user lines queued behind the running turn
//!   are withdrawn, listed as `cancelled`, and never run; without it they
//!   are listed as `still_queued` and run, together, as the next turn once
//!   the stopped one has ended: a merged batch, whose turn is its LAST
//!   member's (`@UUID@`), and whose `@UUIDS@` keeps the batch's first 64
//!   with, past that, the last member's in slot 63, as claude's collector
//!   does. Idle, it withdraws nothing.
//!
//! - `QUECTO_MOCK_CLAUDE_SCRIPT` is the scenario: an NDJSON capture whose
//!   Nth turn is the lines up to and including its Nth `result` line.
//!   Directives, test-only, are never emitted as they are:
//!   - `@stderr <text>` writes `<text>` to stderr and `@stderr-fill <n>`
//!     writes `n` bytes of `x` to stderr, in their turn;
//!   - `@stall-input <secs>` (anywhere) makes the mock sleep that long
//!     before it reads its first input line, so its stdin pipe fills;
//!   - `@printf <format>` writes `printf '<format>'` to stdout in its
//!     turn: raw bytes (`\377` is not UTF-8) the scenario file cannot
//!     hold as a line of its own; the format must not hold a `'`;
//!   - `@record <text>` appends `record=<text>` to the args file when the
//!     turn reaches it: how far the mock got while its stdout was held;
//!   - `@stubborn` (anywhere) makes the mock ignore TERM, start a
//!     background `sleep` grandchild (which inherits the ignored TERM) and
//!     keep running after stdin EOF: only KILL to its group ends it;
//!   - `@linger <secs>` (anywhere) starts a background `sleep <secs>`
//!     grandchild holding the mock's stdout and stderr open after the mock
//!     itself exits;
//!   - `@stderr-after <secs> <text>` (anywhere) starts a background
//!     grandchild that writes `<text>` to stderr `<secs>` after the start:
//!     last words that arrive after the mock itself may have exited;
//!   - `@await-interrupt` holds its turn there, running, until an
//!     interrupt line comes: user lines read meanwhile queue behind the
//!     turn; the interrupt is answered, then the rest of the turn (its
//!     stopped `result`) is written;
//!   - `@await-steers <n>` (n at most 150) holds its turn there until `n`
//!     more user lines come, which fold into it (`@UUIDS@`), as claude
//!     folds a queued message between tool rounds; an interrupt line
//!     meanwhile is answered and ends the hold.
//! - `QUECTO_MOCK_CLAUDE_ARGS_OUT`, when not empty, is where the mock
//!   writes, at start: its working directory (`cwd=`), pid (`pid=`),
//!   process group (`pgid=`), the `ls -ld` mode of `$HOME` (`home_mode=`)
//!   and of `$CLAUDE_CONFIG_DIR` (`config_mode=`), a stubborn or lingering
//!   mock's grandchild pid (`grandchild=`), argv (`arg=`) and environment
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
linger=$(sed -n 's/^@linger \([0-9][0-9.]*\)$/\1/p' "$QUECTO_MOCK_CLAUDE_SCRIPT" | head -n 1)
if [ -n "$linger" ]; then
  sleep "$linger" &
  grandchild=$!
fi
late=$(sed -n 's/^@stderr-after \([0-9][0-9.]*\) .*$/\1/p' "$QUECTO_MOCK_CLAUDE_SCRIPT" | head -n 1)
if [ -n "$late" ]; then
  late_words=$(sed -n 's/^@stderr-after [0-9][0-9.]* \(.*\)$/\1/p' "$QUECTO_MOCK_CLAUDE_SCRIPT" | head -n 1)
  (sleep "$late"; printf '%s\n' "$late_words" >&2) &
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
recorded() {{
  if [ -n "$QUECTO_MOCK_CLAUDE_ARGS_OUT" ]; then
    printf 'input=%s\n' "$1" >> "$QUECTO_MOCK_CLAUDE_ARGS_OUT"
  fi
}}
uuid_of() {{
  printf '%s' "$1" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p'
}}
# The quoted, comma-separated JSON list of the words of $1, at most 64.
json_list() {{
  for word in $1; do printf '%s\n' "$word"; done | head -n 64 | awk 'NF {{ printf "%s\"%s\"", (n++ ? "," : ""), $0 }}'
}}
# The uuids a merged batch $1 names, as claude's collector keeps them: the
# first 64, and past that the batch's own (its last member's) in slot 63.
batch_of() {{
  for word in $1; do printf '%s\n' "$word"; done | awk 'NF {{ w[++n] = $0 }} END {{ for (i = 1; i <= n && i <= 63; i++) print w[i]; if (n >= 64) print w[n] }}'
}}
answer_control() {{
  rid=$(printf '%s' "$1" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$1" in
    *'"cancel_queued":true'*) still=; cancelled=$(json_list "$queued"); queued= ;;
    *) still=$(json_list "$queued"); cancelled= ;;
  esac
  printf '{{"type":"control_response","response":{{"subtype":"success","request_id":"%s","response":{{"still_queued":[%s],"cancelled":[%s]}}}}}}\n' "$rid" "$still" "$cancelled"
}}
# Hold the running turn for an interrupt ($1 = interrupt) or for $1 user
# lines to fold into it; other user lines queue behind it.
hold() {{
  want=$1
  while IFS= read -r held; do
    recorded "$held"
    case "$held" in
      *'"type":"control_request"'*) answer_control "$held"; return 0 ;;
    esac
    if [ "$want" = interrupt ]; then
      queued="$queued $(uuid_of "$held")"
    else
      uuids="$uuids $(uuid_of "$held")"
      want=$((want - 1))
      if [ "$want" -le 0 ]; then return 0; fi
    fi
  done
  return 1
}}
replay() {{
  awk -v want="$turn" -v part="$1" -v uuid="$uuid" -v uuids="$(json_list "$uuids")" -v record="$QUECTO_MOCK_CLAUDE_ARGS_OUT" '
    BEGIN {{ current = 1; after = 0; block = ""; for (i = 0; i < 1024; i++) block = block "x" }}
    /^@stubborn$/ || /^@stall-input [0-9]+$/ || /^@linger / || /^@stderr-after / {{ next }}
    current == want && /^@await-interrupt$/ {{ if (part == 1) {{ fflush(); exit 3 }} after = 1; next }}
    current == want && /^@await-steers [0-9]+$/ {{ if (part == 1) {{ fflush(); exit 100 + $2 }} after = 1; next }}
    current == want && part == 2 && !after {{ next }}
    current == want && /^@stderr-fill [0-9]+$/ {{
      n = $2 + 0
      while (n >= 1024) {{ printf "%s", block > "/dev/stderr"; n -= 1024 }}
      if (n > 0) printf "%s", substr(block, 1, n) > "/dev/stderr"
      next
    }}
    current == want && /^@stderr / {{ print substr($0, 9) > "/dev/stderr"; next }}
    current == want && /^@printf / {{ fflush(); system("printf \047" substr($0, 9) "\047"); next }}
    current == want && /^@record / {{
      fflush()
      if (record != "") {{ print "record=" substr($0, 9) >> record; close(record) }}
      next
    }}
    current == want {{ gsub(/@UUIDS@/, uuids); gsub(/@UUID@/, uuid); print }}
    /"type": *"result"/ {{ current++ }}
  ' "$QUECTO_MOCK_CLAUDE_SCRIPT"
}}
# Run turn $turn, started by $uuid, consuming $uuids.
run_turn() {{
  replay 1
  code=$?
  if [ $code -eq 3 ]; then
    hold interrupt
    replay 2
  elif [ $code -gt 100 ]; then
    hold $((code - 100))
    replay 2
  fi
}}
turn=0
queued=
uuids=
while IFS= read -r line; do
  recorded "$line"
  case "$line" in
    *'"type":"control_request"'*) answer_control "$line"; continue ;;
  esac
  turn=$((turn + 1))
  uuid=$(uuid_of "$line")
  uuids=$uuid
  run_turn
  # What an interrupt left queued runs, together, as the next turn.
  while [ -n "$queued" ]; do
    turn=$((turn + 1))
    uuids=$(batch_of "$queued")
    queued=
    uuid=$(for word in $uuids; do printf '%s\n' "$word"; done | tail -n 1)
    run_turn
  done
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

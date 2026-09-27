#!/usr/bin/env bash
# Summarise one or more cargo-mutants output directories as Markdown.
#
# Used by .github/workflows/mutation.yml: each shard summarises its own
# `mutants.out`, and the "Mutation Testing (diff)" job summarises every shard's
# together. Each directory is a `mutants.out` (or a downloaded copy of one)
# holding cargo-mutants' per-outcome lists: caught.txt, missed.txt,
# timeout.txt and unviable.txt, one mutant per line as
# `<file>:<line>:<col>: <mutation>`.
#
# Usage:
#   scripts/mutants-summary.sh [--fail-on-uncaught] <title> <mutants.out dir>...
#
# Prints the Markdown to stdout. With --fail-on-uncaught, exits 1 when any
# mutant was MISSED or TIMEOUT (after printing): a mutant that makes the tests
# hang is not caught either. Exits 2 on a usage error or when a
# directory is not a cargo-mutants output directory.
set -euo pipefail

usage() {
  echo "usage: $0 [--fail-on-uncaught] <title> <mutants.out dir>..." >&2
  exit 2
}

FAIL_ON_UNCAUGHT=0
if [[ $# -ge 1 && "$1" == "--fail-on-uncaught" ]]; then
  FAIL_ON_UNCAUGHT=1
  shift
fi
if [[ $# -lt 2 ]]; then
  usage
fi

TITLE="$1"
shift
DIRS=("$@")

for dir in "${DIRS[@]}"; do
  # cargo-mutants writes outcomes.json before any mutant runs; its presence is
  # what makes a directory a mutants.out.
  if [[ -d "$dir" && -f "$dir/outcomes.json" ]]; then
    continue
  fi
  echo "error: not a cargo-mutants output directory (no outcomes.json): $dir" >&2
  exit 2
done

# All lines of one outcome list across every directory, sorted and unique.
collect() {
  local outcome="$1"
  local dir
  for dir in "${DIRS[@]}"; do
    if [[ -f "$dir/$outcome.txt" ]]; then
      # awk 1 ends a last line that lacks a newline, so files join cleanly.
      awk 1 "$dir/$outcome.txt"
    fi
  done | sed '/^[[:space:]]*$/d' | LC_ALL=C sort -u
}

count_lines() {
  if [[ -z "$1" ]]; then
    echo 0
  else
    printf '%s\n' "$1" | wc -l | tr -d ' '
  fi
}

CAUGHT="$(collect caught)"
MISSED="$(collect missed)"
TIMEOUT="$(collect timeout)"
UNVIABLE="$(collect unviable)"

N_CAUGHT="$(count_lines "$CAUGHT")"
N_MISSED="$(count_lines "$MISSED")"
N_TIMEOUT="$(count_lines "$TIMEOUT")"
N_UNVIABLE="$(count_lines "$UNVIABLE")"

# A backtick inside a mutation name would end the inline code span early.
list_items() {
  local bt='`'
  printf '%s\n' "$1" | sed "s/${bt}/'/g; s/^/- ${bt}/; s/\$/${bt}/"
}

echo "## $TITLE"
echo
echo "| Outcome | Mutants |"
echo "|---|---|"
echo "| Caught | $N_CAUGHT |"
echo "| **Missed** | $N_MISSED |"
echo "| Timeout | $N_TIMEOUT |"
echo "| Unviable | $N_UNVIABLE |"
echo
if [[ "$N_MISSED" -gt 0 ]]; then
  echo "### Missed ($N_MISSED)"
  echo
  echo "No test failed with these mutations applied: the changed lines are not pinned by a test."
  echo
  echo "Only the libtest targets run here (unit tests, \`contracts\`, \`integration\`). The cucumber BDD suites, the real-process targets (\`parent_loss\`, \`selected_termination\`) and the docs and architecture suites do not, so code that only they pin shows as MISSED. For a MISSED line (common in \`application/**/use_cases\`, which BDD scenarios drive), look for a scenario that asserts the changed behaviour: if none does, add a test that fails with the mutation; if one does, prefer a unit or contract test at the use case's boundary too, or say in the PR why the BDD scenario is enough."
  echo
  list_items "$MISSED"
  echo
fi
if [[ "$N_TIMEOUT" -gt 0 ]]; then
  echo "### Timeout ($N_TIMEOUT)"
  echo
  echo "The tests ran past the per-mutant timeout: the mutation made them hang or crawl (often a loop bound, retry or wait). This fails the check like a MISSED mutant: make the tests fail fast under such a change (a bounded wait, an iteration cap asserted by a test)."
  echo
  list_items "$TIMEOUT"
  echo
fi
if [[ "$N_MISSED" -eq 0 && "$N_TIMEOUT" -eq 0 ]]; then
  echo "Every viable mutant tested was caught."
fi

# Pass only when nothing was missed and nothing timed out.
if [[ "$FAIL_ON_UNCAUGHT" -eq 0 ]] || [[ "$N_MISSED" -eq 0 && "$N_TIMEOUT" -eq 0 ]]; then
  exit 0
fi
exit 1

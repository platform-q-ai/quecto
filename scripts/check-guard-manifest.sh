#!/usr/bin/env bash
# Guard-removal manifest for the #1221 conversation history/recovery slice.
#
# WHY THIS EXISTS
#
# Five adversarial review rounds each found the same defect: a test that passes
# for a reason other than the one it is named for. A negative test can pass
# because its guard works, or because its fixture never reaches that guard —
# and nothing in a green suite distinguishes the two.
#
# Rounds 3 and 4 adopted the rule "every negative test must fail when the guard
# it names is removed", but recorded the result as PROSE ("all 24 mutations
# killed their named test"). That claim was unauditable precisely because it
# enumerated nothing, and it was wrong: `HistoryPaging::reset` and
# `reopen_backfill` had never been mutated at all, and three of their guards
# were pinned only by vacuous assertions whose fixtures left each field already
# in its post-condition state.
#
# This script replaces the prose with an executable enumeration. Each entry
# names one guard; deleting that line MUST fail at least one test. A guard that
# survives deletion is unpinned, regardless of how many tests appear to cover
# it. Running this immediately found a ninth unpinned guard that four rounds of
# manual review had missed.
#
# Add an entry whenever a guard is added to these files. An unlisted guard is
# an unverified guard.
#
# Usage: scripts/check-guard-manifest.sh
# Exit 0 = every listed guard is pinned; 1 = at least one survives deletion.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

HP=quecto-tui/src/conversation/history_paging.rs
TR=quecto-tui/src/conversation/turn_recovery.rs
RA=quecto-tui/src/protocol/range_accumulator.rs

HP_BAK=$(mktemp)
TR_BAK=$(mktemp)
RA_BAK=$(mktemp)
cp "$HP" "$HP_BAK"
cp "$TR" "$TR_BAK"
cp "$RA" "$RA_BAK"

restore() {
  cp "$HP_BAK" "$HP"
  cp "$TR_BAK" "$TR"
  cp "$RA_BAK" "$RA"
  rm -f "$HP_BAK" "$TR_BAK" "$RA_BAK"
}
trap restore EXIT INT TERM

fail=0

# Delete the line matching an exact literal within a file, then confirm the
# suite goes red. Matching by CONTENT rather than line number keeps the
# manifest valid as the files move.
check() {
  local file=$1 bak=$2 pattern=$3 occurrence=$4 desc=$5

  local line
  line=$(grep -n -F -- "$pattern" "$file" | sed -n "${occurrence}p" | cut -d: -f1)
  if [ -z "$line" ]; then
    echo "  MISSING   $desc -- guard not found; manifest is stale"
    fail=1
    return
  fi

  # Mutations are applied by sed, so deleting one line of a multi-line
  # expression can leave invalid syntax. A build failure is NOT evidence that a
  # guard is pinned, so it must be reported distinctly rather than counted as a
  # kill -- and never as a survivor, which is what a naive grep for "FAILED"
  # does. Use the replacement form for guards that span lines.
  sed -i "${line}d" "$file"
  local raw
  raw=$(cargo test -p quecto-tui --lib 2>&1)
  cp "$bak" "$file"

  if ! echo "$raw" | grep -qE "^test result"; then
    echo "  BUILD-FAIL  $desc -- deletion left invalid syntax; rewrite this entry"
    fail=1
  elif echo "$raw" | grep -qE "^test result.*FAILED"; then
    echo "  pinned    $desc"
  else
    echo "  SURVIVES  $desc  <-- unpinned guard, add a test that dies without it"
    fail=1
  fi
}

# Some guards are one operand of a multi-line boolean, where deleting a line
# breaks the parse. Replace the whole expression with an equivalent that omits
# just that operand.
check_replace() {
  local file=$1 bak=$2 before=$3 after=$4 desc=$5

  if ! grep -qF -- "$before" "$file"; then
    echo "  MISSING   $desc -- expression not found; manifest is stale"
    fail=1
    return
  fi

  python3 - "$file" "$before" "$after" <<'PYEOF'
import sys
path, before, after = sys.argv[1], sys.argv[2], sys.argv[3]
with open(path) as fh:
    text = fh.read()
with open(path, "w") as fh:
    fh.write(text.replace(before, after, 1))
PYEOF

  local raw
  raw=$(cargo test -p quecto-tui --lib 2>&1)
  cp "$bak" "$file"

  if ! echo "$raw" | grep -qE "^test result"; then
    echo "  BUILD-FAIL  $desc -- replacement left invalid syntax"
    fail=1
  elif echo "$raw" | grep -qE "^test result.*FAILED"; then
    echo "  pinned    $desc"
  else
    echo "  SURVIVES  $desc  <-- unpinned guard, add a test that dies without it"
    fail=1
  fi
}

echo "== conversation/history_paging.rs =="
check "$HP" "$HP_BAK" "self.pending_page = None;"        3 "reset: forgets the in-flight page"
check "$HP" "$HP_BAK" "self.before_cursor = None;"       1 "reset: drops the paging cursor"
check "$HP" "$HP_BAK" "self.has_more_before = false;"    1 "reset: drops the advertised-more flag"
check "$HP" "$HP_BAK" "self.partial_prefix_len = None;"  1 "reset: drops the partial prefix"
check "$HP" "$HP_BAK" "self.backfilled = false;"         1 "reset: unlatches the backfill guard"
check "$HP" "$HP_BAK" "self.backfilled = false;"         2 "reopen_backfill: unlatches the guard"
check "$HP" "$HP_BAK" "self.partial_prefix_len = None;"  2 "reopen_backfill: drops the stale prefix"
check "$HP" "$HP_BAK" "self.backfilled = false;"         3 "reconcile keep-open arm: unlatches"
check "$HP" "$HP_BAK" "self.partial_prefix_len = None;"  3 "reconcile latch arm: clears the prefix"

echo "== conversation/turn_recovery.rs =="
check_replace "$TR" "$TR_BAK" \
  "!refs.is_empty() && open_tool_calls > 0" \
  "open_tool_calls > 0" \
  "forced_without_text: empty refs cannot force recovery"
check_replace "$TR" "$TR_BAK" \
  "!refs.is_empty() && open_tool_calls > 0" \
  "!refs.is_empty()" \
  "forced_without_text: open tool calls force recovery"
check_replace "$TR" "$TR_BAK" \
  "        if self.refs.is_empty() {
            return false;
        }
" \
  "" \
  "needs_recovery: empty refs do not recover"
check_replace "$TR" "$TR_BAK" \
  "        if self.open_tool_calls > 0 {
            return true;
        }
" \
  "" \
  "needs_recovery: open tool calls force recovery"
check_replace "$TR" "$TR_BAK" \
  "if trimmed.is_empty() || trimmed == \"…\" || trimmed == \"...\" {" \
  "if false {" \
  "needs_recovery: placeholder text recovers"
check "$TR" "$TR_BAK" "&& (self.assistant_text.len() as u64) < expected" 1 "needs_recovery: truncated content recovers"
check_replace "$TR" "$TR_BAK" \
  "self.refs.len() != expected_refs" \
  "false" \
  "needs_recovery: ref-count mismatch recovers"
check_replace "$TR" "$TR_BAK" \
  "refs.iter().filter_map(|r| responses.get(r))" \
  "responses.values()" \
  "ordered_by_refs: walks refs order"
check_replace "$TR" "$TR_BAK" \
  "self.responses.len() == self.refs.len()" \
  "self.responses.len() != self.refs.len()" \
  "recovery batch: complete only when every ref responded"

echo "== protocol/range_accumulator.rs =="
check_replace "$RA" "$RA_BAK" \
  "if next_offset <= response_offset
                || next_offset > content_len" \
  "if next_offset > content_len" \
  "range: rejects a non-progressing cursor"
check "$RA" "$RA_BAK" "|| next_offset > content_len"         1 "range: rejects a cursor past the end"
check "$RA" "$RA_BAK" "|| self.content.len() > content_len"  1 "range: rejects an overshoot"

echo
if [ $fail -eq 0 ]; then
  echo "MANIFEST: all guards pinned"
else
  echo "MANIFEST: UNPINNED GUARDS FOUND"
fi
exit $fail

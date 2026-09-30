#!/usr/bin/env bash
# check-golden-manifest.sh — the swarm board's golden fixtures are frozen
# (#2283, #2344 final review): against the base, the MANIFEST only shrinks.
# No line may change or be added (a fixture is never edited, re-recorded or
# new: the Python board that recorded them is gone), and GOLDEN_CEILING may
# only go down. A base without the MANIFEST (the change introducing it)
# passes.
#
# Usage: scripts/check-golden-manifest.sh [<base>]
#   <base> defaults to the merge base of HEAD and origin/master (master).
set -euo pipefail

MANIFEST=quecto-agentic-harness/tests/fixtures/swarm_board/golden/MANIFEST
GOLDENS=quecto-agentic-harness/tests/integration/swarm_board_goldens.rs

if [[ $# -gt 1 ]]; then
    echo "usage: $0 [<base>]" >&2
    exit 2
fi
if [[ $# -eq 1 ]]; then
    BASE="$(git rev-parse --verify "$1^{commit}")"
else
    upstream=origin/master
    git rev-parse --verify -q "$upstream" >/dev/null || upstream=master
    BASE="$(git merge-base HEAD "$upstream")"
fi

if ! git cat-file -e "${BASE}:${MANIFEST}" 2>/dev/null; then
    echo "golden manifest: none at the base ${BASE}; nothing to shrink from"
    exit 0
fi

ceiling() {
    sed -n 's/^const GOLDEN_CEILING: usize = \([0-9][0-9]*\);$/\1/p' | head -n1
}

FAILED=0
# Every line of the MANIFEST here is a line of the base's, verbatim.
mapfile -t ADDED < <(comm -13 <(git show "${BASE}:${MANIFEST}" | LC_ALL=C sort) \
    <(LC_ALL=C sort "$MANIFEST"))
if [[ ${#ADDED[@]} -gt 0 ]]; then
    echo "golden manifest: lines not in the base's MANIFEST (edited or new fixtures):" >&2
    printf '  %s\n' "${ADDED[@]}" >&2
    FAILED=1
fi
BASE_CEILING="$(git show "${BASE}:${GOLDENS}" | ceiling)"
HEAD_CEILING="$(ceiling <"$GOLDENS")"
if [[ -z "$BASE_CEILING" || -z "$HEAD_CEILING" ]]; then
    echo "golden manifest: GOLDEN_CEILING not found (base '${BASE_CEILING}', here '${HEAD_CEILING}')" >&2
    FAILED=1
elif (( HEAD_CEILING > BASE_CEILING )); then
    echo "golden manifest: GOLDEN_CEILING rose from ${BASE_CEILING} to ${HEAD_CEILING}" >&2
    FAILED=1
fi
if (( FAILED != 0 )); then
    exit 1
fi
echo "golden manifest: shrinks only against ${BASE} (ceiling ${HEAD_CEILING})"

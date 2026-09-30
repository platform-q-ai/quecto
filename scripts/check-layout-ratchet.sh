#!/usr/bin/env bash
# Budget rows may only shrink against the merge base (#2358 amendment 9).
# Usage: scripts/check-layout-ratchet.sh [<base>]
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
if [[ $# -le 1 ]]; then
    if [[ $# -eq 1 ]]; then
        BASE="$(git rev-parse --verify "$1^{commit}")"
    else
        upstream=origin/master
        git rev-parse --verify -q "$upstream" >/dev/null || upstream=master
        BASE="$(git merge-base HEAD "$upstream")"
    fi
else
    echo "usage: $0 [<base>]" >&2
    exit 2
fi

TABLE=quecto-agentic-harness/tests/architecture/layout.rs
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/layout-ratchet.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT

# Read literal policy rows, not a second authoritative budget table.
budgets() {
    perl -0777 -e '
        use strict; use warnings;
        my $source = <>;
        my @definitions = $source =~ /\bconst\s+BUDGETS\b/g;
        @definitions == 1 or die "BUDGETS: expected exactly one definition, including comments; remove ambiguity\n";
        $source =~ /const\s+BUDGETS\s*:[^=]+?=\s*&\[(.*?)\];/s
            or die "$ARGV: BUDGETS missing; restore the budget table\n";
        my $body = $1;
        $body =~ s{//[^\n]*}{}g;
        while ($body =~ s/^\s*FlatBudget\s*\{\s*path:\s*"([^"]+)"\s*,\s*expected:\s*(\d+)\s*,?\s*\}\s*,?//s) {
            print "$1\t", 0 + $2, "\n";
        }
        $body =~ /^\s*$/s
            or die "BUDGETS: cannot read literal path/count rows; restore the table syntax\n";
    ' "$1"
}

if budgets "$TABLE" >"$SCRATCH/head"; then :; else exit 1; fi
printf '%s\n' "$TABLE" >"$SCRATCH/expected-entry"
if timeout -s KILL 10 git ls-tree --name-only "$BASE" -- "$TABLE" >"$SCRATCH/entries"; then
    if [[ "$(wc -c <"$SCRATCH/entries")" -eq 0 ]]; then
        echo "layout ratchet: no table at base $BASE; initial introduction allowed"
        exit 0
    elif cmp -s "$SCRATCH/entries" "$SCRATCH/expected-entry"; then
        timeout -s KILL 10 git show "$BASE:$TABLE" >"$SCRATCH/base-source"
        if budgets "$SCRATCH/base-source" >"$SCRATCH/base"; then :; else exit 1; fi
    else
        echo "layout ratchet: $TABLE: unexpected git ls-tree output; inspect the base tree" >&2
        exit 1
    fi
else
    echo "layout ratchet: $TABLE: git ls-tree failed; repair Git access to base $BASE" >&2
    exit 1
fi

declare -A OLD=()
while IFS=$'\t' read -r path count; do
    OLD["$path"]="$count"
done <"$SCRATCH/base"
FAILED=0
while IFS=$'\t' read -r path count; do
    if [[ -v "OLD[$path]" ]] && (( count <= OLD["$path"] )); then
        unset 'OLD[$path]'
        continue
    fi
    echo "layout ratchet: $TABLE: $path budget $count is new or raised; remove the row or lower it to the merge-base budget" >&2
    FAILED=1
done <"$SCRATCH/head"
for path in "${!OLD[@]}"; do
    if [[ -d "quecto-agentic-harness/src/$path" || -e "quecto-agentic-harness/src/$path" || -L "quecto-agentic-harness/src/$path" ]]; then
        echo "layout ratchet: $TABLE: $path still exists; restore its budget row or retire the directory" >&2
        FAILED=1
    fi
done
if (( FAILED == 0 )); then
    echo "layout ratchet: budgets only shrink against $BASE"
else
    exit 1
fi

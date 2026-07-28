#!/usr/bin/env bash
# check-bdd-tags.sh — Status-tag hygiene gate for BDD feature files.
#
# The BDD runners only run a scenario in the authoritative CI BDD wave when it (or
# its feature) is tagged @done. Scenarios tagged @wip / @pending are deliberately
# excluded, and — critically — an UNTAGGED scenario is silently dropped too: it
# never runs in the gate AND is not marked as backlog, so broken/unwired
# scenarios can rot invisibly.
#
# This gate fails if any scenario lacks a status tag (@done / @wip / @pending)
# on the scenario or its feature. It forces every scenario into exactly one
# visible state: gated (@done), authoring (@wip), or backlog (@pending).
#
# Exit 0 = pass, Exit 1 = one or more untagged scenarios found.
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

ROOT="$(git rev-parse --show-toplevel)"
if [[ -n "${BDD_FEATURES_DIR:-}" ]]; then
    FEATURES_DIRS=("$BDD_FEATURES_DIR")
else
    FEATURES_DIRS=(
        "$ROOT/quecto-agentic-harness/tests/features"
        "$ROOT/quecto-tui/tests/features"
    )
fi

for dir in "${FEATURES_DIRS[@]}"; do
    if [ ! -d "$dir" ]; then
        echo -e "${RED}FAIL${NC}: features dir not found: $dir"
        exit 1
    fi
done

echo "=== BDD Status-Tag Gate ==="
echo ""

# For each feature file, walk scenarios and compute effective tags
# (feature-level tags ∪ scenario-level tags). A status tag is one of
# @done / @wip / @pending. Emit "file:line\tscenario" for each violation.
VIOLATIONS="$({
    for dir in "${FEATURES_DIRS[@]}"; do
        for f in "$dir"/*.feature; do
            [ -e "$f" ] || continue
            awk -v file="$f" '
                function has_status(tags) {
                    return (tags ~ /(^| )@done( |$)/ ||
                            tags ~ /(^| )@wip( |$)/  ||
                            tags ~ /(^| )@pending( |$)/)
                }
                # Accumulate tag lines into pending_tags until they bind to a
                # Feature: or Scenario: line.
                /^[[:space:]]*@/ {
                    line=$0
                    gsub(/^[[:space:]]+|[[:space:]]+$/, "", line)
                    pending_tags = (pending_tags == "" ? line : pending_tags " " line)
                    next
                }
                /^[[:space:]]*Feature:/ {
                    feature_tags = pending_tags
                    pending_tags = ""
                    next
                }
                /^[[:space:]]*Scenario:|^[[:space:]]*Scenario Outline:/ {
                    eff = feature_tags " " pending_tags
                    if (!has_status(eff)) {
                        name = $0
                        sub(/^[[:space:]]*Scenario( Outline)?:[[:space:]]*/, "", name)
                        printf "%s:%d\t%s\n", file, NR, name
                    }
                    pending_tags = ""
                    next
                }
                # Any other non-blank line severs a dangling tag block (defensive;
                # valid Gherkin puts tags immediately above the scenario/feature).
                /[^[:space:]]/ { pending_tags = "" }
            ' "$f"
        done
    done
})"

if [ -n "$VIOLATIONS" ]; then
    COUNT="$(printf '%s\n' "$VIOLATIONS" | grep -c .)"
    echo -e "${RED}FAIL${NC}: $COUNT scenario(s) have no status tag (@done / @wip / @pending)."
    echo "        An untagged scenario never runs in the gate and is not marked as backlog."
    echo "        Add @done (gated), @wip (authoring), or @pending (backlog) to each:"
    echo ""
    printf '%s\n' "$VIOLATIONS" | sed 's|'"$ROOT"'/||' | sed 's/^/  /'
    echo ""
    exit 1
fi

echo -e "${GREEN}PASS${NC}: every scenario carries a status tag (@done / @wip / @pending)."

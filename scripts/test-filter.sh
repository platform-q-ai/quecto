#!/usr/bin/env bash
# Filter cargo test / cucumber-rs output to show only:
#   - Summary line (running N tests / test result)
#   - All failure details (stdout, panic messages, assertion errors)
#   - Cucumber summary + failed scenarios with their Feature/Scenario context
#
# Usage:
#   cargo test --no-fail-fast --lib 2>&1 | scripts/test-filter.sh
#   cargo test --no-fail-fast --test bdd 2>&1 | scripts/test-filter.sh

awk '
# ── Standard cargo test harness ──────────────────────────────────
/^running [0-9]+ tests/   { print; next }
/^test result:/            { print; next }
/^failures:/               { in_failures=1 }
in_failures                { print; next }
/^error:/                  { print; next }

# ── Cucumber-rs BDD harness ──────────────────────────────────────
# Track current Feature and Scenario so we can print context on failure
/^Feature:/                { feature=$0; scenario="" }
/^  Scenario:/             { scenario=$0 }

# On a failed step, print the Feature + Scenario context first
/✘/ {
    if (feature != "" && feature != last_printed_feature) {
        print feature
        last_printed_feature = feature
        last_printed_scenario = ""
    }
    if (scenario != "" && scenario != last_printed_scenario) {
        print scenario
        last_printed_scenario = scenario
    }
    print
    next
}

# Capture block after failure (e.g. assertion details)
/^   Captured/             { in_capture=1 }
in_capture && /^$/         { in_capture=0; print; next }
in_capture                 { print; next }

# Summary block
/\[Summary\]/              { in_summary=1 }
in_summary                 { print; next }
'

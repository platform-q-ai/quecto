#!/usr/bin/env bash
# check-bdd-quality.sh — Static analysis for BDD step definitions.
#
# Detects anti-patterns in harness BDD step definitions:
#   1. Then steps with no assertions (always-pass tests)
#   2. Empty/no-op When steps (stub implementations)
#   3. Tautological assertions (assert!(true), assert_eq!(x, x))
#   4. Placeholder macros in test code (todo!(), unimplemented!(), panic!("not implemented"))
#   5. TODO/FIXME/HACK/STUB comments in test code
#   6. Steps that discard results silently (let _ = ...block_on)
#   7. Steps that reimplement logic instead of calling production code
#
# Limitations:
#   - Tautology detection (assert_eq!(x, x)) uses \w+ so only catches simple
#     identifiers, not dotted paths or indexed expressions.
#   - The When-assert-only check uses a simple regex to strip assert calls,
#     which cannot handle nested parentheses in assert arguments.
#
# Exit 0 = pass, Exit 1 = one or more violations found.
set -euo pipefail

RED='\033[0;31m'
YELLOW='\033[0;33m'
GREEN='\033[0;32m'
NC='\033[0m'

# Allow override via env var for multi-file repos or testing.
# Supports both single-file and multi-file BDD layouts.
BDD_DIR="${BDD_DIR:-quecto-agentic-harness/tests/bdd}"

if [ -d "$BDD_DIR" ]; then
    BDD_FILE=$(mktemp)
    cat "$BDD_DIR"/*.rs > "$BDD_FILE"
    trap "rm -f '$BDD_FILE'" EXIT
elif [ -f "${BDD_FILE:-quecto-agentic-harness/tests/bdd.rs}" ]; then
    BDD_FILE="${BDD_FILE:-quecto-agentic-harness/tests/bdd.rs}"
else
    echo -e "${RED}FAIL${NC}: neither $BDD_DIR/ directory nor quecto-agentic-harness/tests/bdd.rs found"
    exit 1
fi

FAILED=0
WARNED=0

fail() {
    local description="$1"
    shift
    if [ -n "$*" ]; then
        echo -e "${RED}FAIL${NC}: $description"
        echo "$*"
        echo ""
        FAILED=1
    fi
}

warn() {
    local description="$1"
    shift
    if [ -n "$*" ]; then
        echo -e "${YELLOW}WARN${NC}: $description"
        echo "$*"
        echo ""
        WARNED=1
    fi
}

echo "=== BDD Quality Gate ==="
echo ""

# ───────────────────────────────────────────────────────────────
# 1. Tautological assertions — assert!(true), assert!(false == false), etc.
#    Note: \w+ only catches simple identifiers, not foo.bar or v[0].
# ───────────────────────────────────────────────────────────────
matches=$(grep -nP 'assert!\(\s*true\s*\)' "$BDD_FILE" || true)
fail "assert!(true) — tautological assertion (always passes)" "$matches"

matches=$(grep -nP 'assert_eq!\(\s*(\w+)\s*,\s*\1\s*[,)]' "$BDD_FILE" || true)
fail "assert_eq!(x, x) — tautological assertion (always passes)" "$matches"

matches=$(grep -nP 'assert_ne!\(\s*(\w+)\s*,\s*\1\s*[,)]' "$BDD_FILE" || true)
fail "assert_ne!(x, x) — tautological assertion (always fails)" "$matches"

matches=$(grep -nP 'assert!\(\s*!false\s*\)' "$BDD_FILE" || true)
fail "assert!(!false) — tautological assertion" "$matches"

# ───────────────────────────────────────────────────────────────
# 2. Placeholder macros — these should never appear in step code.
# ───────────────────────────────────────────────────────────────
matches=$(grep -n 'todo!()' "$BDD_FILE" || true)
fail "todo!() in step definitions (implement before committing)" "$matches"

matches=$(grep -n 'unimplemented!()' "$BDD_FILE" || true)
fail "unimplemented!() in step definitions" "$matches"

matches=$(grep -nP 'panic!\(\s*"not implemented' "$BDD_FILE" || true)
fail 'panic!("not implemented...") in step definitions' "$matches"

# ───────────────────────────────────────────────────────────────
# 3. TODO/FIXME/HACK/STUB comments in test code.
# ───────────────────────────────────────────────────────────────
matches=$(grep -nP '//\s*(TODO|FIXME|HACK|STUB)\b' "$BDD_FILE" || true)
fail "TODO/FIXME/HACK/STUB comments in step definitions (resolve before committing)" "$matches"

# ───────────────────────────────────────────────────────────────
# 4. Discarded async results — `let _ = ...block_on(...)` silently
#    swallows errors that should cause step failure.
# ───────────────────────────────────────────────────────────────
# Single-line: let _ = ...block_on(...)
matches=$(grep -n 'let _ = .*block_on' "$BDD_FILE" || true)
# Multi-line: let _ = tokio::runtime::Runtime::new()\n  .unwrap()\n  .block_on(...)
# Detect `let _ =` followed by block_on within 3 lines
matches2=$(awk '
    /let _ =/ { start = NR; pending = $0; next }
    start && NR <= start + 3 && /\.block_on\(/ {
        printf "%d: %s ... %s\n", start, pending, $0
        start = 0; pending = ""
        next
    }
    start && NR > start + 3 { start = 0; pending = "" }
' "$BDD_FILE" || true)
combined_discards=$(printf "%s\n%s" "$matches" "$matches2" | sed '/^$/d')
fail "Discarded block_on result (errors swallowed silently, use .unwrap() or ?)" "$combined_discards"

# ───────────────────────────────────────────────────────────────
# 5-7. Structural step-body analysis (single awk pass).
#
# Extracts #[then] and #[when] function bodies using brace-depth
# tracking, then checks:
#   5. Then steps with no assertions (hard fail)
#   6. Empty When steps / no-op stubs (hard fail)
#   7. When steps that only assert (warning)
#
# Handles: multi-line fn signatures, async fn, pub fn.
# Uses gsub-count for brace depth (no per-char split).
# ───────────────────────────────────────────────────────────────
step_analysis=$(awk '
    # Detect step attribute
    /^#\[then/ { step_type = "then"; in_step = 1; fn_line = 0; body = ""; brace_depth = 0; seen_open = 0; next }
    /^#\[when/ { step_type = "when"; in_step = 1; fn_line = 0; body = ""; brace_depth = 0; seen_open = 0; next }

    # Detect fn line (handles: fn, pub fn, async fn, pub async fn, indented)
    in_step && !fn_line && /^\s*(pub\s+)?(async\s+)?fn / {
        fn_line = NR
        fn_name = $0
    }

    in_step && fn_line > 0 {
        body = body "\n" $0
        # Count braces via gsub (returns replacement count)
        line = $0
        brace_depth += gsub(/{/, "{", line)
        brace_depth -= gsub(/}/, "}", line)
        if (brace_depth > 0) seen_open = 1

        # Function body complete when braces balance after seeing at least one {
        if (seen_open && brace_depth <= 0) {
            if (step_type == "then") {
                # Check 5: Then steps with no assertions
                has_assert = 0
                if (body ~ /assert!/ || body ~ /assert_eq!/ || body ~ /assert_ne!/ || \
                    body ~ /\.unwrap\(/ || body ~ /\.expect\(/ || body ~ /panic!/) {
                    has_assert = 1
                }
                if (!has_assert) {
                    printf "THEN_NO_ASSERT|%d: %s — Then step has no assertion\n", fn_line, fn_name
                }
            } else if (step_type == "when") {
                # Extract body content (strip fn signature through opening brace, closing brace)
                clean = body
                sub(/^[^{]*\{/, "", clean)
                sub(/\}[^}]*$/, "", clean)
                gsub(/\/\/[^\n]*/, "", clean)
                gsub(/[ \t\n]/, "", clean)

                if (clean == "") {
                    # Check 6: Empty When steps
                    printf "WHEN_NOOP|%d: %s — When step is a no-op (empty body)\n", fn_line, fn_name
                } else if (clean ~ /^assert/) {
                    # Check 7: When steps that only assert (warning)
                    no_assert = clean
                    gsub(/assert[a-z_]*!\([^)]*\)[;]*/, "", no_assert)
                    gsub(/[ \t\n]/, "", no_assert)
                    if (no_assert == "") {
                        printf "WHEN_ASSERT|%d: %s — When step only asserts (no action)\n", fn_line, fn_name
                    }
                }
            }
            in_step = 0; fn_line = 0; body = ""; brace_depth = 0; seen_open = 0
        }
    }
' "$BDD_FILE" || true)

then_no_assert=$(echo "$step_analysis" | grep '^THEN_NO_ASSERT|' | sed 's/^THEN_NO_ASSERT|//' || true)
when_noop=$(echo "$step_analysis" | grep '^WHEN_NOOP|' | sed 's/^WHEN_NOOP|//' || true)
when_assert_only=$(echo "$step_analysis" | grep '^WHEN_ASSERT|' | sed 's/^WHEN_ASSERT|//' || true)

fail "Then steps with no assertions (tests that always pass)" "$then_no_assert"
fail "When steps that are no-ops (stub implementations that test nothing)" "$when_noop"
warn "When steps that only assert preconditions instead of performing actions" "$when_assert_only"

# ───────────────────────────────────────────────────────────────
# 8. Steps that reimplement logic — heuristic detectors.
#    These are warnings (not hard fails) since some false positives
#    are possible. They flag patterns that warrant manual review.
# ───────────────────────────────────────────────────────────────

# 8a. Hand-rolled for-char loops (mini-parsers in test code)
matches=$(grep -n 'for.*in.*\.chars()' "$BDD_FILE" || true)
warn "Hand-rolled char-by-char parsing in test code (should delegate to production code)" "$matches"

# 8b. Building serde_json objects from scratch (may duplicate serialization logic)
matches=$(grep -n 'serde_json::Map::new()' "$BDD_FILE" || true)
warn "Manual serde_json::Map construction (consider using production serializers or test helpers)" "$matches"

# 8c. Raw JSON string templates via format! with r#" (may duplicate config serialization)
matches=$(grep -nP 'format!\(r#"' "$BDD_FILE" || true)
warn "Raw JSON templates via format!(r#\"...) (fragile if config format changes)" "$matches"

# ───────────────────────────────────────────────────────────────
# 9. Silent error swallowing in match arms.
#    Err(_) => {} or Err(_) => () with no logging or assertion.
# ───────────────────────────────────────────────────────────────
matches=$(grep -nP 'Err\(_\)\s*=>\s*\{\s*\}' "$BDD_FILE" || true)
fail "Silent error swallowing: Err(_) => {} (errors should be asserted or propagated)" "$matches"

# ───────────────────────────────────────────────────────────────
# Results
# ───────────────────────────────────────────────────────────────
if [ $FAILED -eq 0 ] && [ $WARNED -eq 0 ]; then
    echo -e "${GREEN}PASS${NC}: BDD quality gate passed — no anti-patterns detected"
elif [ $FAILED -eq 0 ]; then
    echo -e "${YELLOW}PASS (with warnings)${NC}: No hard failures, but review warnings above"
else
    echo -e "${RED}BLOCKED${NC}: Fix the BDD anti-patterns above before committing"
    exit 1
fi

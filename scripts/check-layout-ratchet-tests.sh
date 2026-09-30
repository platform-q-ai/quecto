#!/usr/bin/env bash
# Disposable, bounded historical-budget fixtures; no production-tree edits.
set -euo pipefail
SCRIPT="$(cd "$(dirname "$0")" && pwd)/check-layout-ratchet.sh"
TABLE=quecto-agentic-harness/tests/architecture/layout.rs
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/layout-ratchet-tests.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR

bounded() { timeout -s KILL 10 "$@"; }
table() {
    local output="$1" field="$2" rows="$3" path count
    mkdir -p "$(dirname "$output")"
    {
        printf "const BUDGETS: &[FlatBudget<'_>] = &[\n"
        while read -r path count; do
            if [[ -n "$path" ]]; then
                printf '    FlatBudget { path: "%s", %s: %s },\n' "$path" "$field" "$count"
            fi
        done <<<"$rows"
        printf '];\n'
    } >"$output"
}

check() {
    local name="$1" previous="$2" current="$3" expected="$4" field="${5:-maximum}" diagnostic="${6:-}" mode="${7:-}" status=0
    local root="$SCRATCH/$name"
    mkdir -p "$root"
    (
        cd "$root"
        bounded git init -q -b master
        bounded git config user.email fixture@example.invalid
        bounded git config user.name Fixture
        if [[ "$previous" == absent ]]; then
            printf 'base\n' >fixture
        else
            table "$TABLE" maximum "$previous"
        fi
        bounded git add .
        bounded git -c core.hooksPath=/dev/null commit -q -m base
        base="$(bounded git rev-parse HEAD)"
        table "$TABLE" "$field" "$current"
        case "$mode" in
            decoy) sed -i "1i// const BUDGETS: &[FlatBudget<'_>] = &[];" "$TABLE" ;;
            duplicate) cat "$TABLE" >>"$root/second"; cat "$root/second" >>"$TABLE" ;;
            live) mkdir -p quecto-agentic-harness/src/application ;;
            subdirectory) mkdir -p nested; cd nested ;;
        esac
        bounded bash "$SCRIPT" "$base"
    ) >"$root/output" 2>&1 || status=$?
    if [[ "$status" -eq "$expected" ]]; then
        if [[ -n "$diagnostic" ]]; then
            case "$(cat "$root/output")" in
                *"$diagnostic"*) ;;
                *) cat "$root/output" >&2; echo "FAIL: $name missing diagnostic" >&2; exit 1 ;;
            esac
        fi
        echo "PASS: $name"
    else
        cat "$root/output" >&2
        echo "FAIL: $name: expected $expected, got $status" >&2
        exit 1
    fi
}

check unchanged 'domain 2' 'domain 2' 0
check raised 'domain 2' 'domain 3' 1 maximum domain
check added 'domain 2' $'domain 2\napplication 1' 1 maximum application
check lower-and-remove $'domain 2\napplication 1' 'domain 1' 0
check all-removed 'domain 2' '' 0
check introduction absent 'domain 2' 0
check comment-decoy 'domain 2' 'domain 99' 1 maximum 'exactly one' decoy
check duplicate-table 'domain 2' 'domain 2' 1 maximum 'exactly one' duplicate
check live-row-removal $'domain 2\napplication 1' 'domain 2' 1 maximum 'application' live
check subdirectory 'domain 2' 'domain 2' 0 maximum '' subdirectory

# Delegate real Git operations except the tree-inspection failure under test.
ROOT="$SCRATCH/git-failure"
mkdir -p "$ROOT/bin"
REAL_GIT="$(command -v git)"
cat >"$ROOT/bin/git" <<EOF
#!/usr/bin/env bash
case "\$1" in
    cat-file|ls-tree) echo 'simulated Git failure' >&2; exit 2 ;;
    *) exec "$REAL_GIT" "\$@" ;;
esac
EOF
chmod +x "$ROOT/bin/git"
(
    cd "$ROOT"
    bounded git init -q -b master
    bounded git config user.email fixture@example.invalid
    bounded git config user.name Fixture
    table "$TABLE" maximum 'domain 2'
    bounded git add .
    bounded git -c core.hooksPath=/dev/null commit -q -m base
    status=0
    PATH="$ROOT/bin:$PATH" bounded bash "$SCRIPT" HEAD >output 2>&1 || status=$?
    if [[ "$status" -eq 1 && "$(cat output)" == *'git ls-tree failed'* ]]; then
        echo 'PASS: Git tree failure rejected'
    else
        cat output >&2; echo "FAIL: Git tree failure: got $status" >&2; exit 1
    fi
)
echo '11 layout ratchet fixtures passed'

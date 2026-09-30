#!/usr/bin/env bash
# Budget rows may only shrink against the merge base (#2358 amendment 9).
# Usage: scripts/check-layout-ratchet.sh [<base>]
set -euo pipefail
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
python3 - "$BASE" <<'PY'
import pathlib
import re
import subprocess
import sys

path = "quecto-agentic-harness/tests/architecture/layout.rs"
base = sys.argv[1]

def budgets(source):
    table = re.search(r"const BUDGETS\s*:[^=]+?=\s*&\[(.*?)\];", source, re.S)
    if table is None:
        raise ValueError(f"{path}: BUDGETS table missing; restore the budget table")
    row = re.compile(r'FlatBudget\s*\{\s*path:\s*"([^"]+)"\s*,\s*(?:maximum|expected):\s*(\d+)\s*,?\s*\}\s*,?', re.S)
    body = re.sub(r"//[^\n]*", "", table[1])
    if row.sub("", body).strip() == "":
        return dict((name, int(count)) for name, count in row.findall(body))
    raise ValueError(f"{path}: cannot read BUDGETS; keep literal path/expected rows")

try:
    head = budgets(pathlib.Path(path).read_text())
    entries = subprocess.run(["git", "ls-tree", "--name-only", base, "--", path],
                             check=True, capture_output=True, text=True, timeout=10)
    if entries.stdout in ("", path + "\n"):
        present = entries.stdout == path + "\n"
    else:
        raise ValueError(f"{path}: unexpected git ls-tree output; inspect the base tree")
    if present:
        previous = subprocess.run(["git", "show", f"{base}:{path}"], check=True, capture_output=True, text=True, timeout=10)
        old = budgets(previous.stdout)
        failures = []
        for name, count in head.items():
            if name in old and count <= old[name]:
                continue
            failures.append(f"{path}: {name} budget {count} is new or raised; remove the row or lower it to the merge-base budget")
        if failures:
            print("\n".join(failures), file=sys.stderr)
            sys.exit(1)
    print(f"layout ratchet: budgets only shrink against {base}")
except (OSError, ValueError, subprocess.SubprocessError) as error:
    print(f"layout ratchet: {error}", file=sys.stderr)
    sys.exit(1)
PY

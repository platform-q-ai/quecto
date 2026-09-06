# Spawn-ready task bodies and acceptance cards

Generated from `catalog.py` (the fixture authority). All variants are deterministic,
standard-library-only, offline and independent of Rust, Git, and GitHub. The fixed
workspace path is `/workspace/task`. See LAUNCH.md for current idle-agent provisioning. Do not copy this evaluator document or catalog
to workers: copy only the selected fixture and its task body. Prompts state product
requirements, not RED/GREEN, step-checking, rubric, or workflow-improvement advice.

Baseline launch adds exactly `Select the built-in <workflow> workflow for this task.`
as an assignment preamble. Revised bound launch omits only that preamble. No other
behavioral coaching is added. Existing smoke tests may be augmented with new test
files; changing existing tests is separately reviewed for weakening.

| Workflow | Baseline | First varied validation | Second varied validation |
|---|---|---|---|
| investigate | Configuration precedence | Timeout/duplicate-send evidence | Specification versus expectation |
| chore | CLI documentation repair | JSON example policy alignment | Relative documentation links |
| bugfix | Half-open interval boundary | Exact decimal cents | Falsy configuration overrides |
| feature | Optional search limit | Ordered category aggregation | JSON CLI output mode |
| refactor | Shared receipt formatting | Extract classifier module | Authorization guard clauses |

## Fixture/setup and verification

For TASK in the catalog, evaluator runs `fixture_tool.py setup --task TASK --root
/workspace/task --output /evidence/setup.json` **before worker activation**, in the
fresh container preparation boundary (or exports that exact resulting tree into the
container). Root must not exist. Only generated task files become worker-visible.
No package install, Git initialization, clock, random seed or external service is
needed. Evaluator keeps metadata and oracles outside worker access. Interpreter is
Python 3.10+; freeze its exact version and image digest across comparisons.

Verification on an archived copy in an isolated sandbox: `fixture_tool.py verify
--task TASK --root /archive/task --output /evidence/verification.json`. This executes
the frozen independent oracle and discovered worker tests, captures commands, exit
codes, stdout/stderr, changed paths and preservation violations. It is NOT a security
sandbox by itself. No acceptance comes from exit code alone: judges audit the code,
structural refactors, investigative answer and test relevance. Doc parser limitations
are evaluator limitations, not unstated formatting requirements. All documentation parser mismatches receive semantic direct review/replay from
retained artifacts; no worker rerun or gate failure solely for representation.

Source/input byte snapshots exclude only `__pycache__` and `.pyc`. An unrelated
`notes/operator-draft.txt` is present in every fixture to check preservation. Build
output is allowed only for chore-base under `build/`. Reports for read-only tasks
belong in final responses, never workspace files. Evaluator-owned oracles and
acceptance cards are not installed in worker fixtures.


Path allowlists below are evaluator review flags, not additional hidden task requirements.

## investigate-base (investigate; read-only)

### Task body

```text
In /workspace/task, explain why the preview service binds port 8123 even though settings.json says 9000. Identify the effective configuration precedence, distinguish facts from uncertainty, and recommend the smallest appropriate next action. This is a diagnosis only: do not change or create workspace files. Return your findings in your final response.
```

### Evaluator acceptance

Fixture files: `README.md`, `settings.json`, `launch.json`, `preview.py`, `logs/preview.log`, `notes/operator-draft.txt`

Expected edit paths (review flags only): none

- Explains environment PORT > settings port > default 8000 using resolve_port and the captured launch input.
- Shows 8123 is expected for supplied input, not evidence of a stale settings cache; missing override yields 9000.
- Recommends removing/changing the launch override if 9000 is desired, without claiming to have changed a live deployment.

Critical conditions, interpreted against explicit task contract:
- Attributes the observed port to a different primary cause despite direct contradictory source/input evidence.
- Any workspace mutation (apart from explicitly excluded interpreter caches).

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
from preview import resolve_port
assert resolve_port({"port": 9000}, {"PORT": "8123"}) == 8123
assert resolve_port({"port": 9000}, {}) == 9000
assert resolve_port({}, {}) == 8000
print("effective=8123; without launch override=9000; default=8000")
```

## investigate-v1 (investigate; read-only)

### Task body

```text
In /workspace/task, investigate why receipt R7 has two successful sends in the supplied event log. Explain what the retry implementation does, the most likely cause, and what the supplied evidence cannot establish about the receiver. Do not change or create workspace files; return findings in your final response.
```

### Evaluator acceptance

Fixture files: `README.md`, `delivery.py`, `events.json`, `receiver-contract.txt`, `notes/operator-draft.txt`

Expected edit paths (review flags only): none

- Connects first acceptance then client timeout to unconditional retry of the same identifier.
- Distinguishes two accepted requests from proof of duplicate business side effects; receiver deduplication is unknown.
- Recommends inspecting receiver idempotency/ack handling rather than claiming a confirmed receiver defect.

Critical conditions, interpreted against explicit task contract:
- Claims duplicate receiver side effects or their absence as established fact.
- Any workspace mutation except interpreter caches.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
from delivery import deliver
calls = []
def send(receipt):
    calls.append(receipt)
    if len(calls) == 1:
        raise TimeoutError("ack lost")
    return "ok"
assert deliver(send, "R7") == "ok" and calls == ["R7", "R7"]
print("timeout triggers a second send of the same receipt; receiver side effects unknown")
```

## investigate-v2 (investigate; read-only)

### Task body

```text
In /workspace/task, explain why the report totals 5 units instead of the operator's expected 9. Identify the relevant filtering semantics and whether the supplied specification supports that expectation. Do not change or create workspace files; return your diagnosis in your final response.
```

### Evaluator acceptance

Fixture files: `README.md`, `SPEC.md`, `rows.json`, `report.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): none

- Explains exact-match filtering and identifies the 4-unit uppercase record.
- Recognizes current result conforms to SPEC; expected 9 requires a clarified/changed contract or normalized upstream data.
- Separates demonstrated local behavior from unobserved upstream intent.

Critical conditions, interpreted against explicit task contract:
- Declares existing implementation contrary to the supplied exact-case contract.
- Any workspace mutation except interpreter caches.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
import json
from pathlib import Path
from report import total
rows = json.loads(Path("rows.json").read_text())
assert total(rows) == 5
assert sum(r["units"] for r in rows if r["status"].lower() == "paid") == 9
print("exact paid=5; case-insensitive counterfactual=9; current behavior conforms to SPEC")
```

## chore-base (chore; docs)

### Task body

```text
In /workspace/task, update README.md so its export example and option reference agree with the current export.py CLI. The example should export data/items.json to build/items.csv with the default comma delimiter. Keep the introductory description and compatibility note. This is documentation maintenance; do not change program behavior or other existing files.
```

### Evaluator acceptance

Fixture files: `README.md`, `export.py`, `data/items.json`, `notes/operator-draft.txt`

Expected edit paths (review flags only): README.md, build/**

- README documents positional source, required --dest, optional --delimiter with comma default and a runnable requested example.
- Preserves introductory description and compatibility note; no program changes.
- Actual documented example produces the expected CSV, including a quoted comma-containing name.

Critical conditions, interpreted against explicit task contract:
- Documented primary export command cannot perform the requested export.
- Product source or unrelated existing files changed.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
import csv, subprocess, sys, tempfile
from pathlib import Path
doc = Path("README.md").read_text()
assert "Export item records as a CSV file for spreadsheet users." in doc
assert "Consumers rely on the id,name column order." in doc
for old in ("--input", "--output", "--separator", "default semicolon"):
    assert old not in doc, old
for new in ("--dest", "--delimiter", "data/items.json", "build/items.csv"):
    assert new in doc, new
# Extract the actual documented example rather than substitute a correct command.
import shlex
lines = [line.strip().strip("`") for line in doc.splitlines()]
examples = [shlex.split(line) for line in lines if line.startswith("python3 ") and "export.py" in line]
assert examples, "expected a standalone python3 export.py example"
cmd = examples[0]
assert cmd[:2] == ["python3", "export.py"] and "data/items.json" in cmd
assert cmd[cmd.index("--dest") + 1] == "build/items.csv"
assert "--delimiter" not in cmd or cmd[cmd.index("--delimiter") + 1] == ","
with tempfile.TemporaryDirectory() as tmp:
    cmd[0] = sys.executable
    cmd[cmd.index("--dest") + 1] = str(Path(tmp) / "items.csv")
    subprocess.run(cmd, check=True, timeout=10, capture_output=True)
    with (Path(tmp) / "items.csv").open(newline="") as f:
        assert list(csv.reader(f)) == [["id", "name"], ["1", "tea"], ["2", "red, blue"]]
print("documented example exports correct CSV; panel checks option descriptions")
```

## chore-v1 (chore; config)

### Task body

```text
In /workspace/task, align config/example.json with the supplied local-development policy: bind only to 127.0.0.1, disable debug, and retain port 8080 and the existing logging settings. Remove the obsolete trace field, which the loader rejects. Do not change the loader or policy; this is example-configuration maintenance.
```

### Evaluator acceptance

Fixture files: `README.md`, `POLICY.md`, `config/example.json`, `loader.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): config/example.json

- Exact policy-conforming JSON, no obsolete trace key.
- Loader, policy, port, and logging remain unchanged.

Critical conditions, interpreted against explicit task contract:
- Example still exposes all interfaces, enables debug, or fails loader validation.
- Loader or policy weakened to accept invalid configuration.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
import json
from pathlib import Path
from loader import validate
config = json.loads(Path("config/example.json").read_text())
assert type(config["debug"]) is bool
assert validate(config) == {"host":"127.0.0.1", "port":8080, "debug":False,
                            "logging":{"level":"info", "format":"json"}}
print("example conforms to loader and local policy")
```

## chore-v2 (chore; docs)

### Task body

```text
In /workspace/task, repair the two stale local links in docs/start.md to point to the existing install and troubleshooting pages. Preserve the visible link text and all other prose. Do not move pages or edit other existing files.
```

### Evaluator acceptance

Fixture files: `README.md`, `docs/start.md`, `docs/guide/install.md`, `docs/support/troubleshooting.md`, `notes/operator-draft.txt`

Expected edit paths (review flags only): docs/start.md

- Both links resolve relative to docs/start.md to the correct existing pages.
- Visible text, remaining prose, and other files unchanged.

Critical conditions, interpreted against explicit task contract:
- Either requested link remains broken or points to the wrong page.
- Deletes or moves existing pages.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
from pathlib import Path
import re
p = Path("docs/start.md")
doc = p.read_text()
links = re.findall(r"\[([^]]+)\]\(([^)]+)\)", doc)
assert [label for label, _ in links] == ["installation", "troubleshooting"]
assert [(p.parent / target).resolve() for _, target in links] == [(p.parent / target).resolve() for target in ("guide/install.md", "support/troubleshooting.md")]
for _, target in links:
    assert (p.parent / target).is_file()
original_shape = "# Start\nUse [installation](LINK) first.\nFor help, see [troubleshooting](LINK).\nOffline use is supported.\n"
assert re.sub(r"\]\([^)]+\)", "](LINK)", doc) == original_shape
print("both relative links resolve; prose unchanged")
```

## bugfix-base (bugfix; bug)

### Task body

```text
In /workspace/task, fix intervals.merge: touching half-open intervals must remain separate. Overlapping intervals must still merge, inputs may be unsorted, and the caller's list must not be changed. Inputs are valid integer (start, end) pairs with start < end. Preserve the public API and add appropriate regression coverage.
```

### Evaluator acceptance

Fixture files: `README.md`, `intervals.py`, `tests/test_smoke.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): intervals.py, tests/**

- Separates touching intervals, merges true overlaps transitively, handles empty/unsorted/nested/duplicate/negative cases.
- Preserves input and API; worker adds meaningful regression coverage.

Critical conditions, interpreted against explicit task contract:
- Touching bookings still merge, overlapping bookings fail to merge, or input is mutated.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
from intervals import merge
cases = [([], []), ([(1,2),(2,3)], [(1,2),(2,3)]),
         ([(5,7),(1,4),(3,6)], [(1,7)]), ([(1,8),(2,3)], [(1,8)]),
         ([(1,3),(1,3)], [(1,3)]), ([(-4,-2),(-2,0)], [(-4,-2),(-2,0)]),
         ([(8,9)], [(8,9)]), ([(1,4),(4,6),(3,5)], [(1,6)])]
for value, expected in cases:
    before = list(value)
    assert merge(value) == expected, (value, expected)
    assert value == before, "mutated input"
print("8 interval cases and input preservation pass")
```

## bugfix-v1 (bugfix; bug)

### Task body

```text
In /workspace/task, fix receipts.total_cents so decimal currency strings are totaled exactly rather than losing cents to binary floating point. Inputs are nonnegative plain decimal strings with zero, one, or two fractional digits, within 0 through 1000000 inclusive. Return integer cents, support an empty list, preserve the API, and add regression coverage. Validation of other inputs is out of scope.
```

### Evaluator acceptance

Fixture files: `README.md`, `receipts.py`, `tests/test_smoke.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): receipts.py, tests/**

- Exact integer-cent totals across cents, single fractional digit, large and repeated values, and empty input.
- No input mutation or new input-validation requirements; regression coverage added.

Critical conditions, interpreted against explicit task contract:
- Any specified valid amount loses or gains cents.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
from receipts import total_cents
for values, expected in [([],0),(["0.29"],29),(["0.57","0.29"],86),
                          (["1.2","3","0.01"],421),(["1000000.00"],100000000),
                          (["0.01"] * 1000,1000),(["0","0.00"],0)]:
    before = list(values)
    actual = total_cents(values)
    assert type(actual) is int and actual == expected, (values, actual, expected)
    assert values == before
print("7 exact-currency cases pass")
```

## bugfix-v2 (bugfix; bug)

### Task body

```text
In /workspace/task, fix settings.resolve so an explicitly supplied false, zero, or empty string overrides the default rather than being discarded. Only None or a missing key should use the default. Return a new dictionary, leave both inputs untouched, and continue ignoring unknown override keys. Preserve the API and add regression coverage.
```

### Evaluator acceptance

Fixture files: `README.md`, `settings.py`, `tests/test_smoke.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): settings.py, tests/**

- False/zero/empty string override; None and absent keys fall back; unknown keys ignored.
- Fresh dictionary, no input mutation, meaningful regressions.

Critical conditions, interpreted against explicit task contract:
- Any required falsy override is discarded or source input changed.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
from settings import resolve
d = {"debug":True,"retries":3,"prefix":"x","host":"local","port":8000}
o = {"debug":False,"retries":0,"prefix":"","host":None,"unknown":1}
before_d, before_o = dict(d), dict(o)
result = resolve(d, o)
assert result == {"debug":False,"retries":0,"prefix":"","host":"local","port":8000}
assert result is not d and result is not o
assert d == before_d and o == before_o
assert resolve({}, {"x":0}) == {}
assert resolve({"x":None}, {}) == {"x":None}
print("falsy, None, missing, unknown and ownership checks pass")
```

## feature-base (feature; feature)

### Task body

```text
In /workspace/task, add an optional keyword argument limit=None to search.find. With None, preserve current case-insensitive substring matching and input order. A nonnegative integer returns at most that many matching names, with zero returning an empty list; duplicates remain meaningful. Reject negative integers with ValueError. Other limit types and non-string names are out of scope. Keep inputs untouched and cover the new behavior.
```

### Evaluator acceptance

Fixture files: `README.md`, `search.py`, `tests/test_smoke.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): search.py, tests/**, README.md

- New optional limit handles None/zero/positive/negative as specified.
- Case-insensitive substring, order, duplicates, empty query/input, unmatched query, and input ownership preserved.

Critical conditions, interpreted against explicit task contract:
- Required limit cases fail or legacy unlimited search regresses.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
from search import find
names = ["Tea","coffee","teapot","Tea"]
before = list(names)
assert find(names, "TEA") == ["Tea","teapot","Tea"]
assert find(["Straße"], "STRASSE") == ["Straße"]
assert find(["Straße"], "STRASSE", limit=1) == ["Straße"]
assert find(names, "tea", limit=None) == ["Tea","teapot","Tea"]
assert find(names, "tea", limit=0) == []
assert find(names, "tea", limit=2) == ["Tea","teapot"]
assert find(names, "tea", limit=9) == ["Tea","teapot","Tea"]
assert find(names, "", limit=2) == names[:2]
assert find([], "", limit=2) == [] and find(names,"zzz",limit=1) == []
try:
    find(names, "tea", limit=-1)
except ValueError:
    pass
else:
    raise AssertionError("negative limit accepted")
assert names == before
print("limit contract and legacy search compatibility pass")
```

## feature-v1 (feature; feature)

### Task body

```text
In /workspace/task, add rollup.by_category(rows). Each row has a string category and an integer amount. Return a new dict mapping categories to summed amounts, ordered by first appearance. Missing category means 'uncategorized'; an explicitly empty category remains empty. Include zero totals, permit negative amounts, return {} for empty input, and do not mutate rows. Inputs otherwise satisfy the contract. Cover this new API without changing total(rows).
```

### Evaluator acceptance

Fixture files: `README.md`, `rollup.py`, `tests/test_smoke.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): rollup.py, tests/**, README.md

- Correct grouping, first-appearance order, missing versus empty category, zero and negative totals.
- No mutation and existing total unchanged; new API covered.

Critical conditions, interpreted against explicit task contract:
- Incorrect sums or category semantics for specified valid rows; existing total regresses.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
import copy
from rollup import by_category, total
rows = [{"category":"food","amount":3},{"category":"travel","amount":4},
        {"category":"food","amount":-3},{"amount":2},{"category":"","amount":5},
        {"category":"uncategorized","amount":-1}]
before = copy.deepcopy(rows)
result = by_category(rows)
assert list(result.items()) == [("food",0),("travel",4),("uncategorized",1),("",5)]
assert by_category([]) == {} and total(rows) == 10
assert rows == before
print("category aggregation, order, zero totals, defaults and ownership pass")
```

## feature-v2 (feature; feature)

### Task body

```text
In /workspace/task, extend the existing linecount.py CLI with --json. Default output must remain exactly 'lines=N' followed by a newline. With --json, emit a JSON object with only the key 'lines' with an integer value, followed by a newline. Count lines from standard input, including a final non-newline-terminated line; empty input counts zero. Keep normal --help behavior and cover both modes.
```

### Evaluator acceptance

Fixture files: `README.md`, `linecount.py`, `tests/test_smoke.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): linecount.py, tests/**, README.md

- JSON mode has exactly integer lines field and trailing newline; default text byte-compatible.
- Empty, final partial, and blank lines counted correctly; help remains usable.

Critical conditions, interpreted against explicit task contract:
- New mode cannot be used or default mode changes; incorrect required counts.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
import json, subprocess, sys
for data, count in [("",0),("a\n",1),("a\nb",2),("\n\n",2)]:
    for args in ([], ["--json"]):
        r = subprocess.run([sys.executable,"-B","linecount.py"] + args,
                           input=data,text=True,capture_output=True,timeout=10)
        assert r.returncode == 0 and r.stderr == "", r
        assert r.stdout.endswith("\n")
        if args:
            obj = json.loads(r.stdout)
            assert obj == {"lines":count} and type(obj["lines"]) is int
        else:
            assert r.stdout == f"lines={count}\n"
help_result = subprocess.run([sys.executable,"-B","linecount.py","--help"],capture_output=True,text=True,timeout=10)
assert help_result.returncode == 0 and "--json" in help_result.stdout
print("8 CLI mode/input cases and help pass")
```

## refactor-base (refactor; refactor)

### Task body

```text
In /workspace/task, remove the duplicated receipt-line formatting from receipts.text_receipt and receipts.html_receipt by extracting one shared private formatter that both use. Preserve public signatures and exact returned strings, including HTML escaping, item order, empty receipts, negative cents, and singular/plural behavior. Do not fix or redesign observable behavior. Add suitable parity coverage.
```

### Evaluator acceptance

Fixture files: `README.md`, `receipts.py`, `tests/test_smoke.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): receipts.py, tests/**

- Both public functions actually use a single private line formatter; duplicated semantic formatting removed.
- Exact strings, escaping, order, empty, quantity and cents behavior and signatures preserved; parity tests added.

Critical conditions, interpreted against explicit task contract:
- Any supplied valid input's output changes, especially lost HTML escaping.
- No substantive requested shared extraction.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
import copy, html, inspect
from receipts import text_receipt, html_receipt
assert str(inspect.signature(text_receipt)) == "(items)"
assert str(inspect.signature(html_receipt)) == "(items)"
cases = [[],[("tea",1,20)],[("<&\\\"'",0,-3),("tea",2,100)],[("é",-1,0),("tea",1,0)]]
for items in cases:
    before = copy.deepcopy(items)
    lines = [f"{name}: {qty} {'item' if qty == 1 else 'items'} @ {cents}c" for name,qty,cents in items]
    assert text_receipt(items) == "\n".join(lines)
    assert html_receipt(items) == "<ul>" + "".join("<li>" + html.escape(line) + "</li>" for line in lines) + "</ul>"
    assert items == before
print("exact text/HTML parity and signatures pass; panel must inspect actual shared extraction")
```

## refactor-v1 (refactor; refactor)

### Task body

```text
In /workspace/task, move the filename classification rules out of inventory.summarize into a private classify_name(name) helper in a new classification.py module, imported by inventory.py (the helper is internal, not a promised public API). Keep summarize's signature and results identical, including case sensitivity, last-suffix behavior, unknown extensions, and order. Add parity coverage; do not expand supported types.
```

### Evaluator acceptance

Fixture files: `README.md`, `inventory.py`, `tests/test_smoke.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): inventory.py, classification.py, tests/**

- Classification rules live once in new internal module helper, used by summarize; aggregation stays in inventory.
- Case-sensitive last suffix, unknowns, order, counts, signature, input ownership all preserved.

Critical conditions, interpreted against explicit task contract:
- Observable summarize output changes or extraction is absent/disconnected.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
import inspect
from inventory import summarize
assert str(inspect.signature(summarize)) == "(names)"
cases = [([],[]), (["x.JSON","a.txt","b.csv","c.md","z"],[("other",2),("text",2),("data",1)]),
         (["a.tar.json",".txt","file.","MD"],[("data",1),("text",1),("other",2)])]
for names, expected in cases:
    before = list(names)
    assert list(summarize(names).items()) == expected
    assert names == before
from classification import classify_name
assert classify_name("a.txt") == "text" and classify_name("a.TXT") == "other"
print("module extraction API and behavior parity pass; panel checks dependency and rule ownership")
```

## refactor-v2 (refactor; refactor)

### Task body

```text
In /workspace/task, simplify access.decide by replacing its nested conditionals with clear guard clauses. Preserve its signature and all current decisions and reason strings for every combination of the four boolean arguments. Keep access precedence unchanged, and add parity coverage. No new authorization policy is requested.
```

### Evaluator acceptance

Fixture files: `README.md`, `access.py`, `tests/test_smoke.py`, `notes/operator-draft.txt`

Expected edit paths (review flags only): access.py, tests/**

- Nested decision tree replaced by clear guard clauses, not merely renamed or reformatted.
- All 16 decisions, precedence, reason strings, bool results, and signature preserved.

Critical conditions, interpreted against explicit task contract:
- Any authorization decision/reason changes or no substantive guard-clause refactor.

Frozen executable verification (documentation semantics may use declared direct-review fallback):

```python
import inspect, itertools
from access import decide
assert str(inspect.signature(decide)) == "(active, locked, admin, owner)"
for active,locked,admin,owner in itertools.product((False,True), repeat=4):
    expected = ((False,"inactive") if not active else (False,"locked") if locked else
                (True,"admin") if admin else (True,"owner") if owner else (False,"not-owner"))
    actual = decide(active,locked,admin,owner)
    assert actual == expected and type(actual[0]) is bool
print("all 16 authorization combinations and exact reasons preserved; panel checks guard-clause structure")
```

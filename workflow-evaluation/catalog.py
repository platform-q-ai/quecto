"""Proposed frozen task fixtures. Evaluator-owned; never copy this module to workers.
Python 3.10+ standard library only. No worker execution or spawning here.
"""
import textwrap

TASKS = {}


def text(s):
    return textwrap.dedent(s).lstrip("\n")


def add(id, workflow, prompt, files, editable, oracle, acceptance, critical, kind):
    files = {p: text(s) for p, s in files.items()}
    files["notes/operator-draft.txt"] = "Unrelated operator draft: retain this exact line.\n"
    files["README.md"] = text(files.get("README.md", "")) + (
        "\nRuntime: Python 3.10+ standard library; no installation or network needed.\n"
        "Existing smoke checks: python3 -B -m unittest discover -s tests -v\n"
        if "tests/test_smoke.py" in files else
        "\nRuntime: Python 3.10+ standard library; no installation or network needed.\n"
    )
    TASKS[id] = dict(id=id, workflow=workflow, prompt=prompt, files=files,
                     editable=editable, oracle=text(oracle), acceptance=acceptance,
                     critical=critical, kind=kind)


add("investigate-base", "investigate",
    "In /workspace/task, explain why the preview service binds port 8123 even though settings.json says 9000. Identify the effective configuration precedence, distinguish facts from uncertainty, and recommend the smallest appropriate next action. This is a diagnosis only: do not change or create workspace files. Return your findings in your final response.",
    {
        "README.md": "Preview service. Reproduce its configuration with: python3 -B preview.py\nThe launch environment is captured in launch.json, not the current shell.\n",
        "settings.json": '{"port": 9000}\n',
        "launch.json": '{"PORT": "8123"}\n',
        "preview.py": '''
            import json
            from pathlib import Path

            def resolve_port(config, env):
                return int(env.get("PORT", config.get("port", 8000)))

            if __name__ == "__main__":
                config = json.loads(Path("settings.json").read_text())
                env = json.loads(Path("launch.json").read_text())
                print(resolve_port(config, env))
        ''',
        "logs/preview.log": "INFO request cache miss\nINFO listening port=8123\n",
    }, [], r'''
        from preview import resolve_port
        assert resolve_port({"port": 9000}, {"PORT": "8123"}) == 8123
        assert resolve_port({"port": 9000}, {}) == 9000
        assert resolve_port({}, {}) == 8000
        print("effective=8123; without launch override=9000; default=8000")
    ''',
    ["Explains environment PORT > settings port > default 8000 using resolve_port and the captured launch input.",
     "Shows 8123 is expected for supplied input, not evidence of a stale settings cache; missing override yields 9000.",
     "Recommends removing/changing the launch override if 9000 is desired, without claiming to have changed a live deployment."],
    ["Attributes the observed port to a different primary cause despite direct contradictory source/input evidence.", "Any workspace mutation (apart from explicitly excluded interpreter caches)."], "read-only")

add("investigate-v1", "investigate",
    "In /workspace/task, investigate why receipt R7 has two successful sends in the supplied event log. Explain what the retry implementation does, the most likely cause, and what the supplied evidence cannot establish about the receiver. Do not change or create workspace files; return findings in your final response.",
    {
        "README.md": "Receipt delivery excerpt; this is the complete captured local evidence, not a live service.\n",
        "delivery.py": '''
            def deliver(send, receipt):
                for attempt in range(2):
                    try:
                        return send(receipt)
                    except TimeoutError:
                        if attempt == 1:
                            raise
        ''',
        "events.json": '''
            [
              {"receipt":"R7","attempt":1,"event":"receiver accepted"},
              {"receipt":"R7","attempt":1,"event":"client timeout"},
              {"receipt":"R7","attempt":2,"event":"receiver accepted"},
              {"receipt":"R7","attempt":2,"event":"client success"}
            ]
        ''',
        "receiver-contract.txt": "POST /receipts accepts a receipt identifier. Idempotency behavior is not recorded in this excerpt.\n",
    }, [], r'''
        from delivery import deliver
        calls = []
        def send(receipt):
            calls.append(receipt)
            if len(calls) == 1:
                raise TimeoutError("ack lost")
            return "ok"
        assert deliver(send, "R7") == "ok" and calls == ["R7", "R7"]
        print("timeout triggers a second send of the same receipt; receiver side effects unknown")
    ''',
    ["Connects first acceptance then client timeout to unconditional retry of the same identifier.",
     "Distinguishes two accepted requests from proof of duplicate business side effects; receiver deduplication is unknown.",
     "Recommends inspecting receiver idempotency/ack handling rather than claiming a confirmed receiver defect."],
    ["Claims duplicate receiver side effects or their absence as established fact.", "Any workspace mutation except interpreter caches."], "read-only")

add("investigate-v2", "investigate",
    "In /workspace/task, explain why the report totals 5 units instead of the operator's expected 9. Identify the relevant filtering semantics and whether the supplied specification supports that expectation. Do not change or create workspace files; return your diagnosis in your final response.",
    {
        "README.md": "Run: python3 -B report.py\n",
        "SPEC.md": "The report includes only records whose status is exactly lowercase 'paid'. Other statuses are excluded; case folding is not specified.\n",
        "rows.json": '[{"status":"paid","units":2},{"status":"paid","units":3},{"status":"PAID","units":4}]\n',
        "report.py": '''
            import json
            from pathlib import Path
            def total(rows):
                return sum(r["units"] for r in rows if r["status"] == "paid")
            if __name__ == "__main__":
                print(total(json.loads(Path("rows.json").read_text())))
        ''',
    }, [], r'''
        import json
        from pathlib import Path
        from report import total
        rows = json.loads(Path("rows.json").read_text())
        assert total(rows) == 5
        assert sum(r["units"] for r in rows if r["status"].lower() == "paid") == 9
        print("exact paid=5; case-insensitive counterfactual=9; current behavior conforms to SPEC")
    ''',
    ["Explains exact-match filtering and identifies the 4-unit uppercase record.",
     "Recognizes current result conforms to SPEC; expected 9 requires a clarified/changed contract or normalized upstream data.",
     "Separates demonstrated local behavior from unobserved upstream intent."],
    ["Declares existing implementation contrary to the supplied exact-case contract.", "Any workspace mutation except interpreter caches."], "read-only")

add("chore-base", "chore",
    "In /workspace/task, update README.md so its export example and option reference agree with the current export.py CLI. The example should export data/items.json to build/items.csv with the default comma delimiter. Keep the introductory description and compatibility note. This is documentation maintenance; do not change program behavior or other existing files.",
    {
        "README.md": '''
            # Item export
            Export item records as a CSV file for spreadsheet users.

            ## Example
            python3 export.py --input data/items.json --output build/items.csv

            ## Options
            --input: JSON source file
            --output: CSV destination file
            --separator: column separator (default semicolon)

            ## Compatibility
            Consumers rely on the id,name column order.
        ''',
        "export.py": '''
            import argparse
            import csv
            import json
            from pathlib import Path
            def main():
                p = argparse.ArgumentParser()
                p.add_argument("source")
                p.add_argument("--dest", required=True)
                p.add_argument("--delimiter", default=",")
                args = p.parse_args()
                rows = json.loads(Path(args.source).read_text())
                target = Path(args.dest)
                target.parent.mkdir(parents=True, exist_ok=True)
                with target.open("w", newline="") as f:
                    writer = csv.writer(f, delimiter=args.delimiter)
                    writer.writerow(["id", "name"])
                    writer.writerows((r["id"], r["name"]) for r in rows)
            if __name__ == "__main__":
                main()
        ''',
        "data/items.json": '[{"id":1,"name":"tea"},{"id":2,"name":"red, blue"}]\n',
    }, ["README.md", "build/**"], r'''
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
    ''',
    ["README documents positional source, required --dest, optional --delimiter with comma default and a runnable requested example.",
     "Preserves introductory description and compatibility note; no program changes.",
     "Actual documented example produces the expected CSV, including a quoted comma-containing name."],
    ["Documented primary export command cannot perform the requested export.", "Product source or unrelated existing files changed."], "docs")

add("chore-v1", "chore",
    "In /workspace/task, align config/example.json with the supplied local-development policy: bind only to 127.0.0.1, disable debug, and retain port 8080 and the existing logging settings. Remove the obsolete trace field, which the loader rejects. Do not change the loader or policy; this is example-configuration maintenance.",
    {
        "README.md": "Example configuration for local development. Policy is in POLICY.md.\n",
        "POLICY.md": "Local examples bind 127.0.0.1 with debug false. Port 8080 and logging settings stay unchanged. Allowed fields are host, port, debug, logging.\n",
        "config/example.json": '{"host":"0.0.0.0","port":8080,"debug":true,"logging":{"level":"info","format":"json"},"trace":true}\n',
        "loader.py": '''
            def validate(config):
                extra = set(config) - {"host", "port", "debug", "logging"}
                if extra:
                    raise ValueError("unknown fields: " + ",".join(sorted(extra)))
                return dict(config)
        ''',
    }, ["config/example.json"], r'''
        import json
        from pathlib import Path
        from loader import validate
        config = json.loads(Path("config/example.json").read_text())
        assert type(config["debug"]) is bool
        assert validate(config) == {"host":"127.0.0.1", "port":8080, "debug":False,
                                    "logging":{"level":"info", "format":"json"}}
        print("example conforms to loader and local policy")
    ''',
    ["Exact policy-conforming JSON, no obsolete trace key.", "Loader, policy, port, and logging remain unchanged."],
    ["Example still exposes all interfaces, enables debug, or fails loader validation.", "Loader or policy weakened to accept invalid configuration."], "config")

add("chore-v2", "chore",
    "In /workspace/task, repair the two stale local links in docs/start.md to point to the existing install and troubleshooting pages. Preserve the visible link text and all other prose. Do not move pages or edit other existing files.",
    {
        "README.md": "Local documentation bundle; all links are relative to their containing file.\n",
        "docs/start.md": "# Start\nUse [installation](install.md) first.\nFor help, see [troubleshooting](help.md).\nOffline use is supported.\n",
        "docs/guide/install.md": "# Install\nUse the supplied package.\n",
        "docs/support/troubleshooting.md": "# Troubleshooting\nCheck the local log.\n",
    }, ["docs/start.md"], r'''
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
    ''',
    ["Both links resolve relative to docs/start.md to the correct existing pages.", "Visible text, remaining prose, and other files unchanged."],
    ["Either requested link remains broken or points to the wrong page.", "Deletes or moves existing pages."], "docs")

add("bugfix-base", "bugfix",
    "In /workspace/task, fix intervals.merge: touching half-open intervals must remain separate. Overlapping intervals must still merge, inputs may be unsorted, and the caller's list must not be changed. Inputs are valid integer (start, end) pairs with start < end. Preserve the public API and add appropriate regression coverage.",
    {
        "README.md": "Booking intervals are half-open [start,end). merge returns sorted tuples and does not mutate inputs.\n",
        "intervals.py": '''
            def merge(intervals):
                result = []
                for start, end in sorted(intervals):
                    if result and start <= result[-1][1]:
                        left, right = result[-1]
                        result[-1] = (left, max(right, end))
                    else:
                        result.append((start, end))
                return result
        ''',
        "tests/test_smoke.py": '''
            import unittest
            from intervals import merge
            class Smoke(unittest.TestCase):
                def test_overlap(self):
                    self.assertEqual(merge([(1, 4), (2, 6)]), [(1, 6)])
        ''',
    }, ["intervals.py", "tests/**"], r'''
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
    ''',
    ["Separates touching intervals, merges true overlaps transitively, handles empty/unsorted/nested/duplicate/negative cases.",
     "Preserves input and API; worker adds meaningful regression coverage."],
    ["Touching bookings still merge, overlapping bookings fail to merge, or input is mutated."], "bug")

add("bugfix-v1", "bugfix",
    "In /workspace/task, fix receipts.total_cents so decimal currency strings are totaled exactly rather than losing cents to binary floating point. Inputs are nonnegative plain decimal strings with zero, one, or two fractional digits, within 0 through 1000000 inclusive. Return integer cents, support an empty list, preserve the API, and add regression coverage. Validation of other inputs is out of scope.",
    {
        "README.md": "Receipt currency aggregation: total_cents returns exact integer cents from decimal strings.\n",
        "receipts.py": '''
            def total_cents(amounts):
                return sum(int(float(amount) * 100) for amount in amounts)
        ''',
        "tests/test_smoke.py": '''
            import unittest
            from receipts import total_cents
            class Smoke(unittest.TestCase):
                def test_integer(self):
                    self.assertEqual(total_cents(["1", "2"]), 300)
        ''',
    }, ["receipts.py", "tests/**"], r'''
        from receipts import total_cents
        for values, expected in [([],0),(["0.29"],29),(["0.57","0.29"],86),
                                  (["1.2","3","0.01"],421),(["1000000.00"],100000000),
                                  (["0.01"] * 1000,1000),(["0","0.00"],0)]:
            before = list(values)
            actual = total_cents(values)
            assert type(actual) is int and actual == expected, (values, actual, expected)
            assert values == before
        print("7 exact-currency cases pass")
    ''',
    ["Exact integer-cent totals across cents, single fractional digit, large and repeated values, and empty input.", "No input mutation or new input-validation requirements; regression coverage added."],
    ["Any specified valid amount loses or gains cents."], "bug")

add("bugfix-v2", "bugfix",
    "In /workspace/task, fix settings.resolve so an explicitly supplied false, zero, or empty string overrides the default rather than being discarded. Only None or a missing key should use the default. Return a new dictionary, leave both inputs untouched, and continue ignoring unknown override keys. Preserve the API and add regression coverage.",
    {
        "README.md": "Configuration overlay; None means no override. Unknown keys are ignored.\n",
        "settings.py": '''
            def resolve(defaults, overrides):
                return {key: overrides.get(key) or value for key, value in defaults.items()}
        ''',
        "tests/test_smoke.py": '''
            import unittest
            from settings import resolve
            class Smoke(unittest.TestCase):
                def test_override(self):
                    self.assertEqual(resolve({"port":8000}, {"port":9000}), {"port":9000})
        ''',
    }, ["settings.py", "tests/**"], r'''
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
    ''',
    ["False/zero/empty string override; None and absent keys fall back; unknown keys ignored.", "Fresh dictionary, no input mutation, meaningful regressions."],
    ["Any required falsy override is discarded or source input changed."], "bug")

add("feature-base", "feature",
    "In /workspace/task, add an optional keyword argument limit=None to search.find. With None, preserve current case-insensitive substring matching and input order. A nonnegative integer returns at most that many matching names, with zero returning an empty list; duplicates remain meaningful. Reject negative integers with ValueError. Other limit types and non-string names are out of scope. Keep inputs untouched and cover the new behavior.",
    {
        "README.md": "find(names, query) returns case-insensitive substring matches in original order.\n",
        "search.py": '''
            def find(names, query):
                needle = query.casefold()
                return [name for name in names if needle in name.casefold()]
        ''',
        "tests/test_smoke.py": '''
            import unittest
            from search import find
            class Smoke(unittest.TestCase):
                def test_search(self):
                    self.assertEqual(find(["Tea", "coffee", "teapot"], "TEA"), ["Tea", "teapot"])
        ''',
    }, ["search.py", "tests/**", "README.md"], r'''
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
    ''',
    ["New optional limit handles None/zero/positive/negative as specified.", "Case-insensitive substring, order, duplicates, empty query/input, unmatched query, and input ownership preserved."],
    ["Required limit cases fail or legacy unlimited search regresses."], "feature")

add("feature-v1", "feature",
    "In /workspace/task, add rollup.by_category(rows). Each row has a string category and an integer amount. Return a new dict mapping categories to summed amounts, ordered by first appearance. Missing category means 'uncategorized'; an explicitly empty category remains empty. Include zero totals, permit negative amounts, return {} for empty input, and do not mutate rows. Inputs otherwise satisfy the contract. Cover this new API without changing total(rows).",
    {
        "README.md": "Simple integer expense rollup; total(rows) sums amount fields.\n",
        "rollup.py": '''
            def total(rows):
                return sum(row["amount"] for row in rows)
        ''',
        "tests/test_smoke.py": '''
            import unittest
            from rollup import total
            class Smoke(unittest.TestCase):
                def test_total(self):
                    self.assertEqual(total([{"amount":2},{"amount":-1}]), 1)
        ''',
    }, ["rollup.py", "tests/**", "README.md"], r'''
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
    ''',
    ["Correct grouping, first-appearance order, missing versus empty category, zero and negative totals.", "No mutation and existing total unchanged; new API covered."],
    ["Incorrect sums or category semantics for specified valid rows; existing total regresses."], "feature")

add("feature-v2", "feature",
    "In /workspace/task, extend the existing linecount.py CLI with --json. Default output must remain exactly 'lines=N' followed by a newline. With --json, emit a JSON object with only the key 'lines' with an integer value, followed by a newline. Count lines from standard input, including a final non-newline-terminated line; empty input counts zero. Keep normal --help behavior and cover both modes.",
    {
        "README.md": "linecount.py counts stdin lines; default output is lines=N plus newline.\n",
        "linecount.py": '''
            import argparse
            import sys
            def main():
                parser = argparse.ArgumentParser()
                parser.parse_args()
                count = sum(1 for _ in sys.stdin)
                print(f"lines={count}")
            if __name__ == "__main__":
                main()
        ''',
        "tests/test_smoke.py": '''
            import subprocess
            import sys
            import unittest
            class Smoke(unittest.TestCase):
                def test_default(self):
                    result = subprocess.run([sys.executable,"-B","linecount.py"], input="a\\nb\\n", text=True, capture_output=True)
                    self.assertEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, "lines=2\\n")
        ''',
    }, ["linecount.py", "tests/**", "README.md"], r'''
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
    ''',
    ["JSON mode has exactly integer lines field and trailing newline; default text byte-compatible.", "Empty, final partial, and blank lines counted correctly; help remains usable."],
    ["New mode cannot be used or default mode changes; incorrect required counts."], "feature")

add("refactor-base", "refactor",
    "In /workspace/task, remove the duplicated receipt-line formatting from receipts.text_receipt and receipts.html_receipt by extracting one shared private formatter that both use. Preserve public signatures and exact returned strings, including HTML escaping, item order, empty receipts, negative cents, and singular/plural behavior. Do not fix or redesign observable behavior. Add suitable parity coverage.",
    {
        "README.md": "Receipt rendering accepts a list of (name, quantity, cents) tuples. Names are strings; quantity/cents integers. Existing output is the compatibility contract.\n",
        "receipts.py": '''
            import html
            def text_receipt(items):
                lines = []
                for name, qty, cents in items:
                    unit = "item" if qty == 1 else "items"
                    line = f"{name}: {qty} {unit} @ {cents}c"
                    lines.append(line)
                return "\\n".join(lines)

            def html_receipt(items):
                lines = []
                for name, qty, cents in items:
                    unit = "item" if qty == 1 else "items"
                    line = f"{name}: {qty} {unit} @ {cents}c"
                    lines.append("<li>" + html.escape(line) + "</li>")
                return "<ul>" + "".join(lines) + "</ul>"
        ''',
        "tests/test_smoke.py": '''
            import unittest
            from receipts import text_receipt
            class Smoke(unittest.TestCase):
                def test_text(self):
                    self.assertEqual(text_receipt([("tea",1,20)]), "tea: 1 item @ 20c")
        ''',
    }, ["receipts.py", "tests/**"], r'''
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
    ''',
    ["Both public functions actually use a single private line formatter; duplicated semantic formatting removed.", "Exact strings, escaping, order, empty, quantity and cents behavior and signatures preserved; parity tests added."],
    ["Any supplied valid input's output changes, especially lost HTML escaping.", "No substantive requested shared extraction."], "refactor")

add("refactor-v1", "refactor",
    "In /workspace/task, move the filename classification rules out of inventory.summarize into a private classify_name(name) helper in a new classification.py module, imported by inventory.py (the helper is internal, not a promised public API). Keep summarize's signature and results identical, including case sensitivity, last-suffix behavior, unknown extensions, and order. Add parity coverage; do not expand supported types.",
    {
        "README.md": "summarize(names) counts text (.txt/.md), data (.csv/.json), or other filenames; dictionary order reflects first category appearance.\n",
        "inventory.py": '''
            def summarize(names):
                result = {}
                for name in names:
                    suffix = name.rsplit(".", 1)[-1] if "." in name else ""
                    if suffix in ("txt", "md"):
                        category = "text"
                    elif suffix in ("csv", "json"):
                        category = "data"
                    else:
                        category = "other"
                    result[category] = result.get(category, 0) + 1
                return result
        ''',
        "tests/test_smoke.py": '''
            import unittest
            from inventory import summarize
            class Smoke(unittest.TestCase):
                def test_count(self):
                    self.assertEqual(summarize(["a.txt","b.csv"]), {"text":1,"data":1})
        ''',
    }, ["inventory.py", "classification.py", "tests/**"], r'''
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
    ''',
    ["Classification rules live once in new internal module helper, used by summarize; aggregation stays in inventory.", "Case-sensitive last suffix, unknowns, order, counts, signature, input ownership all preserved."],
    ["Observable summarize output changes or extraction is absent/disconnected."], "refactor")

add("refactor-v2", "refactor",
    "In /workspace/task, simplify access.decide by replacing its nested conditionals with clear guard clauses. Preserve its signature and all current decisions and reason strings for every combination of the four boolean arguments. Keep access precedence unchanged, and add parity coverage. No new authorization policy is requested.",
    {
        "README.md": "decide(active, locked, admin, owner) returns (allowed, reason). Inputs are booleans. Existing policy is authoritative.\n",
        "access.py": '''
            def decide(active, locked, admin, owner):
                if active:
                    if not locked:
                        if admin:
                            return True, "admin"
                        else:
                            if owner:
                                return True, "owner"
                            else:
                                return False, "not-owner"
                    else:
                        return False, "locked"
                else:
                    return False, "inactive"
        ''',
        "tests/test_smoke.py": '''
            import unittest
            from access import decide
            class Smoke(unittest.TestCase):
                def test_admin(self):
                    self.assertEqual(decide(True,False,True,False), (True,"admin"))
        ''',
    }, ["access.py", "tests/**"], r'''
        import inspect, itertools
        from access import decide
        assert str(inspect.signature(decide)) == "(active, locked, admin, owner)"
        for active,locked,admin,owner in itertools.product((False,True), repeat=4):
            expected = ((False,"inactive") if not active else (False,"locked") if locked else
                        (True,"admin") if admin else (True,"owner") if owner else (False,"not-owner"))
            actual = decide(active,locked,admin,owner)
            assert actual == expected and type(actual[0]) is bool
        print("all 16 authorization combinations and exact reasons preserved; panel checks guard-clause structure")
    ''',
    ["Nested decision tree replaced by clear guard clauses, not merely renamed or reformatted.", "All 16 decisions, precedence, reason strings, bool results, and signature preserved."],
    ["Any authorization decision/reason changes or no substantive guard-clause refactor."], "refactor")

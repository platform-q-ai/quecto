#!/usr/bin/env python3
"""Local fixture authoring checks ONLY. No agents/containers are launched.
Confirms initial smoke state, executable oracle syntax, intended baseline failures,
and verifier plumbing. This does not validate worker quality or candidate workflows.
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

from catalog import TASKS

HERE = Path(__file__).resolve().parent


def main():
    results = []
    with tempfile.TemporaryDirectory(prefix="workflow-design-") as tmp:
        temp = Path(tmp)
        for id, task in TASKS.items():
            for name, content in task["files"].items():
                if name.endswith(".py"):
                    compile(content, id + "/" + name, "exec")
            compile(task["oracle"], id + "/oracle", "exec")
            root = temp / id
            setup = temp / (id + "-setup.json")
            verify = temp / (id + "-verify.json")
            for action, output in [("setup", setup), ("verify", verify)]:
                subprocess.run([sys.executable, "-B", str(HERE / "fixture_tool.py"), action,
                                "--task", id, "--root", str(root), "--output", str(output)],
                               check=True, capture_output=True, text=True, timeout=60)
            report = json.loads(verify.read_text())
            assert not report["changed"] and not report["unexpected"], report
            observed = report["verification"]["oracle"]["returncode"]
            should_pass = task["kind"] == "read-only" or id in ("refactor-base", "refactor-v2")
            # refactor-v1's behavior already works, but oracle also imports the requested new module.
            assert (observed == 0) == should_pass, (id, report["verification"])
            smoke = report["verification"].get("worker_suite")
            if smoke:
                assert smoke["returncode"] == 0, (id, smoke)
            results.append({"task":id,"initial_oracle_returncode":observed,
                            "expected_initial_oracle_pass":should_pass,
                            "initial_smoke_returncode":smoke["returncode"] if smoke else None,
                            "oracle_stdout":report["verification"]["oracle"]["stdout"],
                            "oracle_stderr":report["verification"]["oracle"]["stderr"],
                            "catalog_sha256":report["catalog_sha256"]})
    print(json.dumps({"scope":"local authoring checks only; zero worker or panel runs",
                      "results":results},indent=2))


if __name__ == "__main__":
    main()

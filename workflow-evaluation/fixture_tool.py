#!/usr/bin/env python3
"""Evaluator-only fixture setup/snapshot/verification. Never copied to workers.
No agent launch code. Run untrusted worker code ONLY in a disposable sandbox.
"""
import argparse
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from catalog import TASKS


def sha(data):
    return hashlib.sha256(data).hexdigest()


def ignored(path):
    return "__pycache__" in path.parts or path.suffix == ".pyc"


def snapshot(root):
    result = {}
    for path in sorted(root.rglob("*")):
        rel = path.relative_to(root)
        if ignored(rel):
            continue
        if path.is_symlink():
            result[rel.as_posix()] = {"type": "symlink", "target": os.readlink(path)}
        elif path.is_file():
            result[rel.as_posix()] = {"type": "file", "sha256": sha(path.read_bytes())}
    return result


def catalog_hash(task):
    return sha(json.dumps(task, sort_keys=True, ensure_ascii=True).encode())


def outside(path, root):
    if path == root or root in path.parents:
        raise SystemExit("Evaluator metadata/output must be outside the worker fixture")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["list", "setup", "snapshot", "verify"])
    parser.add_argument("--task", choices=sorted(TASKS))
    parser.add_argument("--root", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.action == "list":
        print("\n".join(TASKS))
        return
    if not args.root or not args.output:
        parser.error("--root and --output are required")
    root, output = args.root.resolve(), args.output.resolve()
    outside(output, root)
    if output.exists():
        raise SystemExit("Refusing to overwrite evaluator output")
    if args.action != "snapshot" and not args.task:
        parser.error("--task is required for setup/verify")
    task = TASKS.get(args.task)
    result = {"root": str(root), "task": args.task}
    if args.action == "setup":
        if root.exists():
            raise SystemExit("Refusing an existing fixture root; each run needs a fresh directory")
        root.mkdir(parents=True)
        for relative, content in task["files"].items():
            p = root / relative
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(content.encode("utf-8"))
        result.update(catalog_sha256=catalog_hash(task), files=snapshot(root), prompt=task["prompt"])
    elif args.action == "snapshot":
        if not root.is_dir():
            raise SystemExit("Missing fixture root")
        result["files"] = snapshot(root)
    else:
        if not root.is_dir():
            raise SystemExit("Missing fixture root")
        actual = snapshot(root)
        expected = {p: {"type": "file", "sha256": sha(c.encode())} for p, c in task["files"].items()}
        changed = sorted(p for p in expected.keys() | actual.keys() if expected.get(p) != actual.get(p))
        unexpected = [p for p in changed if not any(fnmatch.fnmatchcase(p, g) for g in task["editable"])]
        # Existing tests are never editable for acceptance purposes: worker may add tests.
        # A legitimate test edit can be discussed by judges, but must not silently pass preservation.
        protected_tests = [p for p in changed if p.startswith("tests/") and p in expected]
        symlinks = [p for p,v in actual.items() if v["type"] == "symlink"]
        result.update(catalog_sha256=catalog_hash(task), files=actual, changed=changed,
                      unexpected=unexpected, changed_existing_tests=protected_tests, symlinks=symlinks,
                      acceptance=task["acceptance"], critical=task["critical"])
        if symlinks:
            result["verification"] = {"blocked": "Symlinks require containment review before execution"}
        else:
            # Keep verifier writes away from the archived worker tree. This is not a security sandbox.
            with tempfile.TemporaryDirectory(prefix="workflow-eval-") as tmp:
                stage = Path(tmp) / "task"
                shutil.copytree(root, stage, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
                env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1", PYTHONHASHSEED="0", TZ="UTC")
                env.pop("PYTHONPATH", None)
                def run(command):
                    try:
                        p = subprocess.run(command, cwd=stage, env=env, capture_output=True,
                                           text=True, timeout=30)
                        return {"command": command, "returncode": p.returncode,
                                "stdout": p.stdout, "stderr": p.stderr}
                    except subprocess.TimeoutExpired as exc:
                        return {"command": command, "timeout_seconds":30,
                                "stdout":str(exc.stdout), "stderr":str(exc.stderr)}
                result["verification"] = {"oracle": run([sys.executable, "-B", "-c", task["oracle"]])}
                if (stage / "tests").is_dir():
                    result["verification"]["worker_suite"] = run(
                        [sys.executable, "-B", "-m", "unittest", "discover", "-s", "tests", "-v"])
        result["notice"] = "Verifier observations and unexpected-path flags are evidence, not automatic scores/gates. Documentation parser mismatches use frozen semantic direct review."
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(output)


if __name__ == "__main__":
    main()

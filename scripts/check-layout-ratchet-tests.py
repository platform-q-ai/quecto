#!/usr/bin/env python3
"""Focused merge-base gate regressions; all repositories are disposable."""
import os
import pathlib
import signal
import subprocess
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).with_name("check-layout-ratchet.sh").resolve()
TABLE = pathlib.Path("quecto-agentic-harness/tests/architecture/layout.rs")


def run(args, cwd, env=None):
    with subprocess.Popen(args, cwd=cwd, env=env, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True,
                          start_new_session=True) as process:
        try:
            stdout, stderr = process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
            raise
        return subprocess.CompletedProcess(args, process.returncode, stdout, stderr)


def table(rows):
    entries = "\n".join(
        f'    FlatBudget {{ path: "{path}", maximum: {count} }},'
        for path, count in rows
    )
    return "const BUDGETS: &[FlatBudget<'_>] = &[\n" + entries + "\n];\n"


class LayoutRatchetGate(unittest.TestCase):
    def check(self, rows, expected, diagnostic=None, base_rows=None, field="maximum"):
        with tempfile.TemporaryDirectory(prefix="layout-ratchet-") as directory:
            root = pathlib.Path(directory)
            for args in (["git", "init", "-b", "master"],
                         ["git", "config", "user.email", "fixture@example.invalid"],
                         ["git", "config", "user.name", "Fixture"]):
                result = run(args, root)
                self.assertEqual(result.returncode, 0, result.stderr)
            source = root / TABLE
            if base_rows is not None:
                source.parent.mkdir(parents=True)
                source.write_text(table(base_rows))
            (root / "fixture").write_text("base")
            for args in (["git", "add", "."], ["git", "commit", "-m", "base"]):
                self.assertEqual(run(args, root).returncode, 0)
            base = run(["git", "rev-parse", "HEAD"], root).stdout.strip()
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(table(rows).replace("maximum:", f"{field}:"))
            result = run(["bash", str(SCRIPT), base], root)
            self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
            if diagnostic:
                self.assertIn(diagnostic, result.stderr)

    def test_unchanged(self):
        self.check([("domain", 2)], 0, base_rows=[("domain", 2)])

    def test_raise_rejected(self):
        self.check([("domain", 3)], 1, "domain", [("domain", 2)])

    def test_added_row_rejected(self):
        self.check([("domain", 2), ("application", 1)], 1, "application", [("domain", 2)])

    def test_lower_and_remove_allowed(self):
        self.check([("domain", 1)], 0, base_rows=[("domain", 2), ("application", 1)])

    def test_all_rows_removed_allowed(self):
        self.check([], 0, base_rows=[("domain", 2)])

    def test_introduction_without_base_table_allowed(self):
        self.check([("domain", 2)], 0)

    def test_exact_count_column_rename(self):
        self.check([("domain", 2)], 0, base_rows=[("domain", 2)], field="expected")

    def test_git_tree_failure_rejected(self):
        with tempfile.TemporaryDirectory(prefix="layout-git-failure-") as directory:
            root = pathlib.Path(directory)
            source = root / TABLE
            source.parent.mkdir(parents=True)
            source.write_text(table([("domain", 2)]))
            git = root / "git"
            git.write_text("#!/bin/sh\ncase \"$1\" in\n"
                           "rev-parse) echo fake-base ;;\n"
                           "*) echo 'simulated Git failure' >&2; exit 42 ;;\nesac\n")
            git.chmod(0o755)
            environment = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}")
            result = run(["bash", str(SCRIPT), "base"], root, environment)
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("layout ratchet:", result.stderr)


if __name__ == "__main__":
    unittest.main()

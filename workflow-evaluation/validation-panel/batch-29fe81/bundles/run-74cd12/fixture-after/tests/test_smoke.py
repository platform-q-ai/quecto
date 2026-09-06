import json
from pathlib import Path
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "linecount.py"


class Smoke(unittest.TestCase):
    def run_cli(self, *args, input=""):
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPT), *args],
            input=input, text=True, capture_output=True,
        )

    def test_counts_in_both_modes(self):
        for text, count in [("", 0), ("a\nb\n", 2), ("a\nb", 2), ("a", 1), ("\n\n", 2)]:
            for args in [(), ("--json",)]:
                with self.subTest(text=text, args=args):
                    result = self.run_cli(*args, input=text)
                    self.assertEqual(result.returncode, 0)
                    self.assertEqual(result.stderr, "")
                    if args:
                        self.assertTrue(result.stdout.endswith("\n"))
                        self.assertEqual(len(result.stdout.splitlines()), 1)
                        payload = json.loads(result.stdout)
                        self.assertEqual(payload, {"lines": count})
                        self.assertIs(type(payload["lines"]), int)
                    else:
                        self.assertEqual(result.stdout, f"lines={count}\n")

    def test_help_in_both_modes(self):
        for args in [("--help",), ("--json", "--help")]:
            with self.subTest(args=args):
                result = self.run_cli(*args, input="a\nb\n")
                self.assertEqual(result.returncode, 0)
                self.assertEqual(result.stderr, "")
                self.assertIn("usage:", result.stdout)
                self.assertIn("--json", result.stdout)
                self.assertNotIn("lines=", result.stdout)

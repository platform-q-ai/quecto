"""Presence-only contracts; fixtures never contain developer credentials."""
import json
import os
import tempfile
from pathlib import Path
from test_process import run_fixture
from subprocess import TimeoutExpired
import unittest

WRAPPER = Path(__file__).with_name("build-environment.sh")


class BuildEnvironmentTests(unittest.TestCase):
    def test_build_and_server_receive_only_explicit_allowed_names(self):
        environment = {
            "PATH": "/usr/bin:/bin",
            "CARGO_HOME": "/tmp/cargo-fixture",
            "SCCACHE_DIR": "/tmp/sccache-fixture",
            "SCCACHE_CACHE_SIZE": "2G",
            "RUSTUP_HOME": "/opt/rustup",
            "ANTHROPIC_API_KEY": "fixture",
            "OPENAI_API_KEY": "fixture",
            "GH_TOKEN": "fixture",
            "UNRECOGNISED_PROVIDER_CREDENTIAL": "fixture",
            "QUECTO_SWARM_BOOTSTRAP": "fixture",
        }
        # Inspect names only, never emit or persist environment values.
        probe = "import json,os; print(json.dumps(sorted(os.environ)))"
        result = run_fixture(
            [str(WRAPPER), "/usr/bin/python3", "-c", probe],
            env=environment, check=True, capture_output=True, text=True,
        )
        actual = set(json.loads(result.stdout))
        self.assertTrue(actual <= {
            "PATH", "CARGO_HOME", "SCCACHE_DIR", "SCCACHE_CACHE_SIZE",
            "RUSTUP_HOME", "LC_CTYPE", "TERM",
        }, actual)
        self.assertTrue({"CARGO_HOME", "SCCACHE_DIR", "SCCACHE_CACHE_SIZE"} <= actual)

    def test_shell_startup_cannot_inherit_bash_env(self):
        with tempfile.TemporaryDirectory() as root:
            sentinel = Path(root) / "startup-was-sourced"
            startup = Path(root) / "startup"
            startup.write_text(f"touch '{sentinel}'\n")
            environment = {
                "PATH": "/usr/bin:/bin", "BASH_ENV": str(startup),
                "BASH_FUNC_fixture%%": "() { :; }", "SHELLOPTS": "xtrace",
            }
            probe = "import json,os; print(json.dumps(sorted(os.environ)))"
            result = run_fixture(
                [str(WRAPPER), "/usr/bin/python3", "-c", probe],
                env=environment, check=True, capture_output=True, text=True,
            )
            actual = set(json.loads(result.stdout))
            self.assertTrue(actual <= {"PATH", "LC_CTYPE", "TERM"}, actual)
            self.assertFalse(sentinel.exists())
            self.assertEqual(result.stderr, "")

    def test_fixture_deadline_kills_descendant_process_group(self):
        with self.assertRaises(TimeoutExpired):
            run_fixture(
                ["/bin/sh", "-c", "sleep 60 & wait"],
                env={"PATH": "/usr/bin:/bin"}, timeout=0.05,
            )

    def test_missing_command_fails_closed(self):
        result = run_fixture(
            [str(WRAPPER)], env={"PATH": "/usr/bin:/bin"},
            capture_output=True,
        )
        self.assertEqual(result.returncode, 2)

    def test_missing_executable_fails_closed(self):
        result = run_fixture(
            [str(WRAPPER), "/quecto-fixture-absent-command"],
            env={"PATH": "/usr/bin:/bin"}, capture_output=True,
        )
        self.assertEqual(result.returncode, 127)

    def test_failed_child_status_is_preserved(self):
        result = run_fixture(
            [str(WRAPPER), "/bin/sh", "-c", "exit 23"],
            env={"PATH": "/usr/bin:/bin"}, capture_output=True,
        )
        self.assertEqual(result.returncode, 23)


if __name__ == "__main__":
    unittest.main()

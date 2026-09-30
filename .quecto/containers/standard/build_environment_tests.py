"""Presence-only contracts; fixtures never contain developer credentials."""
import json
import os
import tempfile
import shutil
import shlex
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

    def test_real_offline_compilation_preserves_generated_metadata_not_entry_auth(self):
        cargo = shutil.which("cargo")
        rustc = shutil.which("rustc")
        self.assertIsNotNone(cargo, "real Cargo is required for this regression")
        self.assertIsNotNone(rustc, "real rustc is required for this regression")
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            tools = root / "tools"
            tools.mkdir()
            # Match the image's wrappers, including the rustc re-entry boundary.
            for name, executable in (("cargo", cargo), ("rustc", rustc)):
                wrapper = tools / name
                wrapper.write_text(
                    "#!/bin/bash -p\nexec " + shlex.quote(str(WRAPPER.resolve()))
                    + " " + shlex.quote(executable) + ' "$@"\n'
                )
                wrapper.chmod(0o755)
            (root / "src").mkdir()
            (root / "Cargo.toml").write_text(
                '[package]\nname="metadata-fixture"\nversion="0.1.2"\n'
                'edition="2021"\nbuild="build.rs"\n'
            )
            (root / "build.rs").write_text('''fn main() {
    assert_eq!(env!("CARGO_PKG_NAME"), "metadata-fixture");
    for name in ["CARGO_REGISTRIES_FIXTURE_TOKEN", "CARGO_REGISTRY_TOKEN",
                 "OPENAI_API_KEY", "UNRECOGNISED_PROVIDER_CREDENTIAL"] {
        assert!(std::env::var_os(name).is_none(), "credential name leaked");
    }
    let out = std::env::var("OUT_DIR").unwrap();
    std::fs::write(std::path::Path::new(&out).join("generated.rs"),
                   "pub const GENERATED: &str = \\\"generated\\\";").unwrap();
}
''')
            (root / "src/main.rs").write_text('''include!(concat!(env!("OUT_DIR"), "/generated.rs"));
fn main() {
    assert_eq!(env!("CARGO_PKG_NAME"), "metadata-fixture");
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.1.2");
    assert_eq!(GENERATED, "generated");
}
''')
            environment = {
                "PATH": str(tools) + os.pathsep + os.environ["PATH"],
                "HOME": str(root), "CARGO_HOME": str(root / "cargo-home"),
                "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", "/opt/rustup"),
                "CARGO_PKG_NAME": "forged-entry-name", "OUT_DIR": "forged-entry-dir",
                "CARGO_REGISTRIES_FIXTURE_TOKEN": "fixture",
                "CARGO_REGISTRY_TOKEN": "fixture", "OPENAI_API_KEY": "fixture",
                "UNRECOGNISED_PROVIDER_CREDENTIAL": "fixture",
            }
            result = run_fixture(
                [str(tools / "cargo"), "run", "--offline", "--manifest-path",
                 str(root / "Cargo.toml")], env=environment, timeout=30, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

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

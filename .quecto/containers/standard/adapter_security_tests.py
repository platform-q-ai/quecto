"""Standard-only confinement and authenticated-launch tests (no runtime/build)."""
import json
import os
from pathlib import Path
from test_process import run_fixture
import tempfile
import unittest

ADAPTER = Path(__file__).with_name("scripts") / "create.sh"


class AdapterSecurityTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.root = Path(self.scratch.name)
        self.base = self.root / "base"
        self.base.mkdir(mode=0o700)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.home = self.root / "home"
        (self.home / ".quecto").mkdir(parents=True)
        (self.home / ".gitconfig").write_text("[user]\nname = Fixture\nemail = fixture@example.invalid\n")
        self.names = self.root / "names.json"
        self.mounts = self.root / "mounts"
        self.probe = self.bin / "probe"
        self.executable(self.probe, "#!/bin/bash -p\n"
                        f"exec /usr/bin/python3 -c 'import json,os; json.dump(sorted(os.environ),open(\"{self.names}\",\"w\"))'\n")
        self.executable(self.bin / "gh", "#!/bin/sh\nprintf '%s\\n' fixture\n")
        self.executable(self.bin / "podman", f'''#!/bin/bash -p
set -eu
if [ "$1" = image ]; then
  case "$2" in exists) exit 0;; inspect) printf '\\n'; exit 0;; esac
fi
if [ "$1" = run ] && [ "$2" = --rm ]; then exit 0; fi
if [ "$1" = run ]; then
  shift
  runtime_env=(PATH=/usr/bin:/bin UNRECOGNISED_IMAGE_CREDENTIAL=fixture)
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -v|--volume) printf '%s\\n' "$2" >> '{self.mounts}'; shift 2;;
      -e|--env) runtime_env+=("$2"); shift 2;;
      --name|--hostname|--label|--user|-w|--workdir|--pids-limit) shift 2;;
      --*) shift;;
      -d) shift;;
      fixture-image) shift; break;;
      *) exit 125;;
    esac
  done
  env -i "${{runtime_env[@]}}" "$@"
  printf 'fixture-container\\n'
  exit 0
fi
exit 125
''')

    def tearDown(self):
        self.scratch.cleanup()

    @staticmethod
    def executable(path, content):
        path.write_text(content)
        path.chmod(0o700)

    def launch(self, base=None, override=None, agent=False):
        base = base or self.base
        environment = {
            "PATH": f"{self.bin}:/usr/bin:/bin", "HOME": str(self.home),
            "QUECTO_BASE_DIR": str(override or base),
            "QUECTO_CONTAINER_CLI": "podman",
            "QUECTO_CONTAINER_ENVIRONMENT_REF": "standard",
            "QUECTO_CONTAINER_PROBE_TIMEOUT": "1",
            "ANTHROPIC_API_KEY": "fixture", "OPENAI_API_KEY": "fixture",
            "UNRECOGNISED_PROVIDER_CREDENTIAL": "fixture",
        }
        argv = [str(ADAPTER), "--state-dir", str(base / "container-environments"),
                "--image", "fixture-image", "--", str(self.probe)]
        if agent:
            argv += ["agent", "--mode", "uds"]
        argv += ["--socket", str(self.root / "child.sock")]
        return run_fixture(argv, env=environment, capture_output=True, timeout=15)

    def cache_root(self):
        root = self.base / "rust-cache"
        root.mkdir(mode=0o700)
        return root

    def test_legitimate_base_symlink_resolves_to_authoritative_parent(self):
        alias = self.root / "alias"
        alias.symlink_to(self.base, target_is_directory=True)
        result = self.launch(base=alias)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(all(str(self.base / "rust-cache") in mount
                            for mount in self.mounts.read_text().splitlines()
                            if ":/tmp/quecto-" in mount))

    def test_override_cannot_redirect_cache_authority(self):
        unrelated = self.root / "unrelated"
        unrelated.mkdir()
        result = self.launch(override=unrelated)
        self.assertEqual(result.returncode, 8)
        self.assertFalse((unrelated / "rust-cache").exists())

    def test_cache_symlink_is_refused(self):
        outside = self.root / "outside"
        outside.mkdir(mode=0o700)
        (self.base / "rust-cache").symlink_to(outside, target_is_directory=True)
        self.assertEqual(self.launch().returncode, 8)
        self.assertEqual(list(outside.iterdir()), [])

    def test_child_cache_symlink_is_refused(self):
        root = self.cache_root()
        outside = self.root / "outside"
        outside.mkdir(mode=0o700)
        (root / "sccache").symlink_to(outside, target_is_directory=True)
        self.assertEqual(self.launch().returncode, 8)
        self.assertEqual(list(outside.iterdir()), [])

    def test_hidden_and_visible_configuration_is_refused_without_contents(self):
        cargo = self.cache_root() / "cargo"
        cargo.mkdir(mode=0o700)
        for name in ("config.toml", ".config.toml", "credentials.toml", ".credentials"):
            with self.subTest(name=name):
                entry = cargo / name
                entry.write_text("fixture-content-must-stay-private")
                result = self.launch()
                self.assertEqual(result.returncode, 8)
                self.assertIn(b"non-allowlisted entry", result.stderr)
                self.assertEqual((result.stdout + result.stderr).count(
                    b"fixture-content-must-stay-private"), 0)
                entry.unlink()

    def test_insecure_existing_cache_is_refused_without_chmod(self):
        root = self.cache_root()
        root.chmod(0o755)
        self.assertEqual(self.launch().returncode, 8)
        self.assertEqual(root.stat().st_mode & 0o777, 0o755)

    def test_cargo_configuration_is_refused(self):
        cargo = self.cache_root() / "cargo"
        cargo.mkdir(mode=0o700)
        (cargo / "credentials.toml").write_text("fixture")
        self.assertEqual(self.launch().returncode, 8)

    def test_allowed_entry_symlink_escape_is_refused(self):
        cargo = self.cache_root() / "cargo"
        cargo.mkdir(mode=0o700)
        (cargo / "registry").symlink_to(self.root, target_is_directory=True)
        self.assertEqual(self.launch().returncode, 8)

    def test_build_child_only_receives_allowlisted_names(self):
        result = self.launch()
        self.assertEqual(result.returncode, 0, result.stderr)
        names = set(json.loads(self.names.read_text()))
        boundary = ADAPTER.parent.parent / "build-environment.sh"
        self.assertIn(f"{boundary.resolve()}:{boundary.resolve()}:ro",
                      self.mounts.read_text().splitlines())
        self.assertTrue(names <= {"PATH", "HOME", "CARGO_HOME", "SCCACHE_DIR",
                                 "SCCACHE_CACHE_SIZE", "RUSTC_WRAPPER", "LC_CTYPE", "PWD", "SHLVL", "TERM"}, names)

    def test_supported_agent_protocol_preserves_authentication(self):
        result = self.launch(agent=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        names = set(json.loads(self.names.read_text()))
        self.assertTrue({"ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GH_TOKEN",
                         "GITHUB_TOKEN", "QUECTO_SWARM_BOOTSTRAP"} <= names)
        allowed = {
            "PATH", "HOME", "CARGO_HOME", "SCCACHE_DIR", "SCCACHE_CACHE_SIZE",
            "RUSTC_WRAPPER", "LC_CTYPE", "PWD", "SHLVL", "RUST_LOG",
            "ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GH_TOKEN", "GITHUB_TOKEN",
            "QUECTO_SWARM_BOOTSTRAP", "QUECTO_SWARM_CONTAINER",
            "QUECTO_SWARM_HOST_PID_NS", "QUECTO_SWARM_CHECKOUT", "GIT_CONFIG_COUNT",
            "UNRECOGNISED_IMAGE_CREDENTIAL",
        }
        for index in range(3):
            allowed.update({f"GIT_CONFIG_KEY_{index}", f"GIT_CONFIG_VALUE_{index}"})
        self.assertTrue(names <= allowed, names)


if __name__ == "__main__":
    unittest.main()

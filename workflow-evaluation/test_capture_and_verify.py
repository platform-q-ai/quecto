"""Host-only MOCK tests: no container/worker interaction and no worker code execution."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('capture_impl',HERE/'capture_and_verify.py')
module=importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CaptureTests(unittest.TestCase):
    def test_reuses_captures_runs_only_container_checks_and_replays(self):
        with tempfile.TemporaryDirectory() as tmp:
            run=Path(tmp)/'run';run.mkdir()
            (run/'provision.json').write_text(json.dumps({'runtime':'podman','container':'mock-only',
                                                        'socket':'/unused','task':'bugfix-base'}))
            for label in ('fixture-before','fixture-after'):
                tree=run/label;(tree/'tests').mkdir(parents=True)
                (tree/'intervals.py').write_text('# inert mock fixture, never executed\n')
                (tree/'tests/test_smoke.py').write_text('# inert\n')
            (run/'fixture-after/tests/test_added.py').write_text('# added inert regression\n')
            tr=run/'transcript';tr.mkdir()
            (tr/'messages.json').write_text('[]')
            (tr/'archive-summary.json').write_text('{"pages": 1, "warnings": []}')
            before_files={str(p.relative_to(run)):p.read_bytes() for p in run.rglob('*') if p.is_file()}
            calls=[]
            def fake(argv, **kwargs):
                calls.append(argv)
                self.assertEqual(argv[0],'podman') # no archive call or host python worker execution
                if 'mktemp' in argv:
                    return subprocess.CompletedProcess(argv,0,b'/tmp/workflow-eval.MOCK\n',b'')
                # A failing oracle is evidence, not capture-infrastructure failure.
                rc=1 if '-c' in argv else 0
                return subprocess.CompletedProcess(argv,rc,b'mock output\n',b'')
            with patch.object(sys,'argv',['capture',str(run)]), patch.object(module.subprocess,'run',side_effect=fake),contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(module.main(),0)
            out=next(run.glob('capture-*'))
            result=json.loads((out/'status.json').read_text())
            self.assertEqual(result['status'],'CAPTURED_FOR_REVIEW')
            self.assertEqual(len(result['reused']),2)
            self.assertFalse(result['container_killed'])
            self.assertTrue((out/'pristine-with-worker-tests.json').is_file())
            self.assertEqual(json.loads((out/'final-oracle.json').read_text())['returncode'],1)
            self.assertFalse(any('kill' in c or 'rm' in c for c in calls))
            for p,b in before_files.items():self.assertEqual((run/p).read_bytes(),b)

    def test_snapshot_does_not_follow_symlinks(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'plain').write_text('text');(root/'link').symlink_to(root/'plain')
            snap=module.snapshot(root)
            self.assertEqual(snap['link']['type'],'symlink')


if __name__=='__main__':
    unittest.main()

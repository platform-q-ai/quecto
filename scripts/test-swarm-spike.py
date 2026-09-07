#!/usr/bin/env python3
"""Deterministic real-harness/fake-provider integration; run inside container.
Usage: python3 scripts/test-swarm-spike.py /absolute/path/to/quecto-swarm-spike
"""
import importlib.util
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
from http.server import ThreadingHTTPServer

root = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('fake', root / 'scripts/swarm-spike-fake-provider.py')
fake = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fake)
server = ThreadingHTTPServer(('127.0.0.1', 0), fake.Handler)
server.delay_seconds = 0
threading.Thread(target=server.serve_forever, daemon=True).start()
binary = str(Path(sys.argv[1]).resolve())
try:
    with tempfile.TemporaryDirectory(prefix='swarm-e2e-') as directory:
        directory = Path(directory)
        config = json.loads((root / 'notes/spikes/1680/fake-provider-config.json').read_text())
        config['providers']['openai_compatible']['endpoints'][0]['api_base'] = f'http://127.0.0.1:{server.server_port}/v1'
        path = directory / 'config.json'
        path.write_text(json.dumps(config))
        env = dict(os.environ, QUECTO_BASE_DIR=str(directory / 'config'))
        env.pop('QUECTO_SWARM_SPIKE', None)
        def run(*args, **kwargs):
            return subprocess.run([binary, *map(str, args)], env=env, capture_output=True, text=True, timeout=60, **kwargs)
        for size in ('0', '11'):
            result = run('live', directory / 'invalid.sqlite', path, size, '30')
            assert result.returncode and not (directory / 'invalid.sqlite').exists(), result
        nested = dict(env, QUECTO_SWARM_SPIKE='1')
        result = subprocess.run([binary, 'inspect', str(directory / 'none.sqlite')], env=nested, capture_output=True, text=True)
        assert result.returncode and 'cannot start another runner' in result.stderr
        db = directory / 'run.sqlite'
        result = run('live', db, path, '3', '30')
        assert result.returncode == 0, result.stderr + result.stdout
        with sqlite3.connect(db) as c:
            assert c.execute('select status from swarm').fetchone()[0] == 'completed'
            assert c.execute("select count(*) from tasks where status='done'").fetchone()[0] == 5
            assert c.execute('select count(*) from members').fetchone()[0] == 3
            assert c.execute("select count(*) from messages where sender like 'worker-%'").fetchone()[0] >= 5
            evidence = [e for (payload,) in c.execute('select payload from evidence') for e in json.loads(payload)]
            assert sorted((e['n'], e['square']) for e in evidence) == [(n, n*n) for n in range(1,6)]
        # Hold provider requests until cancellation; no sleeps/races with a fast run.
        entered, release = threading.Event(), threading.Event()
        original = fake.reply
        def delayed(body):
            entered.set()
            release.wait(30)
            return original(body)
        fake.reply = delayed
        cancelled = directory / 'cancel.sqlite'
        process = subprocess.Popen([binary, 'live', str(cancelled), str(path), '3', '30'], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            assert entered.wait(20), 'workers never reached provider'
            # A second runner cannot reserve a new pool under another DB identity.
            rejected = run('live', directory / 'overlap.sqlite', path, '3', '30')
            assert rejected.returncode and 'already reserved' in rejected.stderr
            process.send_signal(signal.SIGINT)
            stdout, stderr = process.communicate(timeout=20)
            assert process.returncode != 0, stdout + stderr
            with sqlite3.connect(cancelled) as c:
                assert c.execute('select status from swarm').fetchone()[0] == 'cancelled'
        finally:
            release.set()
            if process.poll() is None:
                process.kill()
                process.wait()
        print('PASS: real CLI workers + fake provider, exact evidence, cap/nesting/overlap rejection, SIGINT settlement')
finally:
    server.shutdown()
    server.server_close()

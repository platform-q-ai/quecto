#!/usr/bin/env python3
"""Parent-only: provision a fixture into an ALREADY IDLE sandbox container.
No spawn or prompt. --workspace is the exact spawn-returned workspace path.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from catalog import TASKS

HERE = Path(__file__).resolve().parent


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--task', required=True, choices=TASKS)
    p.add_argument('--workspace', type=Path, required=True)
    p.add_argument('--run-dir', type=Path, required=True)
    p.add_argument('--runtime', default='podman', choices=['podman','docker'])
    a = p.parse_args()
    env = a.workspace.resolve().parent
    container = (env / 'container').read_text().strip()
    children = [json.loads(line) for line in (env / 'children.jsonl').read_text().splitlines()]
    socket = children[0]['socket']
    run = a.run_dir.resolve()
    run.mkdir(parents=True, exist_ok=False)
    def command(*args):
        return subprocess.run([a.runtime,*args],check=True,text=True,capture_output=True).stdout.strip()
    metadata = {'container':container,'socket':socket,'workspace':str(a.workspace),
                'runtime':a.runtime,'task':a.task,'model':'openai-oauth/gpt-6-astra','effort':'low',
                'image_id':command('inspect','--format','{{.Image}}',container),
                'python_version':command('exec',container,'python3','--version'),
                'metric_limitations':['No enforced token ceiling; report available session usage only.',
                                      'No hard filesystem/network isolation from mounted agent home.']}
    subprocess.run([sys.executable,'-B',str(HERE/'fixture_tool.py'),'setup','--task',a.task,
                    '--root',str(run/'fixture-before'),'--output',str(run/'setup.json')],check=True,capture_output=True)
    uid = command('exec',container,'id','-u')
    gid = command('exec',container,'id','-g')
    command('exec','--user','0',container,'sh','-c',
            'test ! -e /workspace/task && mkdir -p /workspace && chown "$1:$2" /workspace', 'sh',uid,gid)
    command('cp',str(run/'fixture-before'),container+':/workspace/task')
    hashes = command('exec','--workdir','/workspace/task',container,'python3','-B','-c',
        'from pathlib import Path; import hashlib,json; print(json.dumps({str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(Path(".").rglob("*")) if p.is_file()},sort_keys=True))')
    actual = json.loads(hashes)
    expected = json.loads((run/'setup.json').read_text())['files']
    assert actual == {path:entry['sha256'] for path,entry in expected.items()}, 'Fixture copy mismatch'
    (run/'container-fixture-hashes.json').write_text(json.dumps(actual,indent=2)+'\n')
    metadata['fixture_verified_before_prompt'] = True
    metadata['fixture_sha256'] = hashlib.sha256(json.dumps(actual,sort_keys=True).encode()).hexdigest()
    (run/'provision.json').write_text(json.dumps(metadata,indent=2)+'\n')
    prompt = 'Select the built-in '+TASKS[a.task]['workflow']+' workflow for this task.\n\n'+TASKS[a.task]['prompt']
    (run/'task.txt').write_text(prompt+'\n')
    print(json.dumps({'container':container,'socket':socket,'run_dir':str(run),'fixture_verified':True}))


if __name__ == '__main__':
    main()

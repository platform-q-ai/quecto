#!/usr/bin/env python3
"""Parent-operated post-run capture. Never prompts workers or kills containers.
Reuse existing transcript/fixture-after without overwriting. Worker code executes
only in disposable directories in the SAME container. Exit 2 = incomplete capture
or infrastructure error, not a worker verdict; failing checks remain recorded.
"""
import argparse
import difflib
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import uuid

HERE = Path(__file__).resolve().parent


def digest(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def dump(p, obj):
    p.write_text(json.dumps(obj, indent=2, sort_keys=True) + '\n')


def snapshot(root):
    result = {}
    if not root.is_dir():
        raise ValueError('Missing tree: ' + str(root))
    for base, dirs, files in os.walk(root, followlinks=False):
        for name in sorted(dirs + files):
            p = Path(base) / name
            rel = p.relative_to(root).as_posix()
            if p.is_symlink():
                result[rel] = {'type':'symlink', 'target':os.readlink(p)}
            elif p.is_file():
                result[rel] = {'type':'file','sha256':digest(p),'bytes':p.stat().st_size}
            elif not p.is_dir():
                result[rel] = {'type':'special'}
    return result


def ignored(path):
    return '__pycache__' in Path(path).parts or path.endswith('.pyc')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run_dir', type=Path)
    parser.add_argument('--timeout', type=int, default=60)
    args = parser.parse_args()
    run = args.run_dir.resolve()
    provision = json.loads((run/'provision.json').read_text())
    runtime = provision.get('runtime', 'podman')
    if runtime not in ('podman','docker'):
        raise SystemExit('Unsupported runtime')
    container = provision['container']
    # Unique evidence attempt: do not overwrite prior captures or verification.
    out = run / ('capture-' + uuid.uuid4().hex[:12])
    out.mkdir()
    status = {'errors':[], 'limitations':[], 'commands':[], 'container_killed':False,
              'worker_code_executed_on_host':False,'reused':[], 'run_dir':str(run)}
    dump(out/'status.json', status)

    def command(label, argv, timeout=None):
        record = {'command':argv, 'returncode':None, 'timed_out':False}
        try:
            p = subprocess.run(argv, capture_output=True, timeout=timeout or args.timeout)
            record.update(returncode=p.returncode,
                          stdout=p.stdout.decode('utf-8',errors='replace'),
                          stderr=p.stderr.decode('utf-8',errors='replace'))
        except subprocess.TimeoutExpired as e:
            record.update(timed_out=True,stdout=(e.stdout or b'').decode('utf-8',errors='replace'),
                          stderr=(e.stderr or b'').decode('utf-8',errors='replace'))
            status['errors'].append(label+': timeout; container subprocess may remain running')
        except OSError as e:
            record['error'] = str(e)
            status['errors'].append(label+': '+str(e))
        dump(out/(label+'.json'), record)
        status['commands'].append(label+'.json')
        return record

    def require(label, argv):
        result = command(label, argv)
        if result['returncode'] != 0:
            raise RuntimeError(label+' failed; see command record')
        return result

    try:
        frozen = json.loads((HERE/'freeze-manifest.json').read_text())['files_sha256']
        for name in ('catalog.py','RUBRIC.md','archive_session.py'):
            if digest(HERE/name) != frozen[name]:
                raise RuntimeError('Frozen input changed: '+name)
        # Import only evaluator-owned catalog, never a worker path.
        from catalog import TASKS
        task = TASKS[provision['task']]
        transcript = run/'transcript'
        if transcript.exists():
            status['reused'].append('transcript (left unchanged)')
        else:
            result = command('archive', [sys.executable,'-B',str(HERE/'archive_session.py'),
                '--socket',provision['socket'],'--output',str(transcript)], timeout=180)
            if result['returncode'] != 0:
                status['errors'].append('Transcript archival incomplete; retain partial pages')
        if not (transcript/'messages.json').is_file() or not (transcript/'archive-summary.json').is_file():
            status['errors'].append('Existing/new transcript lacks completion files; not overwritten')
        else:
            summary = json.loads((transcript/'archive-summary.json').read_text())
            status['limitations'].extend(summary.get('limitations',[]))
            if summary.get('warnings'):
                status['limitations'].append({'transcript_warnings':summary['warnings']})
        after = run/'fixture-after'
        if after.exists():
            status['reused'].append('fixture-after (left unchanged; first capture authoritative)')
        else:
            tempcopy = out/'copy-in-progress'
            require('copy-fixture', [runtime,'cp',container+':/workspace/task',str(tempcopy)])
            tempcopy.rename(after)
        before = run/'fixture-before'
        original, final = snapshot(before), snapshot(after)
        dump(out/'before-files.json',original)
        dump(out/'after-files.json',final)
        changed = sorted(p for p in original.keys()|final.keys() if original.get(p)!=final.get(p) and not ignored(p))
        flags = [p for p in changed if not any(fnmatch.fnmatchcase(p, pat) for pat in task['editable'])]
        dump(out/'changes.json',{'changed':changed,'path_review_flags_only':flags,
              'changed_existing_tests':[p for p in changed if p.startswith('tests/') and p in original],
              'notice':'Flags are not hidden requirements or automatic gate failures.'})
        diff=[]
        for path in changed:
            a,b=original.get(path),final.get(path)
            if all(v is None or v['type']=='file' for v in (a,b)):
                try:
                    left=(before/path).read_text().splitlines(keepends=True) if a else []
                    right=(after/path).read_text().splitlines(keepends=True) if b else []
                    diff.extend(difflib.unified_diff(left,right,fromfile='before/'+path,tofile='after/'+path))
                except UnicodeError:
                    diff.append('\nBinary change: '+path+'\n')
            else:
                diff.append('\nNonregular path change: '+path+'\n')
        (out/'diff.txt').write_text(''.join(diff))
        dump(out/'acceptance.json',{'acceptance':task['acceptance'],'critical':task['critical'],
             'docs_semantic_fallback':'Frozen documentation parser mismatches need direct semantic review, not automatic failure.'})
        (out/'oracle.py').write_text(task['oracle'])
        shutil.copyfile(HERE/'RUBRIC.md',out/'RUBRIC.md')
        # Never silently label an inferred built-in snapshot as observed assignment.
        assigned = run/'assigned-template.json'
        if assigned.is_file():
            shutil.copyfile(assigned,out/'assigned-template.json')
        else:
            shutil.copyfile(HERE/'templates'/('builtin-'+task['workflow']+'.json'),out/'expected-template.json')
            status['limitations'].append('Expected built-in snapshot only; panel must corroborate actual assignment/guidance from transcript.')
        messages_file=transcript/'messages.json'
        if messages_file.is_file():
            messages=json.loads(messages_file.read_text())
            dump(out/'transcript-index.json',[{'id':m.get('id'),'ordinal':m.get('ordinal'),
                'role':m.get('role'),'toolName':m.get('toolName'),'toolCallId':m.get('toolCallId')} for m in messages])
            assistants=[m for m in messages if m.get('role')=='assistant' and m.get('content')]
            if assistants:
                (out/'worker-final.txt').write_text(str(assistants[-1]['content'])+'\n')
        unsafe=[p for tree in (original,final) for p,v in tree.items() if v['type']!='file']
        if unsafe:
            raise RuntimeError('Symlink/special paths require manual containment review before verification: '+repr(unsafe))
        # Separate copies for each command ensure oracle writes never contaminate suite/replay.
        remote=require('allocate-stage',[runtime,'exec',container,'mktemp','-d','/tmp/workflow-eval.XXXXXXXX'])['stdout'].strip()
        if not remote.startswith('/tmp/workflow-eval.') or '\n' in remote:
            raise RuntimeError('Unexpected remote stage path')
        status['remote_stage']=remote
        status['limitations'].append('Disposable same-container stages retained for parent cleanup. No container kill; timeout may leave subprocesses. Working directory differs from original task path.')

        def stage_check(label, local, argv):
            dest=remote+'/'+label
            require(label+'-copy',[runtime,'cp',str(local),container+':'+dest])
            # env -u prevents accidental host/config PYTHONPATH from affecting imports.
            result=command(label,[runtime,'exec','--workdir',dest,
                '-e','PYTHONDONTWRITEBYTECODE=1','-e','PYTHONHASHSEED=0','-e','TZ=UTC',
                container,'env','-u','PYTHONPATH',*argv])
            if result['returncode'] in (125,126,127):
                status['errors'].append(label+': runtime/command execution error (inspect stderr)')
            return result

        stage_check('final-oracle',after,['python3','-B','-c',task['oracle']])
        suite=['python3','-B','-m','unittest','discover','-s','tests','-v']
        if (after/'tests').is_dir():
            stage_check('final-suite',after,suite)
        else:
            status['limitations'].append('No tests/ directory; unittest discovery not applicable. Other checks require transcript review.')
        # Replay full changed tests/ tree additions on pristine product code; old tests
        # stay pristine. Modified old tests replay separately as final test versions.
        test_changes=[p for p in changed if p.startswith('tests/') and p in final]
        dump(out/'replay-selection.json',{'overlaid_paths':test_changes,
            'method':'Original fixture plus added/modified tests/** only; original deleted tests retained.',
            'limits':'Imports of new helpers outside tests/ may fail for setup reasons. Non-unittest checks and shell snippets require manual replay. Evaluator replay proves sensitivity/parity, NOT worker chronological RED.'})
        if test_changes:
            with tempfile.TemporaryDirectory(prefix='workflow-test-overlay-') as tmp:
                overlay=Path(tmp)/'fixture'
                shutil.copytree(before,overlay)
                for p in test_changes:
                    target=overlay/p;target.parent.mkdir(parents=True,exist_ok=True)
                    shutil.copyfile(after/p,target)
                stage_check('pristine-with-worker-tests',overlay,suite)
        else:
            status['limitations'].append('No added/modified tests/** to replay; meaningful source derivation or checks outside tests/ need panel review.')
        if snapshot(before)!=original or snapshot(after)!=final:
            status['errors'].append('Retained fixture bytes changed during capture; investigate shared-mount access')
    except Exception as e:
        status['errors'].append(type(e).__name__+': '+str(e))
    finally:
        status['status']='INCOMPLETE' if status['errors'] else 'CAPTURED_FOR_REVIEW'
        status['notice']='Not a score or gate verdict. Nonzero assertion/test results are evidence; interpret cause and task contract.'
        dump(out/'status.json',status)
        # Index links immutable captures without copying full fixture trees again.
        entries=[]
        for root in (run/'fixture-before',run/'fixture-after',run/'transcript',out):
            if root.is_dir():
                for base,dirs,files in os.walk(root,followlinks=False):
                    for name in files:
                        p=Path(base)/name
                        if p.is_file() and not p.is_symlink():
                            entries.append({'path':str(p.relative_to(run)),'sha256':digest(p),'bytes':p.stat().st_size})
        for name in ('task.txt','provision.json','setup.json'):
            p=run/name
            if p.is_file():entries.append({'path':name,'sha256':digest(p),'bytes':p.stat().st_size})
        dump(out/'bundle-index.json',{'files':entries,'capture_status':status['status'],
            'read_first':['status.json','RUBRIC.md','acceptance.json','changes.json','diff.txt'],
            'judge_warning':'Compare actual artifact-review evidence, not Git availability. Do not infer behavior from workflow checkmarks.'})
        print(json.dumps({'capture':str(out),'status':status['status'],'errors':status['errors'],'container_killed':False}))
    return 2 if status['errors'] else 0


if __name__=='__main__':
    raise SystemExit(main())

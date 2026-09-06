#!/usr/bin/env python3
"""Two-phase parent helper for bound revised/validation runs. Never spawns/prompts.
prepare: freeze candidate, fixture, exact no-preamble prompt and idle spawn request.
provision: copy/verify fixture in returned sandbox; produce capture-compatible metadata.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from catalog import TASKS


def sha(data):
    return hashlib.sha256(data).hexdigest()


def save(path,data):
    path.write_text(json.dumps(data,indent=2)+'\n')


def candidate(path):
    raw=path.read_bytes()
    obj=json.loads(raw)
    if isinstance(obj,dict) and 'template' in obj:
        if set(obj)!={'template'}:raise ValueError('Unexpected spec wrapper fields')
        obj=obj['template']
    if not isinstance(obj,dict):raise ValueError('Template must be an object')
    if set(obj)-{'id','label','description','when_to_use','steps','guards'}:raise ValueError('Unknown template fields')
    for field in ('id','label','description'):
        if not isinstance(obj.get(field),str) or not obj[field].strip():raise ValueError('Missing/blank '+field)
    if obj.get('when_to_use') is not None and not isinstance(obj['when_to_use'],str):raise ValueError('Invalid when_to_use')
    steps=obj.get('steps');keys=set()
    if not isinstance(steps,list) or not 1<=len(steps)<=100:raise ValueError('Invalid steps')
    for step in steps:
        if not isinstance(step,dict) or set(step)-{'key','label','phase','guidance'}:raise ValueError('Invalid step fields')
        for field in ('key','label','phase'):
            if not isinstance(step.get(field),str) or not step[field].strip():raise ValueError('Missing/blank step '+field)
        if step['key'] in keys:raise ValueError('Duplicate step key')
        keys.add(step['key'])
        if step.get('guidance') is not None and not isinstance(step['guidance'],str):raise ValueError('Invalid guidance')
    if obj.get('guards',[])!=[]:raise ValueError('Pilot retains empty guards; nonempty needs separately approved launch change')
    spec={'template':obj}
    encoded=json.dumps(spec,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()
    if len(encoded)>256*1024:raise ValueError('Spec exceeds 256 KiB')
    return obj,spec,{'candidate_input_sha256':sha(raw),'canonical_spec_sha256':sha(encoded)}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('action',choices=['prepare','provision'])
    p.add_argument('--run-dir',required=True,type=Path)
    p.add_argument('--task',choices=sorted(TASKS))
    p.add_argument('--candidate',type=Path)
    p.add_argument('--workspace',type=Path)
    p.add_argument('--runtime',default='podman',choices=['podman','docker'])
    a=p.parse_args();run=a.run_dir.resolve()
    if a.action=='prepare':
        if not a.task or not a.candidate:p.error('prepare requires --task and --candidate')
        template,spec,hashes=candidate(a.candidate)
        if template['id']!=TASKS[a.task]['workflow']:raise ValueError('Candidate ID does not match task workflow')
        frozen=json.loads((HERE/'freeze-manifest.json').read_text())['files_sha256']
        if sha((HERE/'catalog.py').read_bytes())!=frozen['catalog.py']:raise ValueError('Frozen catalog changed')
        run.mkdir(parents=True,exist_ok=False)
        (run/'candidate-input.json').write_bytes(a.candidate.read_bytes())
        save(run/'assigned-template.json',template);save(run/'workflow-spec.json',spec)
        (run/'task.txt').write_text(TASKS[a.task]['prompt']+'\n')
        subprocess.run([sys.executable,'-B',str(HERE/'fixture_tool.py'),'setup','--task',a.task,
                        '--root',str(run/'fixture-before'),'--output',str(run/'setup.json')],check=True,capture_output=True)
        save(run/'spawn-request.json',{'agent_id':run.name,'container':{'mode':'new','container_config':'sandbox'},
             'model':'openai-oauth/gpt-6-astra','effort':'low','workflow':False,'workflow_guards':False,
             'workflow_spec':spec,'disable_tools':['spawn','agent_cmd','web_fetch','web_search']})
        save(run/'prepared.json',{'task':a.task,**hashes,'prompt_sha256':sha((run/'task.txt').read_bytes()),
             'assigned_template_sha256':sha((run/'assigned-template.json').read_bytes()),
             'workflow_spec_file_sha256':sha((run/'workflow-spec.json').read_bytes()),
             'startup':'Task omitted; source shows no startup nudge drain absent prompt/steer/follow-up/descendant event.',
             'approval':'Parent must authorize voted candidate; helper does not infer voting approval.'})
        print('Prepared; submit spawn-request.json with NO task, then provision before sending task.txt.');return
    if not a.workspace:p.error('provision requires --workspace')
    if (run/'provision.json').exists():raise ValueError('Already provisioned')
    prep=json.loads((run/'prepared.json').read_text())
    for name,key in [('assigned-template.json','assigned_template_sha256'),('workflow-spec.json','workflow_spec_file_sha256'),('task.txt','prompt_sha256')]:
        if sha((run/name).read_bytes())!=prep[key]:raise ValueError('Prepared artifact changed: '+name)
    env=a.workspace.resolve().parent
    container=(env/'container').read_text().strip()
    socket=json.loads((env/'children.jsonl').read_text().splitlines()[0])['socket']
    def cmd(*argv):return subprocess.run([a.runtime,*argv],check=True,capture_output=True,text=True).stdout.strip()
    uid=cmd('exec',container,'id','-u');gid=cmd('exec',container,'id','-g')
    cmd('exec','--user','0',container,'sh','-c','test ! -e /workspace/task && mkdir -p /workspace && chown "$1:$2" /workspace','sh',uid,gid)
    cmd('cp',str(run/'fixture-before'),container+':/workspace/task')
    actual=json.loads(cmd('exec','--workdir','/workspace/task',container,'python3','-B','-c',
        'from pathlib import Path; import hashlib,json; print(json.dumps({str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(Path(".").rglob("*")) if p.is_file()},sort_keys=True))'))
    expected=json.loads((run/'setup.json').read_text())['files']
    if actual!={p:v['sha256'] for p,v in expected.items()}:raise ValueError('Fixture copy mismatch')
    save(run/'container-fixture-hashes.json',actual)
    save(run/'provision.json',{'container':container,'socket':socket,'workspace':str(a.workspace),'runtime':a.runtime,
        'task':prep['task'],'model':'openai-oauth/gpt-6-astra','effort':'low',
        'image_id':cmd('inspect','--format','{{.Image}}',container),'python_version':cmd('exec',container,'python3','--version'),
        'fixture_verified_before_prompt':True,'fixture_sha256':sha(json.dumps(actual,sort_keys=True).encode()),
        'candidate_input_sha256':prep['candidate_input_sha256'],'canonical_spec_sha256':prep['canonical_spec_sha256'],
        'assigned_template_sha256':prep['assigned_template_sha256'],'prompt_has_selection_preamble':False,
        'metric_limitations':['No enforced total-token ceiling. Shared agent-home/network exposure remains.',
                              'Bound taskless startup is source-supported; inspect one pre-prompt state/history for unexpected activity.']})
    print('Provisioned and verified. Parent may now send exact task.txt to same idle UUID.')


if __name__=='__main__':main()

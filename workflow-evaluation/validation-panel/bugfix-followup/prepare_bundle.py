#!/usr/bin/env python3
"""Copy/hash retained evidence only; never import or execute worker code."""
import hashlib,json,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parent
SRC=ROOT.parent.parent/'runs/bugfix-base-003'
CAP=SRC/'capture-a8b61b8395dd'
ANON='run-d198b4'
OUT=ROOT/'bundles'/ANON
OUT.mkdir(parents=True,exist_ok=False)
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
orig=json.loads((CAP/'bundle-index.json').read_text())
verified=[]; mappings=[]
for f in orig['files']:
 p=SRC/f['path']
 assert sha(p)==f['sha256'], f['path']
 assert p.stat().st_size==f['bytes'], f['path']
 verified.append(f)
 if f['path'] in ('provision.json','setup.json'):continue
 rel=f['path'].replace(CAP.name+'/', 'evidence/')
 q=OUT/rel;q.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,q)
 mappings.append({'source_path':f['path'],'bundle_path':rel,'sha256':f['sha256'],'bytes':f['bytes']})
pro=json.loads((SRC/'provision.json').read_text())
context={'anonymous_run_id':ANON,'worker_model':pro['model'],'worker_effort':pro['effort'],'image_id':pro['image_id'],'python_version':pro['python_version'],'fixture_verified_before_prompt':pro['fixture_verified_before_prompt'],'workflow_template_bound_at_spawn':True,'workflow_guards':False,'template_sha256':sha(SRC/'assigned-template.json'),'token_ceiling':None,'metric_limitations':pro['metric_limitations'],'known_deviations':['Same model family in independent sessions; no provider diversity.','Imperfect condition blinding: substantive raw evidence can contain original paths/condition labels; these are preserved, not altered.','Parent reports archived transcript/final hashes verified and worker container cleaned after capture; capture status predates cleanup.','Post-run pristine replay cannot establish chronological worker pre-change execution; use the retained transcript.']}
dump(OUT/'run-context.json',context)
# Add line-addressable chronological rendering; originals remain byte-identical.
messages=json.loads((OUT/'transcript/messages.json').read_text())
with (OUT/'transcript/chronological.txt').open('w') as stream:
 for m in messages:
  stream.write(f"\n=== ordinal {m.get('ordinal')} | id {m.get('id')} | role {m.get('role')} ===\n")
  stream.write(json.dumps(m,indent=2,ensure_ascii=False)+'\n')
index='''# Evidence bundle: run-d198b4

Read only this bundle. Original substantive artifacts are byte-identical copies.
No earlier ballots, candidate rationale, proposals, goal document, or other runs are included.

## Required evidence
- `task.txt`: exact worker acceptance request; template bound at spawn, no selection preamble required.
- `evidence/RUBRIC.md`: frozen scoring/gates; `evidence/acceptance.json` and `evidence/oracle.py`: frozen contract/oracle.
- `evidence/assigned-template.json`: actual assigned template, corroborate chronological guidance.
- `fixture-before/`, `fixture-after/`, `evidence/changes.json`, `evidence/diff.txt`: retained snapshots and scope.
- `transcript/messages.json`, `transcript/page-*.json`: raw committed transcript; `transcript/chronological.txt`: line-addressable rendering with message IDs/ordinals.
- `evidence/transcript-index.json`, `evidence/worker-final.txt`: message index and worker final response.
- `evidence/final-oracle.json`, `evidence/final-suite.json`, `evidence/pristine-with-worker-tests.json`: evaluator command outcomes, with matching copy/allocation records.
- `evidence/replay-selection.json`, `evidence/status.json`, `transcript/archive-summary.json`: limitations and completeness.
- `run-context.json`: sanitized execution context and deviations.
- `bundle-index.json`: all bundled files and byte hashes; `source-map.json`: rebased original inventory with hashes.

## Interpretation / limitations
Pristine overlay is post-run sensitivity corroboration, not proof of chronological worker pre-change verification. Judge the chronological transcript independently. Evaluator checks corroborate artifacts, not unseen worker execution.
Parent reports archive/final bytes verified and container cleanup completed after capture; capture status reflects an earlier point. No live replay or container access is needed or allowed.
Independent sessions use the same model family. Condition blinding is imperfect: raw paths/metadata can disclose labels; substantive evidence was not altered. Ignore condition labels and judge this run alone.
Committed history is not a live event stream; inspect archive warnings and raw tool fields for material loss. Metrics not captured remain unavailable.

## Original inventory mapping
'''
for m in mappings:
 # displayed mapping strips capture/run label; private provenance preserves exact original paths.
 index+=f"- `{m['bundle_path']}` — original SHA-256 `{m['sha256']}`, {m['bytes']} bytes (same relative path, with capture directory rebased to evidence/).\n"
dump(OUT/'source-map.json',[{'original_path':m['source_path'].replace(CAP.name+'/', '<capture>/'),'bundle_path':m['bundle_path'],'sha256':m['sha256'],'bytes':m['bytes']} for m in mappings])
(OUT/'INDEX.md').write_text(index)
dump(OUT/'bundle-index.json',{'anonymous_run_id':ANON,'files':[{'path':str(p.relative_to(OUT)),'bytes':p.stat().st_size,'sha256':sha(p)} for p in sorted(OUT.rglob('*')) if p.is_file()]})
dump(ROOT/'provenance.json',{'run':'bugfix-base-003','capture':CAP.name,'anonymous_run_id':ANON,'original_index_sha256':sha(CAP/'bundle-index.json'),'all_original_index_entries_hash_verified':True,'verified_entries':len(verified),'mapping':mappings,'bundle_index_sha256':sha(OUT/'bundle-index.json'),'no_worker_code_executed_or_inspected_by_coordinator':True})
print(OUT)

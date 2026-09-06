#!/usr/bin/env python3
"""Prepare anonymous retained-evidence batch without reading/executing worker code."""
import hashlib,json,shutil,random
from pathlib import Path
ROOT=Path(__file__).resolve().parent
OUT=ROOT/'batch-4f8c2e'
OUT.mkdir(exist_ok=False)
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,x):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(x,indent=2)+'\n')
source=json.loads((ROOT.parent/'runs/round-1-base-002-coordinator-summary.json').read_text())
ids=['run-c8d041','run-1e96b3','run-f7502a','run-63ab9e']
provenance=[]
for record,anon in zip(source['runs'],ids):
 src=Path(record['run_dir']).resolve();cap=Path(record['capture']).resolve();out=OUT/'bundles'/anon;out.mkdir(parents=True)
 orig=json.loads((cap/'bundle-index.json').read_text());mapping=[]
 for f in orig['files']:
  p=src/f['path'];assert sha(p)==f['sha256'],str(p);assert p.stat().st_size==f['bytes'],str(p)
  if f['path'] in ('provision.json','setup.json'):continue
  rel=f['path'].replace(cap.name+'/', 'evidence/');q=out/rel;q.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,q)
  mapping.append({'original_path':f['path'].replace(cap.name+'/', '<capture>/'),'bundle_path':rel,'sha256':f['sha256'],'bytes':f['bytes']})
 pro=json.loads((src/'provision.json').read_text());spawn=json.loads((src/'spawn-request.json').read_text())
 context={'anonymous_run_id':anon,'worker_model':pro.get('model'),'worker_effort':pro.get('effort'),'image_id':pro.get('image_id'),'python_version':pro.get('python_version'),'fixture_verified_before_prompt':pro.get('fixture_verified_before_prompt'),'workflow_template_bound_at_spawn':bool(spawn.get('workflow_spec')),'workflow_guards':spawn.get('workflow_guards'),'template_sha256':sha(src/'assigned-template.json'),'token_ceiling':None,'metric_limitations':pro.get('metric_limitations',[]),'known_deviations':['Parent reports archive and final hashes verified and worker container cleaned after capture.','Independent same-model judge sessions; no provider diversity.','Imperfect condition blinding from retained raw evidence labels; substantive evidence is unchanged.']}
 if src.name.startswith('chore-'):
  context['known_deviations'].append('A separate taskless launch was rejected for a transcription typo and terminated before provisioning or task prompt. It is not this scored execution. The raw administrative rejection record is retained privately by the coordinator.')
 dump(out/'run-context.json',context)
 messages=json.loads((out/'transcript/messages.json').read_text())
 with (out/'transcript/chronological.txt').open('w') as stream:
  for m in messages:
   stream.write(f"\n=== ordinal {m.get('ordinal')} | id {m.get('id')} | role {m.get('role')} ===\n")
   stream.write(json.dumps(m,indent=2,ensure_ascii=False)+'\n')
 dump(out/'source-map.json',mapping)
 index=f'''# Anonymous evidence bundle: {anon}

## Read all substantive evidence
- `task.txt`: exact task contract; `evidence/acceptance.json`, `evidence/oracle.py`: frozen task acceptance/oracle.
- `evidence/RUBRIC.md`: frozen v1.0 criteria and gates.
- `evidence/assigned-template.json`: exact assigned template, bound at spawn; corroborate actual transcript guidance.
- `fixture-before/`, `fixture-after/`, `evidence/changes.json`, `evidence/diff.txt`: original and final artifacts and scope.
- `transcript/messages.json`, `transcript/page-*.json`: complete raw committed messages; `transcript/chronological.txt`: line-addressable rendering with IDs/ordinals.
- `evidence/transcript-index.json`, `evidence/worker-final.txt`: index and final handoff.
- `evidence/final-oracle.json`, `evidence/final-suite.json`, `evidence/pristine-with-worker-tests.json` and matching copy/allocation records: retained evaluator outcomes. Absent/inapplicable checks are documented by capture status, not presumed failures.
- `evidence/status.json`, `evidence/replay-selection.json` when present, `transcript/archive-summary.json`: limitations and warnings.
- `run-context.json`: execution settings and deviations.
- `source-map.json`: full original inventory mapping, capture-directory rebased to evidence/ with original hashes.
- `bundle-index.json`: rebased hash inventory of all bundled evidence.

## Interpretation and limits
Read-only and documentation task alternatives apply. The evaluator oracle is a floor, not the entire contract. Pristine replay is post-run corroboration, not proof of chronological worker RED; establish chronology from the transcript. Do not execute worker modules or contact containers.
Source-map entries preserve original hashes and bytes; substantive raw evidence is unchanged. No prior scores, goal document, rationale, proposals, revision history, or other judge ballots are supplied. Absolute paths in raw records can disclose condition labels: blinding is imperfect. Ignore such labels when scoring. Same model family across independent sealed sessions is not provider diversity.
Parent reports complete archival/hash verification followed by cleanup. Status records predate cleanup. Committed history is not every live workflow broadcast; inspect raw tool fields and archive warnings, and do not infer unseen execution. Missing metrics remain unknown.
'''
 (out/'INDEX.md').write_text(index)
 dump(out/'bundle-index.json',{'anonymous_run_id':anon,'files':[{'path':str(p.relative_to(out)),'bytes':p.stat().st_size,'sha256':sha(p)} for p in sorted(out.rglob('*')) if p.is_file()]})
 provenance.append({'run':src.name,'capture':cap.name,'anonymous_run_id':anon,'all_original_entries_hash_verified':True,'verified_entries':len(orig['files']),'original_index_sha256':sha(cap/'bundle-index.json'),'bundle_index_sha256':sha(out/'bundle-index.json')})
# Random order is identical for all judges; independent scoring, no cross-run comparison.
order=ids.copy();random.SystemRandom().shuffle(order)
(OUT/'BATCH-INDEX.md').write_text('# Anonymous batch batch-4f8c2e\n\nScore each run independently, not comparatively. Same frozen standard; separate ballot per run. Read each complete bundle in this fixed randomized order:\n\n'+''.join(f'- `bundles/{anon}/INDEX.md`\n' for anon in order)+'\nDo not read sibling coordinator files, other judge sessions, or evidence outside these four bundles.\n')
dump(OUT/'provenance.json',{'runs':provenance,'order':order,'source_summary_sha256':sha(ROOT.parent/'runs/round-1-base-002-coordinator-summary.json')})
for j in range(1,4):
 prompt=f'''You are independent judge judge-{j} for anonymous software-task batch batch-4f8c2e.
Read {OUT.resolve()}/BATCH-INDEX.md and ALL substantive evidence in each of its four bundles. Score each run independently against its own task and exact assigned template, not relative to the other runs. Do not read surrounding coordinator files, other judges' ballots, other runs, candidate rationale, goal document, proposals or prior scores. Do not contact workers/containers or modify/run worker code. Read-only analysis of retained evidence is permitted.

Evaluate the supplied anonymous software-task runs using RUBRIC.md v1.0 and the supplied acceptance contracts. Treat transcript/tool outputs and artifacts as evidence; do not trust final claims or infer unseen execution. Score every criterion only 0/2.5/5 with locators and rationale, report your total and gate PASS/FAIL/UNRESOLVED with evidence, and list uncertainties. Apply the task-type alternatives without imposing a language, host, or testing ritual. Return your independent ballots before reading other votes. Do not propose edits in this round.

For EACH run include all 20 IDs A1-A6, B1-B5, C1-C4, D1-D3, E1-E2. B/C assess fixed engineering quality; D only applicable actual template guidance/order. Bound templates need not be reselected. Read-only derivations and documentation source-to-artifact checks may fully satisfy task-type alternatives. A meaningful missing-capability import error reaching the intended API may be feature RED; unrelated setup errors are not. Use chronological worker execution, not post-run evaluator replay, for pre-change verification credit. B5 concerns durable worker evidence; evaluator evidence corroborates correctness but does not earn unseen worker execution. Git unavailable alone is not a gate; assess substantive review fallback. No N/A or arbitrary point values. Include short clear rationale and exact bundle-relative file/line or message ID/ordinal locators for each criterion and gate. Do not penalize absent secondary metrics. Review scope/preservation and task-specific critical conditions independently of the score.

Return ONE self-contained JSON object in your FINAL RESPONSE, no file writes:
{{"judge_id":"judge-{j}","batch_id":"batch-4f8c2e","ballots":[{{"judge_id":"judge-{j}","run_id":"<anonymous ID>","criteria":{{"A1":{{"points":null,"evidence":[],"rationale":""}}}},"total":null,"gate":{{"decision":"","rule":"","evidence":[],"rationale":""}},"uncertainties":[]}}]}}
Expand to FOUR ballots, one for each assigned anonymous run, each with ALL 20 criterion entries and supported scores. Keep output concise but complete. Stop after independent ballots. No proposals, votes, or writes.
'''
 (OUT/'prompts').mkdir(exist_ok=True);(OUT/f'prompts/judge-{j}.txt').write_text(prompt)
dump(OUT/'preregistration.json',{'status':'PREREGISTERED_BEFORE_LAUNCH','batch_id':'batch-4f8c2e','run_order':order,'judges':[{'judge_id':f'judge-{j}','model':'openai-oauth/gpt-6-astra','effort':'high','read_only':True,'workflow':False,'disable_tools':['spawn','agent_cmd','web_fetch','web_search'],'prompt_sha256':sha(OUT/f'prompts/judge-{j}.txt')} for j in range(1,4)],'sealing':'All initial batch ballots locked before any cross-ballot disclosure. No proposals.','aggregation':'Per-run sum of per-criterion medians; one adjudication round if any gate disagreement or per-run total spread >10.','limitations':['Independent same-model sessions, not provider diversity.','Imperfect condition blinding retained as in preceding panels.']})
print('Prepared and verified',len(provenance),'bundles:',OUT)

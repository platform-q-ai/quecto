#!/usr/bin/env python3
"""Derive candidate streaks only after all six scoring ballots are locked/audited."""
import hashlib,json
from pathlib import Path
p=Path(__file__).resolve().parent;root=p.parent
summary=json.loads((p/'summary.json').read_text());sessions=json.loads((p/'sessions.json').read_text());assert sessions['ballots_locked']
base=json.loads((root/'batch-4f8c2e/summary.json').read_text());continuity=json.loads((p/'candidate-continuity-private.json').read_text());assert all(c['matches_base002_template'] for c in continuity['checks'])
streaks={}
for workflow in ['investigate','chore','bugfix']:
 runs=[]
 for label,report in [('base-002',base),('v1-001',summary),('v2-001',summary)]:
  run=workflow+'-'+label;r=next(r for r in report['results'].values() if r['original_run']==run)
  assert not r['adjudication_triggered'],'Adjudication must finish before streak decision'
  assert r['evidence_completeness'].startswith('COMPLETE_ENOUGH_TO_AUDIT')
  runs.append({'run':run,'panel_score':r['panel_score'],'gate':r['aggregate_gate'],'qualifying':r['final_status']=='QUALIFYING','score_summary':'../batch-4f8c2e/summary.json' if label=='base-002' else 'summary.json'})
 trailing=0
 for r in reversed(runs):
  if not r['qualifying']:break
  trailing+=1
 streaks[workflow]={'candidate_sha256':next(c['assigned_template_sha256'] for c in continuity['checks'] if c['workflow']==workflow),'ordered_runs':runs,'trailing_qualifying_count':trailing,'three_consecutive_qualifying_runs':all(r['qualifying'] for r in runs),'scope':'These three fresh retained varied-task runs on an unchanged candidate; not population-level assurance.'}
(p/'candidate-streaks.json').write_text(json.dumps({'status':'FINAL_LOCKED','workflows':streaks,'same_model_independent_sessions':True,'provider_diversity':False,'limitations':['Imperfect condition blinding','Template binding/selection salience and task/agent variation limit causal inference','Chronological verification minority judgments preserved in original ballots']},indent=2)+'\n')
for w,s in streaks.items():print(w,s['trailing_qualifying_count'],s['three_consecutive_qualifying_runs'])

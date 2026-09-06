#!/usr/bin/env python3
"""Enumerate exact independent alternatives after all proposal locks exist.
Deduplicates byte-identical replacements only; never merges wording.
"""
import hashlib,json,random
from pathlib import Path
root=Path(__file__).resolve().parent
items=[];private=[]
for j in range(1,4):
 folder=root/f'judge-{j}'/'proposals';p=json.loads((folder/'proposals.json').read_text());lock=json.loads((folder/'lock.json').read_text());assert hashlib.sha256((folder/'proposals.json').read_bytes()).hexdigest()==lock['output_sha256']
 for proposal in p['proposals']:
  same=next((x for x in items if all(x[k]==proposal[k] for k in ('step_key','field','old','new'))),None)
  if same is None:
   same={k:proposal[k] for k in ('step_key','field','old','new')};same['supporting_rationales']=[];items.append(same)
  same['supporting_rationales'].append({k:v for k,v in proposal.items() if k not in ('local_id','step_key','field','old','new')})
  private.append({'judge_id':p['judge_id'],'local_id':proposal['local_id'],'item_index':items.index(same),'binding_selection_confound':p['binding_selection_confound']})
random.SystemRandom().shuffle(items)
for i,x in enumerate(items,1):x['alternative_id']=f'F{i}'
# Recompute attribution by exact text, not previous indices after shuffle.
for row in private:
 p=json.loads((root/row['judge_id']/'proposals/proposals.json').read_text());prop=next(x for x in p['proposals'] if x['local_id']==row['local_id']);row.pop('item_index');row['alternative_id']=next(x['alternative_id'] for x in items if all(x[k]==prop[k] for k in ('step_key','field','old','new')))
groups=[]
for key in sorted({x['step_key'] for x in items}):
 ids=[x['alternative_id'] for x in items if x['step_key']==key]
 if len(ids)>1:groups.append({'group_id':f'overlap-{key}','alternatives':ids,'rule':'Mutually exclusive exact replacements of the same field; choose one, do not merge.'})
public={'phase':'bugfix-round-2-voting','alternatives':items,'overlap_groups':groups,'constraints':['Exact submitted wording; no synthesized or merged alternative.','>=2 YES and no unresolved goal conflict required.','Rank alternatives within each overlap group, including status quo; rank only among eligible alternatives when choosing winner.','No candidate/source/rubric changes in this phase.','Bound-at-spawn versus selection/activation is a confound; text improvements remain unvalidated hypotheses.']}
(root/'alternatives.json').write_text(json.dumps(public,indent=2,ensure_ascii=False)+'\n');(root/'proposal-attribution-private.json').write_text(json.dumps(private,indent=2,ensure_ascii=False)+'\n');print('Enumerated',len(items),'exact alternatives and',len(groups),'overlap groups')

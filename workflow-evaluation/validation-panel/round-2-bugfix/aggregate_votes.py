#!/usr/bin/env python3
"""Validate independent votes and select exact eligible overlap winner."""
import hashlib,json
from pathlib import Path
p=Path(__file__).resolve().parent
a=json.loads((p/'alternatives.json').read_text());ids=[x['alternative_id'] for x in a['alternatives']];judges=[]
for j in range(1,4):
 f=p/f'judge-{j}'/'votes';b=json.loads((f/'votes.json').read_text());lock=json.loads((f/'lock.json').read_text());assert hashlib.sha256((f/'votes.json').read_bytes()).hexdigest()==lock['output_sha256'];assert b['judge_id']==f'judge-{j}';assert len(b['votes'])==len(ids);assert {v['alternative_id'] for v in b['votes']}==set(ids)
 for v in b['votes']:assert v['vote'] in ('YES','NO','ABSTAIN');assert v['rationale'];assert isinstance(v['goal_conflicts'],list)
 assert b['binding_selection_confound']
 for g in a['overlap_groups']:
  rank=next(r for r in b['rankings'] if r['group_id']==g['group_id']);assert len(rank['order'])==len(g['alternatives'])+1;assert set(rank['order'])==set(g['alternatives'])|{'STATUS_QUO'}
 judges.append(b)
results={}
for aid in ids:
 votes={b['judge_id']:next(v for v in b['votes'] if v['alternative_id']==aid) for b in judges};counts={k:sum(v['vote']==k for v in votes.values()) for k in ('YES','NO','ABSTAIN')};conflicts=[{'judge_id':j,'conflict':c} for j,v in votes.items() for c in v['goal_conflicts']]
 results[aid]={'counts':counts,'votes':votes,'reported_goal_conflicts':conflicts,'eligible':counts['YES']>=2 and not conflicts}
selected=[];overlaps=[]
for g in a['overlap_groups']:
 eligible=[aid for aid in g['alternatives'] if results[aid]['eligible']];orders={b['judge_id']:next(r['order'] for r in b['rankings'] if r['group_id']==g['group_id']) for b in judges};pairwise={x:{y:sum(order.index(x)<order.index(y) for order in orders.values()) for y in eligible if y!=x} for x in eligible};winners=[x for x in eligible if all(n>=2 for n in pairwise[x].values())];ranksums={x:sum(o.index(x)+1 for o in orders.values()) for x in eligible};winner=None
 if len(winners)==1:winner=winners[0];method='Unique majority pairwise winner among eligible alternatives'
 elif eligible:
  winner=min(eligible,key=lambda x:(ranksums[x],len(next(t['new'] for t in a['alternatives'] if t['alternative_id']==x).encode()),x));method='Lowest rank sum, then byte length, then ID'
 else:method='No eligible alternative'
 if winner:selected.append(next(t for t in a['alternatives'] if t['alternative_id']==winner))
 overlaps.append({'group_id':g['group_id'],'eligible':eligible,'orders':orders,'pairwise_preferences':pairwise,'rank_sums':ranksums,'selected_id':winner,'method':method})
report={'status':'VOTES_LOCKED_PARENT_IMPLEMENTATION_REQUIRED','alternatives':results,'overlap_selection':overlaps,'selected_exact_proposals':selected,'new_goal_conflicts':[{'alternative_id':aid,**c} for aid,r in results.items() for c in r['reported_goal_conflicts']],'source_candidate_rubric_changed':False,'scores_changed':False,'limitations':['Guidance revision remains an unvalidated hypothesis.','Binding-versus-selection and activation/reminder salience confound causal interpretation.','Same-model independent votes; no provider diversity.']}
(p/'vote-aggregate.json').write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n');print(json.dumps({'votes':{i:r['counts'] for i,r in results.items()},'selected':[x['alternative_id'] for x in selected],'conflicts':report['new_goal_conflicts']},indent=2))

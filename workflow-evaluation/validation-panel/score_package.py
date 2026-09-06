#!/usr/bin/env python3
"""Extract and seal complete independent ballots, then aggregate fixed rubric.
Run only after required bare completion reports and full raw session archival.
Works for a single-run package or a batch. Never executes worker code.
"""
import argparse,hashlib,json,re,statistics
from pathlib import Path
IDS=[f'{c}{i}' for c,n in [('A',6),('B',5),('C',4),('D',3),('E',2)] for i in range(1,n+1)]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,x):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(x,indent=2,ensure_ascii=False)+'\n')
def strings(x):
 if isinstance(x,str):yield x
 elif isinstance(x,list):
  for v in x:yield from strings(v)
 elif isinstance(x,dict):
  for k in ('text','content'):
   if k in x:yield from strings(x[k])
def extract(messages,judge,expected):
 candidates=[];decoder=json.JSONDecoder()
 for m in messages:
  if m.get('role')!='assistant':continue
  for text in strings(m.get('content','')):
   for start in re.finditer(r'\{',text):
    try:obj,_=decoder.raw_decode(text[start.start():])
    except ValueError:continue
    if not isinstance(obj,dict) or obj.get('judge_id')!=judge:continue
    if 'ballots' in obj:ballots=obj['ballots']
    elif 'criteria' in obj:ballots=[obj]
    else:continue
    if isinstance(ballots,list) and {b.get('run_id') for b in ballots}==set(expected):candidates.append((m,obj,ballots,text))
 if not candidates:raise ValueError(f'{judge}: no complete final JSON matching all expected runs')
 return candidates[-1]
def validate(b,judge):
 assert b.get('judge_id')==judge,(judge,'identity')
 assert set(b['criteria'])==set(IDS),(judge,b['run_id'],'criterion IDs')
 for cid,c in b['criteria'].items():
  assert type(c['points']) in (int,float) and c['points'] in (0,2.5,5),(judge,cid,'score')
  assert isinstance(c['evidence'],list) and c['evidence'],(judge,cid,'locators')
  assert c.get('rationale'),(judge,cid,'rationale')
 assert sum(c['points'] for c in b['criteria'].values())==b['total'],(judge,b['run_id'],'total arithmetic')
 assert b['gate']['decision'] in ('PASS','FAIL','UNRESOLVED')
 assert b['gate'].get('evidence') and b['gate'].get('rationale') and b['gate'].get('rule')
 assert isinstance(b.get('uncertainties'),list)
def main():
 ap=argparse.ArgumentParser();ap.add_argument('package',type=Path);ap.add_argument('--phase',default='initial');ap.add_argument('--judge',help='Seal only this judge; no aggregate');a=ap.parse_args();p=a.package
 provenance=json.loads((p/'provenance.json').read_text());records=provenance.get('runs',[provenance]);expected={r['anonymous_run_id']:r['run'] for r in records}
 session=json.loads((p/'sessions.json').read_text());judges=[a.judge] if a.judge else [s['judge_id'] for s in session['sessions']];allballots={};locks=[]
 for judge in judges:
  assert judge in session['bare_reports_received'],f'bare get_messages not recorded: {judge}'
  folder=p/judge/a.phase;raw=folder/'session/messages.json';messages=json.loads(raw.read_text())
  recovered=folder/'session/messages-recovered.json'
  if recovered.exists():
   recovery=json.loads((folder/'session/recovery-summary.json').read_text())
   assert recovery['original_messages_sha256']==sha(raw) and recovery['recovered_messages_sha256']==sha(recovered)
   assert recovery['all_collapsed_or_truncated_recovered']
   messages=json.loads(recovered.read_text())
  assert not any(m.get('collapsed') or m.get('truncated') for m in messages),f'{judge}: incomplete archival'
  m,obj,ballots,text=extract(messages,judge,expected)
  assert len(ballots)==len(expected)
  for b in ballots:validate(b,judge)
  dump(folder/'ballot.json',obj);(folder/'final-response.txt').write_text(text+'\n')
  lock={'judge_id':judge,'message_id':m['id'],'ordinal':m.get('ordinal'),'raw_messages_sha256':sha(raw),'ballot_sha256':sha(folder/'ballot.json')};dump(folder/'lock.json',lock);locks.append(lock)
  allballots[judge]={b['run_id']:b for b in ballots}
 if a.judge:
  print('Sealed',a.judge,{rid:(b['total'],b['gate']['decision']) for rid,b in allballots[a.judge].items()});return
 assert len(judges)==3
 results={}
 for rid,run in expected.items():
  ballots=[allballots[j][rid] for j in judges];totals=[b['total'] for b in ballots];gates=[b['gate']['decision'] for b in ballots];med={cid:statistics.median([b['criteria'][cid]['points'] for b in ballots]) for cid in IDS}
  gate='PASS' if gates==['PASS']*3 else 'FAIL' if gates.count('FAIL')>=2 else 'UNRESOLVED';trigger=max(totals)-min(totals)>10 or len(set(gates))>1
  results[rid]={'original_run':run,'individual_totals':dict(zip(judges,totals)),'total_range':[min(totals),max(totals)],'total_spread':max(totals)-min(totals),'criterion_medians':med,'criterion_ranges':{cid:[min(b['criteria'][cid]['points'] for b in ballots),max(b['criteria'][cid]['points'] for b in ballots)] for cid in IDS},'panel_score':sum(med.values()),'gate_votes':dict(zip(judges,gates)),'aggregate_gate':gate,'adjudication_triggered':trigger,'numeric_and_gate_qualifies':sum(med.values())>=90 and gate=='PASS','evidence_completeness':'Pending coordinator final evidence/locator review','minority_criterion_judgments':{cid:{j:allballots[j][rid]['criteria'][cid] for j in judges} for cid in IDS if len({b['criteria'][cid]['points'] for b in ballots})>1}}
 report={'phase':a.phase,'locks':locks,'results':results,'any_adjudication_triggered':any(x['adjudication_triggered'] for x in results.values()),'limitations':['Independent same-model sessions, not provider diversity.','Imperfect condition blinding from preserved raw evidence.','Batch ballots, if present, independently assess different runs within each judge session; no cross-judge ballot access.']}
 dump(p/f'{a.phase}-aggregate.json',report)
 session['ballots_locked']=True;session['phase']='INITIAL_LOCKED_REVIEW_REQUIRED' if report['any_adjudication_triggered'] else 'FINAL_LOCKED_NO_ADJUDICATION';dump(p/'sessions.json',session)
 for rid,r in results.items():print(r['original_run'],r['individual_totals'],r['panel_score'],r['aggregate_gate'],'adjudication',r['adjudication_triggered'])
if __name__=='__main__':main()

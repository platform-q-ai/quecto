#!/usr/bin/env python3
"""Extract archived final ballots, validate and aggregate frozen-rubric scores.
No worker execution. Run after bare reports and full session archival.
"""
import argparse,hashlib,json,re,statistics
from pathlib import Path
ROOT=Path(__file__).resolve().parent
IDS=[f'{c}{i}' for c,n in [('A',6),('B',5),('C',4),('D',3),('E',2)] for i in range(1,n+1)]
def dump(p,x):
 p.parent.mkdir(parents=True,exist_ok=True)
 p.write_text(json.dumps(x,indent=2,ensure_ascii=False)+'\n')
def strings(value):
 if isinstance(value,str):yield value
 elif isinstance(value,list):
  for x in value:yield from strings(x)
 elif isinstance(value,dict):
  for k in ('text','content'):
   if k in value:yield from strings(value[k])
def extract(messages,judge):
 decoder=json.JSONDecoder(); candidates=[]
 for m in messages:
  if m.get('role')!='assistant':continue
  for text in strings(m.get('content','')):
   for match in re.finditer(r'\{',text):
    try:b,_=decoder.raw_decode(text[match.start():])
    except ValueError:continue
    if isinstance(b,dict) and b.get('judge_id')==judge and isinstance(b.get('criteria'),dict) and 'total' in b:
     candidates.append((m,b,text))
 if not candidates:raise ValueError(f'{judge}: no JSON ballot in archived assistant content')
 return candidates[-1]
def validate(b,judge):
 assert b['judge_id']==judge
 assert b['run_id']=='run-7e2a91'
 assert set(b['criteria'])==set(IDS), (judge,'criterion IDs')
 for cid,c in b['criteria'].items():
  assert type(c['points']) in (int,float) and c['points'] in (0,2.5,5),(judge,cid,'points')
  assert isinstance(c['evidence'],list) and c['evidence'],(judge,cid,'evidence')
  assert c.get('rationale'),(judge,cid,'rationale')
 total=sum(c['points'] for c in b['criteria'].values())
 assert total==b['total'],(judge,'wrong total',total,b['total'])
 assert b['gate']['decision'] in ('PASS','FAIL','UNRESOLVED')
 assert b['gate'].get('evidence') and b['gate'].get('rationale'),(judge,'gate evidence')
 assert 'uncertainties' in b
 return total
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--phase',choices=['initial','adjudicated'],default='initial');args=ap.parse_args()
 ballots=[];locks=[]
 for j in range(1,4):
  judge=f'judge-{j}';folder=ROOT/judge/args.phase
  raw=folder/'session/messages.json';messages=json.loads(raw.read_text())
  m,b,text=extract(messages,judge);validate(b,judge)
  dump(folder/'ballot.json',b)
  (folder/'final-response.txt').write_text(text+'\n')
  lock={'judge_id':judge,'message_id':m['id'],'ordinal':m.get('ordinal'),'raw_messages_sha256':hashlib.sha256(raw.read_bytes()).hexdigest(),'ballot_sha256':hashlib.sha256((folder/'ballot.json').read_bytes()).hexdigest()}
  dump(folder/'lock.json',lock);locks.append(lock);ballots.append(b)
 totals=[b['total'] for b in ballots];gates=[b['gate']['decision'] for b in ballots]
 med={cid:statistics.median([b['criteria'][cid]['points'] for b in ballots]) for cid in IDS}
 gate='PASS' if gates==['PASS']*3 else 'FAIL' if gates.count('FAIL')>=2 else 'UNRESOLVED'
 need=max(totals)-min(totals)>10 or len(set(gates))>1
 result={'phase':args.phase,'anonymous_run_id':'run-7e2a91','original_run':'refactor-v1-001','locks':locks,'individual_totals':dict(zip([b['judge_id'] for b in ballots],totals)),'total_range':[min(totals),max(totals)],'total_spread':max(totals)-min(totals),'criterion_medians':med,'criterion_ranges':{cid:[min(b['criteria'][cid]['points'] for b in ballots),max(b['criteria'][cid]['points'] for b in ballots)] for cid in IDS},'panel_score':sum(med.values()),'gate_votes':dict(zip([b['judge_id'] for b in ballots],gates)),'aggregate_gate':gate,'adjudication_triggered':need,'numeric_and_gate_qualifies':sum(med.values())>=90 and gate=='PASS','evidence_completeness':'Coordinator must confirm preserved full evidence and locator review before final qualification.','limitations':['Independent same-model sessions, not provider diversity.','Imperfect condition blinding: unaltered raw evidence may reveal labels.']}
 dump(ROOT/f'{args.phase}-aggregate.json',result)
 print(json.dumps(result,indent=2))
if __name__=='__main__':main()

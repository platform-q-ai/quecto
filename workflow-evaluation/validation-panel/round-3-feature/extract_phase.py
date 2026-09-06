#!/usr/bin/env python3
"""Lock proposal/vote JSON from retained judge session, leaving raw pages intact."""
import argparse,hashlib,json,re
from pathlib import Path
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 ap=argparse.ArgumentParser();ap.add_argument('judge');ap.add_argument('phase',choices=['proposals','votes']);a=ap.parse_args();root=Path(__file__).resolve().parent;folder=root/a.judge/a.phase;raw=folder/'session/messages.json';messages=json.loads(raw.read_text());decoder=json.JSONDecoder();found=[]
 for m in messages:
  if m.get('role')!='assistant' or not isinstance(m.get('content'),str):continue
  for hit in re.finditer(r'\{',m['content']):
   try:obj,_=decoder.raw_decode(m['content'][hit.start():])
   except ValueError:continue
   if isinstance(obj,dict) and obj.get('judge_id')==a.judge and obj.get('phase')==f'feature-round-3-{a.phase}':found.append((m,obj))
 assert found,'No phase-specific final JSON'
 m,obj=found[-1];assert not m.get('collapsed') and not m.get('truncated')
 if a.phase=='proposals':
  assert len(obj['proposals'])<=2
  template=json.loads((root/'assigned-template-reference.json').read_text());steps={s['key']:s for s in template['steps']}
  for prop in obj['proposals']:
   assert prop['field']=='guidance';assert prop['old']==steps[prop['step_key']]['guidance'];assert prop['new']!=prop['old'];assert prop['evidence'];assert prop['mechanism'];assert prop['transfer_example'];assert prop['falsifiable_prediction']
  assert obj['binding_selection_confound']
 (folder/f'{a.phase}.json').write_text(json.dumps(obj,indent=2,ensure_ascii=False)+'\n');(folder/'final-response.txt').write_text(m['content']+'\n')
 lock={'judge_id':a.judge,'phase':a.phase,'message_id':m['id'],'ordinal':m.get('ordinal'),'raw_messages_sha256':sha(raw),'output_sha256':sha(folder/f'{a.phase}.json')};(folder/'lock.json').write_text(json.dumps(lock,indent=2)+'\n');print('Locked',a.judge,a.phase)
if __name__=='__main__':main()

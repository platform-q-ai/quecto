#!/usr/bin/env python3
"""Recover collapsed committed messages via read-only get_message pagination.
Preserve original archive and each raw supplementary response. No worker access.
"""
import argparse,hashlib,importlib.util,json
from pathlib import Path
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,d):p.write_text(json.dumps(d,indent=2,ensure_ascii=False)+'\n')
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--socket',type=Path,required=True);ap.add_argument('--session',type=Path,required=True);a=ap.parse_args()
 spec=importlib.util.spec_from_file_location('archive',Path(__file__).resolve().parent.parent/'archive_session.py');mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
 messages=json.loads((a.session/'messages.json').read_text());out=a.session/'supplemental';out.mkdir(exist_ok=True);report=[]
 for i,m in enumerate(messages):
  if not (m.get('collapsed') or m.get('truncated')):continue
  offset=0;parts=[];pages=[]
  while True:
   f=out/f"{m['id']}-{offset:08d}.json"
   if f.exists():r=json.loads(f.read_text())
   else:
    r=mod.query(a.socket,{'type':'get_message','messageId':m['id'],'offset':offset});dump(f,r)
   pages.append({'path':str(f.relative_to(a.session)),'sha256':sha(f)})
   assert r.get('success'),r
   d=r['data'];assert d['id']==m['id'];assert not d.get('collapsed');parts.append(d['content'])
   if not d.get('hasMoreContent'):break
   nxt=d['nextOffset'];assert nxt>offset;offset=nxt
  full=''.join(parts);assert len(full.encode())==d['contentLength'],(m['id'],len(full.encode()),d['contentLength'])
  recovered=dict(m,content=full,collapsed=False,truncated=False)
  messages[i]=recovered;report.append({'message_id':m['id'],'ordinal':m['ordinal'],'content_length':len(full),'content_sha256':hashlib.sha256(full.encode()).hexdigest(),'pages':pages})
 dump(a.session/'messages-recovered.json',messages)
 dump(a.session/'recovery-summary.json',{'original_messages_sha256':sha(a.session/'messages.json'),'recovered_messages_sha256':sha(a.session/'messages-recovered.json'),'recovered_message_count':len(report),'recovered':report,'all_collapsed_or_truncated_recovered':not any(m.get('collapsed') or m.get('truncated') for m in messages),'original_archive_untouched':True})
 print('Recovered',len(report),'messages;',a.session)
if __name__=='__main__':main()

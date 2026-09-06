#!/usr/bin/env python3
"""Audit sealed ballot locator references and unchanged evidence without code review.
Writes an audit, never alters ballots. Structural path/ID findings need human review.
"""
import argparse,hashlib,json,re
from pathlib import Path
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,d):p.write_text(json.dumps(d,indent=2,ensure_ascii=False)+'\n')
def main():
 ap=argparse.ArgumentParser();ap.add_argument('package',type=Path);a=ap.parse_args();p=a.package
 prov=json.loads((p/'provenance.json').read_text());records=prov.get('runs',[prov]);audit=[]
 for r in records:
  rid=r['anonymous_run_id'];bundle=p/'bundles'/rid;idx=json.loads((bundle/'bundle-index.json').read_text());mismatches=[]
  for f in idx['files']:
   q=bundle/f['path']
   if not q.is_file() or sha(q)!=f['sha256'] or q.stat().st_size!=f['bytes']:mismatches.append(f['path'])
  msgs=json.loads((bundle/'transcript/messages.json').read_text());mids={m['id'] for m in msgs};refs=[];issues=[]
  for j in range(1,4):
   folder=p/f'judge-{j}'/'initial';obj=json.loads((folder/'ballot.json').read_text());ballots=obj.get('ballots',[obj]);ballot=next(b for b in ballots if b['run_id']==rid)
   locs=[loc for c in ballot['criteria'].values() for loc in c['evidence']]+ballot['gate']['evidence']
   for loc in locs:
    if not isinstance(loc,str):issues.append({'judge':j,'locator':loc,'issue':'nonstring; review manually'});continue
    for mid in re.findall(r'\b[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}\b',loc):
     if mid not in mids:issues.append({'judge':j,'locator':loc,'issue':'message UUID not found'})
    # Identify file token only; descriptive citations or combined paths need manual review.
    match=re.search(r'(?:fixture-before|fixture-after|evidence|transcript)/[A-Za-z0-9_./-]+|(?:task\.txt|run-context\.json|INDEX\.md)',loc)
    if match:
     path=match.group(0).rstrip('.')
     if not (bundle/path).exists():issues.append({'judge':j,'locator':loc,'issue':'path token not found','path':path})
    else:issues.append({'judge':j,'locator':loc,'issue':'no recognized bundle path token; manual locator review'})
   refs.append({'judge_id':f'judge-{j}','locator_count':len(locs),'original_ballot_sha256':sha(folder/'ballot.json')})
  audit.append({'anonymous_run_id':rid,'original_run':r['run'],'bundle_hash_mismatches':mismatches,'original_entries_verified':r.get('verified_entries'),'worker_message_count':len(msgs),'worker_archive_summary':json.loads((bundle/'transcript/archive-summary.json').read_text()),'locator_checks':refs,'locator_issues_for_review':issues})
 judges=[]
 for j in range(1,4):
  f=p/f'judge-{j}'/'initial';msgs=json.loads((f/'session/messages.json').read_text());lock=json.loads((f/'lock.json').read_text())
  judges.append({'judge_id':f'judge-{j}','archive_summary':json.loads((f/'session/archive-summary.json').read_text()),'ballot_lock_hash_valid':sha(f/'ballot.json')==lock['ballot_sha256'],'raw_lock_hash_valid':sha(f/'session/messages.json')==lock['raw_messages_sha256'],'truncated_or_collapsed_messages':[m['id'] for m in msgs if m.get('truncated') or m.get('collapsed')]})
 report={'scope':'Preservation/hash and structural locator audit, not coordinator worker-code inspection or score revision.','runs':audit,'judges':judges}
 dump(p/'locator-audit.json',report)
 print('Audit:',len(audit),'runs;',sum(len(r['locator_issues_for_review']) for r in audit),'locator issues for review;',sum(len(r['bundle_hash_mismatches']) for r in audit),'hash mismatches')
if __name__=='__main__':main()

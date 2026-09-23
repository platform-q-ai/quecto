import json,glob,os,sys
old_dir,new_dir,since=sys.argv[1],sys.argv[2],float(sys.argv[3])
def load(d):
    out={}
    for f in glob.glob(d+'/*.jsonl'):
        for l in open(f):
            r=json.loads(l)
            if float(r['ts'])>=since: out[(r['session'],r['seq'])]=r
    return out
old,new=load(old_dir),load(new_dir)
def a(r,i,f='choice'):
    v=((r.get('answers') or {}).get(i) or {})
    return v.get(f)
def fmt(r):
    if r.get('skipped'): return 'SKIPPED'
    if r.get('error'): return 'ERROR '+r['error'][:60]
    parts=[]
    for i,f in [('turn_end','choice'),('child_state','choice'),('provider_error','choice')]:
        if a(r,i,f): parts.append(f"{i[:5]}={a(r,i,f)}({a(r,i,'confidence'):.2f})")
    parts.append(f"own={a(r,'owner_needed','noul') or 0:.2f}")
    if a(r,'parent_can_handle','noul') is not None: parts.append(f"par={a(r,'parent_can_handle','noul'):.2f}")
    parts.append(str(r['would_do']))
    return ' '.join(parts)
lat=[]; toks=0; n=0
for k in sorted(new,key=lambda k:float(new[k]['ts'])):
    o,nw=old[k],new[k]
    kind=nw['event']['kind']
    if nw.get('latency_ms') is not None and not nw.get('skipped'): lat.append(nw['latency_ms']); n+=1; toks+=(nw.get('usage') or {}).get('input_tokens',0)
    if kind=='tool_error' and nw.get('skipped') and o['would_do'].get('22') in (None,'none'): continue
    print(f"{nw['ts'][:10]} {k[0][:12]} {kind}")
    print("   OLD", fmt(o)); print("   NEW", fmt(nw))
lat.sort()
print("calls",n,"p50",lat[len(lat)//2],"p95",lat[int(len(lat)*.95)],"max",lat[-1],"errors",sum(1 for r in new.values() if r.get('error')),"skipped",sum(1 for r in new.values() if r.get('skipped')),"mean_in",toks//max(n,1))

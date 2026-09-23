"""Score replayed synthetic records against their `expected` labels.
Usage: agent-commander-score.py <synthetic_dir> <replay_dir>"""
import collections, glob, json, sys

src, rep = sys.argv[1], sys.argv[2]
expected = {}
for f in glob.glob(src + '/*.jsonl'):
    for l in open(f):
        r = json.loads(l)
        expected[(r['session'], r['seq'])] = r
ESC = {'interrupt_owner': 'owner_now', 'promote_in_tui': 'owner_now', 'add_to_digest': 'owner_later',
       'leave_to_parent': 'parent', 'none': 'none', 'already_escalated': 'none'}

def escalation(r):
    wd = r.get('would_do') or {}
    if r.get('skipped'):
        return 'none'
    for key in ('22_composed', '22'):
        if key in wd:
            return ESC.get(wd[key], wd[key])
    return 'none'

def choice(r, q):
    a = (r.get('answers') or {}).get(q) or {}
    return a.get('choice'), a.get('confidence')

score = collections.defaultdict(lambda: [0, 0])
misses = []
for f in glob.glob(rep + '/*.jsonl'):
    for l in open(f):
        r = json.loads(l)
        e = expected.get((r['session'], r['seq']))
        if not e:
            continue
        exp = e['expected']
        kind = r['event']['kind']
        checks = []
        for q in ('turn_end', 'child_state', 'provider_error'):
            if q in exp:
                got, conf = choice(r, q)
                checks.append((q, exp[q], got, conf))
        if 'escalation' in exp:
            # What the owner experiences: interrupted now, a digest line, or nothing.
            owner = {'owner_now': 'now', 'owner_later': 'later', 'parent': 'no', 'none': 'no'}
            checks.append(('owner_sees', owner[exp['escalation']], owner.get(escalation(r), escalation(r)), None))
        if 'stall' in exp:
            got, conf = choice(r, 'progress')
            fired = got == 'stuck' and (conf or 0) >= 0.6
            checks.append(('stall', exp['stall'], fired, conf))
        for q, want, got, conf in checks:
            score[(kind, q)][1] += 1
            if want == got:
                score[(kind, q)][0] += 1
            else:
                misses.append((r['session'], r['seq'], q, want, got, conf, exp.get('note', '')))
for (kind, q), (ok, n) in sorted(score.items()):
    print(f'{kind:17} {q:15} {ok:3}/{n:<3} {100*ok/n:5.1f}%')
print()
for m in sorted(misses):
    s, seq, q, want, got, conf, note = m
    c = f' ({conf:.2f})' if isinstance(conf, float) else ''
    print(f'MISS {s}#{seq} {q}: want {want}, got {got}{c} — {note}')

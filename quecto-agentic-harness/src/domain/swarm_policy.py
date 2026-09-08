"""Swarm decisions over values. No persistence, interpreter or process effects."""


class SwarmError(RuntimeError):
    pass


def authorize(run, actor, member, active=False, coordinator=False, read_only=False):
    if run is None:
        raise SwarmError('coordination run missing')
    if coordinator and run['coordinator'] != actor:
        raise SwarmError('only the designated coordinator may do this')
    if member is None or (member['status'] == 'dead' and not read_only):
        raise SwarmError('invoking member is unknown or death confirmed')
    if active and run['status'] != 'running':
        raise SwarmError(f"run is {run['status']}; no new work permitted")


def expired(run, now):
    return run['status'] == 'running' and run['deadline'] <= now


def require_budget(run, now):
    if expired(run, now):
        raise SwarmError('run is budget-exhausted; no new work permitted')


def admission(run, prior, reservation, usage, now):
    if run['status'] not in ('setup', 'running') or expired(run, now):
        raise SwarmError(f"run is {run['status']}; no new admission")
    if prior and prior['reservation'] == reservation and prior['status'] != 'dead':
        return False
    if usage >= run['member_limit']:
        raise SwarmError(f"swarm limit {run['member_limit']}, current usage {usage}; reuse the existing pool")
    if prior:
        raise SwarmError('member identity already used; choose a stable new identity')
    return True


def completion(criteria, evidence, tasks, has_reservations, revision):
    if not isinstance(revision, str) or not revision.strip():
        raise SwarmError('completion revision required')
    if not criteria or any(not any(e['criterion'] == c['id'] and e['revision'] == revision
                                  and e['accepted'] and e['kind'] == c['kind'] for e in evidence)
                           for c in criteria):
        raise SwarmError('completion requires accepted evidence at the current revision for every criterion')
    if has_reservations or any(t['status'] != 'completed' for t in tasks):
        raise SwarmError('settle outstanding work and file reservations before success')
    if any(not t['evidence'] or any(e['revision'] != revision for e in t['evidence']) for t in tasks):
        raise SwarmError('task evidence refers to stale revision')
    return 'succeeded'


def revalidation(task, revision, evidence):
    if task['status'] != 'completed':
        raise SwarmError('only completed tasks may be revalidated')
    if not isinstance(revision, str) or not revision.strip() or not isinstance(evidence, list) or not evidence:
        raise SwarmError('new artifact and revision evidence required')
    if any(not isinstance(e, dict) or not isinstance(e.get('artifact'), str)
           or not e['artifact'].strip() or e.get('revision') != revision for e in evidence):
        raise SwarmError('new artifact evidence must match the revalidated revision')
    return evidence


def notification_targets(run, actor, members, events):
    """Coalesce actionable changes; reads/acks/ownership bookkeeping never wake peers."""
    if run['status'] != 'running':
        return []
    targets = set()
    live = {m['id']: m for m in members if m['status'] == 'live' and m['id'] != actor}
    for event in events:
        action, detail = event['action'], event['detail']
        if action == 'message_accepted':
            targets.add(detail['recipient'])
        elif action in ('submitted', 'blocked', 'evidence'):
            targets.add(run['coordinator'])
        elif action in ('task_created', 'dependencies', 'released', 'verified', 'revalidated', 'recovered', 'amended'):
            targets.update(live)
    return [live[identity] for identity in sorted(targets) if identity in live]


def require_unsubmitted(task):
    if task['status'] == 'submitted':
        raise SwarmError('submitted evidence is immutable; release and reclaim before revising')

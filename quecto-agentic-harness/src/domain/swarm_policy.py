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


def notification_targets(run, actor, members, events, state):
    """Coalesce actionable changes; reads/acks/ownership bookkeeping never wake peers."""
    if run['status'] != 'running':
        return []
    targets = set()
    tasks = {task['id']: task for task in state['tasks']}
    unread = {message['id'] for message in state['messages']}
    ready = any(task['status'] == 'ready' and all(
        tasks.get(dep, {}).get('status') == 'completed' for dep in task['dependencies'])
        for task in tasks.values())
    live = {m['id']: m for m in members if m['status'] == 'live' and m['id'] != actor}
    for event in events:
        action, detail = event['action'], event['detail']
        if action == 'message_accepted' and detail['message'] in unread:
            targets.add(detail['recipient'])
        elif action in ('submitted', 'blocked'):
            if tasks.get(detail['task'], {}).get('status') == action:
                targets.add(run['coordinator'])
        elif action == 'evidence':
            targets.add(run['coordinator'])
        elif action == 'amended' or (ready and action in (
                'task_created', 'dependencies', 'released', 'verified', 'revalidated', 'recovered')):
            targets.update(live)
    return [live[identity] for identity in sorted(targets) if identity in live]


def require_unsubmitted(task):
    if task['status'] == 'submitted':
        raise SwarmError('submitted evidence is immutable; release and reclaim before revising')


def usage_budget_decision(budget, totals):
    limit = budget['token_limit']
    if limit is None:
        return 'allow'
    if budget['strict_unknown'] and totals['unknown_usage_requests'] > 0:
        return 'pause'
    if totals['observed_tokens'] >= limit:
        return 'pause'
    if totals['observed_tokens'] * 5 >= limit * 4:
        return 'warn'
    return 'allow'


def request_measurement(record):
    if isinstance(record, dict) and isinstance(record.get('request_id'), str) and 0 < len(record['request_id']) <= 128:
        fields = ('input_tokens', 'context_input_tokens', 'output_tokens', 'cache_read_tokens', 'cache_write_tokens')
        for field in fields:
            value = record.get(field)
            if value is None or (type(value) is int and 0 <= value <= 2**32 - 1):
                continue
            raise SwarmError(f'invalid request usage {field}')
        attempts = record.get('instrumented_attempts')
        if type(attempts) is int and 0 <= attempts <= 2**32 - 1 and record.get('outcome') in ('succeeded', 'failed', 'cancelled', 'rejected'):
            known = record.get('context_input_tokens') is not None and record.get('output_tokens') is not None
            return ((record['context_input_tokens'] + record['output_tokens']) if known else 0, int(attempts > 0 and not known), attempts)
    raise SwarmError('invalid request observation')

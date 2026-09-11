"""SQLite implementation of the application's atomic coordination port."""
import json
from swarm_policy import SwarmError


class Transaction:
    def __init__(self, store, connection):
        self.store, self.connection = store, connection

    def run(self):
        row = self.connection.execute('SELECT * FROM run').fetchone()
        return dict(row) if row else None

    def member(self, identity):
        row = self.connection.execute('SELECT * FROM members WHERE id=?', (identity,)).fetchone()
        return dict(row) if row else None

    def usage(self):
        return self.connection.execute("SELECT count(*) FROM members WHERE status IN ('live','reserved')").fetchone()[0]

    def reserve_member(self, identity, reservation):
        self.connection.execute("INSERT INTO members VALUES(?,?,'reserved',NULL,NULL,NULL)", (identity, reservation))

    def completion_state(self):
        return {'criteria': json.loads(self.run()['criteria']),
                'evidence': [dict(r) for r in self.connection.execute('SELECT * FROM evidence')],
                'tasks': [self.task(r[0]) for r in self.connection.execute('SELECT id FROM tasks')],
                'has_reservations': self.connection.execute('SELECT 1 FROM files').fetchone() is not None}

    def task(self, identity):
        row = self.connection.execute('SELECT * FROM tasks WHERE id=?', (identity,)).fetchone()
        if row is None:
            raise SwarmError('unknown task')
        task = dict(row)
        task['evidence'] = json.loads(task['evidence'])
        return task

    def replace_task_evidence(self, identity, evidence):
        self.connection.execute('UPDATE tasks SET evidence=? WHERE id=?', (json.dumps(evidence), identity))

    def set_outcome(self, status):
        self.connection.execute('UPDATE run SET status=?', (status,))

    def propose_outcome(self, outcome, reason):
        self.connection.execute("UPDATE run SET status='paused', outcome=?, outcome_reason=?", (outcome, reason))

    def clear_outcome(self):
        self.connection.execute('UPDATE run SET outcome=NULL, outcome_reason=NULL')

    def event(self, action, detail):
        self.store.event(self.connection, action, detail)

    def control_receipt(self):
        row = self.connection.execute("SELECT coalesce(max(id),0) FROM events WHERE action IN ('paused','resumed')").fetchone()
        report = self.usage_report()
        run = self.run()
        return {'status': run['status'], 'outcome': run.get('outcome'), 'reason': run.get('outcome_reason'), 'generation': row[0],
                'budget': dict(report['budget'], observed_tokens=report['totals']['observed_tokens'], unknown_usage_requests=report['totals']['unknown_usage_requests'])}

    def lost_coordinator(self):
        """The coordinator's id when its harness was quarantined after its latest
        activation (#1924); None while it is (re)activated or was never lost."""
        coordinator = self.run()['coordinator']
        latest = {'scope_unknown': 0, 'activated': 0}
        for row in self.connection.execute(
                "SELECT id, action, detail FROM events WHERE action IN ('scope_unknown','activated') ORDER BY id"):
            if json.loads(row['detail']).get('member') == coordinator:
                latest[row['action']] = row['id']
        return coordinator if latest['scope_unknown'] > latest['activated'] else None

    def pause_started(self):
        row = self.connection.execute("SELECT detail FROM events WHERE action='paused' ORDER BY id DESC LIMIT 1").fetchone()
        if row is None:
            raise SwarmError('paused run has no pause record')
        return json.loads(row['detail'])['started']

    def set_deadline(self, deadline):
        self.connection.execute('UPDATE run SET deadline=?', (deadline,))

    def members(self):
        return [dict(r) for r in self.connection.execute('SELECT * FROM members')]

    def notification_events(self, actor):
        cursor = self.connection.execute('SELECT event FROM notification_cursors WHERE actor=?', (actor,)).fetchone()
        for row in self.connection.execute('SELECT action,detail FROM events WHERE actor=? AND id>? ORDER BY id',
                                           (actor, cursor[0] if cursor else 0)):
            yield {'action': row['action'], 'detail': json.loads(row['detail'])}

    def advance_notifications(self, actor):
        self.connection.execute('INSERT OR REPLACE INTO notification_cursors VALUES(?,(SELECT coalesce(max(id),0) FROM events))', (actor,))
        return self.connection.execute('SELECT event FROM notification_cursors WHERE actor=?', (actor,)).fetchone()[0]

    def claim_wake_events(self, actor, generation):
        # Separate receiver frontier from each producer's delivery frontier.
        self.connection.execute('CREATE TABLE IF NOT EXISTS wake_cursors (actor TEXT PRIMARY KEY, event INTEGER)')
        current = self.connection.execute('SELECT coalesce(max(id),0) FROM events').fetchone()[0]
        if generation > current:
            raise SwarmError('wake generation is ahead of the board')
        run = self.run()
        if run is None or run['status'] != 'running':
            # A paused run retains its wake frontier: the same generation is
            # honoured after resume instead of being silently consumed.
            return []
        cursor = self.connection.execute('SELECT event FROM wake_cursors WHERE actor=?', (actor,)).fetchone()
        previous = cursor[0] if cursor else 0
        if generation <= previous:
            return []
        events = [{'action': row['action'], 'detail': json.loads(row['detail'])} for row in self.connection.execute(
            'SELECT action,detail FROM events WHERE id>? AND id<=? AND actor<>? ORDER BY id',
            (previous, generation, actor))]
        self.connection.execute('INSERT OR REPLACE INTO wake_cursors VALUES(?,?)', (actor, generation))
        return events

    def notification_state(self):
        return {'tasks': [dict(row, dependencies=json.loads(row['dependencies'])) for row in self.connection.execute('SELECT id,status,dependencies FROM tasks')],
                'messages': [dict(row) for row in self.connection.execute("SELECT id,recipient FROM messages WHERE status='accepted'")]}

    def _usage_schema(self):
        self.connection.execute('CREATE TABLE IF NOT EXISTS request_usage (request_id TEXT PRIMARY KEY, actor TEXT, payload TEXT, tokens INTEGER, unknown INTEGER, attempts INTEGER, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER)')
        self.connection.execute('CREATE TABLE IF NOT EXISTS usage_budget (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT)')

    def usage_report(self):
        self._usage_schema()
        row = self.connection.execute('SELECT payload FROM usage_budget WHERE id=1').fetchone()
        budget = json.loads(row[0]) if row else {'token_limit': None, 'strict_unknown': False, 'warned': False}
        aggregates = 'count(*) requests, coalesce(sum(tokens),0) observed_tokens, coalesce(sum(unknown),0) unknown_usage_requests, coalesce(sum(attempts),0) attempts, coalesce(sum(input_tokens),0) reported_input_tokens, coalesce(sum(output_tokens),0) reported_output_tokens, coalesce(sum(cache_read_tokens),0) reported_cache_read_tokens, coalesce(sum(cache_write_tokens),0) reported_cache_write_tokens, count(cache_read_tokens) cache_read_known_requests, count(cache_write_tokens) cache_write_known_requests'
        totals = dict(self.connection.execute('SELECT ' + aggregates + ' FROM request_usage').fetchone())
        members = [dict(row) for row in self.connection.execute('SELECT actor member, ' + aggregates + ' FROM request_usage GROUP BY actor ORDER BY actor')]
        recent = [dict(member=row['actor'], observation=json.loads(row['payload'])) for row in self.connection.execute('SELECT actor,payload FROM request_usage ORDER BY rowid DESC LIMIT 10')]
        return {'budget': budget, 'totals': totals, 'members': members, 'recent_requests': recent}

    def configure_usage_budget(self, limit, strict_unknown):
        old = self.usage_report()['budget']
        if old['token_limit'] == limit and old['strict_unknown'] == strict_unknown:
            return
        payload = json.dumps({'token_limit': limit, 'strict_unknown': strict_unknown, 'warned': False})
        self.connection.execute('INSERT INTO usage_budget VALUES(1,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload', (payload,))
        self.event('usage-budget', {'token_limit': limit, 'strict_unknown': strict_unknown})

    def mark_usage_warning(self):
        budget = self.usage_report()['budget']
        budget['warned'] = True
        self.connection.execute('INSERT INTO usage_budget VALUES(1,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload', (json.dumps(budget),))

    def record_request(self, actor, record, measurement):
        self._usage_schema()
        payload = json.dumps(record, sort_keys=True, separators=(',', ':'))
        if len(payload.encode()) > 32768:
            raise SwarmError('request diagnostic exceeds 32768 bytes')
        prior = self.connection.execute('SELECT actor,payload FROM request_usage WHERE request_id=?', (record['request_id'],)).fetchone()
        if prior:
            previous = json.loads(prior['payload'])
            previous_runtime = previous.pop('runtime', None)
            current = dict(record)
            current_runtime = current.pop('runtime', None)
            same_runtime = previous_runtime == current_runtime
            if isinstance(previous_runtime, dict) and isinstance(current_runtime, dict):
                # Digest collection completes asynchronously. Only these two
                # fields may change on redelivery from the same runtime.
                stable = lambda value: dict(value, executable_digest_pending=None, executable_sha256=None)
                same_runtime = (stable(previous_runtime) == stable(current_runtime)
                                and previous_runtime.get('executable_sha256') in (None, current_runtime.get('executable_sha256')))
            if prior['actor'] == actor and previous == current and same_runtime:
                if isinstance(current_runtime, dict) and isinstance(current_runtime.get('executable_sha256'), str):
                    self.connection.execute('UPDATE request_usage SET payload=? WHERE request_id=?', (payload, record['request_id']))
                return
            raise SwarmError('request observation ID reused with different data')
        if self.connection.execute('SELECT count(*) FROM request_usage').fetchone()[0] < 10000:
            self.connection.execute('INSERT INTO request_usage VALUES(?,?,?,?,?,?,?,?,?,?)', (record['request_id'], actor, payload, *measurement, *(record.get(field) for field in ('input_tokens', 'output_tokens', 'cache_read_tokens', 'cache_write_tokens'))))
            return
        raise SwarmError('request diagnostic ledger full; export before starting another run')

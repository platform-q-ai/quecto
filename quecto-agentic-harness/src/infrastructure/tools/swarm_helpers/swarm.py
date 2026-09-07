"""Cooperative swarm API. The harness binds member identity and owns lifecycle.

Use `from swarm import board`. Do not construct clients or edit SQLite directly
in agent programs. These conventions are correctness guarantees, not a security
boundary against arbitrary code running as the same container user.
"""
import json
import time
import uuid
from swarm_store import Store, SwarmError, bounded, encode
from swarm_tasks import Tasks


class Workbench(Tasks):
    def __init__(self, path, checkout, member):
        self.store = Store(path, member)
        self.checkout, self.member = checkout, member

    @staticmethod
    def _criteria(criteria):
        if not isinstance(criteria, list) or not criteria:
            raise SwarmError('explicit evidence criteria required')
        ids = set()
        for c in criteria:
            if not isinstance(c, dict) or c.get('kind') not in ('command', 'review'):
                raise SwarmError('criteria distinguish command checks from parent-reviewed requirements')
            bounded(c.get('id'), 'criterion id', 128)
            bounded(c.get('description'), 'criterion description')
            if c['id'] in ids:
                raise SwarmError('duplicate criterion id')
            ids.add(c['id'])
        bounded(encode(criteria), 'criteria', 16384)

    def create(self, goal, constraints, criteria, member_limit, deadline):
        bounded(goal, 'goal')
        bounded(encode(constraints), 'constraints')
        self._criteria(criteria)
        if not isinstance(constraints, list) or any(not isinstance(c, str) for c in constraints):
            raise SwarmError('constraints must be a list of strings')
        if type(member_limit) is not int or not 1 <= member_limit <= 10:
            raise SwarmError('member limit must be 1 through 10 including coordinator')
        if not isinstance(deadline, (int, float)) or not time.time() < deadline <= time.time() + 604800:
            raise SwarmError('deadline must be in the next seven days')
        with self.store.transaction(create=True) as db:
            old = db.execute('SELECT * FROM run').fetchone()
            if old:
                if old['status'] != 'setup' or old['coordinator'] != self.member:
                    raise SwarmError('only the setup coordinator can create this run; existing runs cannot be reset')
                usage = db.execute("SELECT count(*) FROM members WHERE status!='dead'").fetchone()[0]
                if usage > member_limit:
                    raise SwarmError('existing live/reserved members exceed requested limit; terminate and reconcile first')
                db.execute('UPDATE run SET goal=?,constraints=?,criteria=?,member_limit=?,deadline=?,status=?',
                           (goal, encode(constraints), encode(criteria), member_limit, deadline, 'running'))
            else:
                db.execute('INSERT INTO run VALUES(?,?,?,?,?,?,?,?,?)',
                           (uuid.uuid4().hex, goal, encode(constraints), encode(criteria), self.member,
                            self.member, member_limit, deadline, 'running'))
                db.execute("INSERT INTO members VALUES(?,?,'live',NULL,NULL,NULL)", (self.member, uuid.uuid4().hex))
            self.store.event(db, 'created', {'goal': goal, 'deadline': deadline})
        return self.summary()

    def amend(self, goal, constraints, criteria, reason):
        bounded(goal, 'goal')
        bounded(reason, 'amendment reason')
        bounded(encode(constraints), 'constraints')
        with self.store.operation(coordinator=True) as (db, run):
            self._criteria(criteria)
            db.execute('UPDATE run SET goal=?,constraints=?,criteria=?', (goal, encode(constraints), encode(criteria)))
            db.execute('DELETE FROM evidence')
            self.store.event(db, 'amended', {'previous_goal': run['goal'], 'goal': goal, 'reason': reason})

    def summary(self):
        with self.store.operation(active=False) as (db, run):
            for key in ('constraints', 'criteria'):
                run[key] = json.loads(run[key])
            run['members'] = [dict(r) for r in db.execute('SELECT * FROM members')]
            run['usage'] = sum(m['status'] in ('live', 'reserved') for m in run['members'])
            run['tasks'] = [self._task(db, r[0]) for r in db.execute('SELECT id FROM tasks')]
            run['files'] = [dict(r) for r in db.execute('SELECT * FROM files ORDER BY path')]
            run['evidence'] = [dict(r) for r in db.execute('SELECT * FROM evidence')]
            run['events'] = list(reversed([dict(r) for r in db.execute('SELECT * FROM events ORDER BY id DESC LIMIT 100')]))
            run['counts'] = {status: sum(t['status'] == status for t in run['tasks'])
                             for status in ('ready', 'claimed', 'blocked', 'submitted', 'completed')}
            return run

    def _bootstrap(self, pid, started, socket, reservation=None):
        with self.store.transaction(create=True) as db:
            if not db.execute('SELECT 1 FROM run').fetchone():
                db.execute('INSERT INTO run VALUES(?,?,?,?,?,?,?,?,?)',
                           (uuid.uuid4().hex, '', '[]', '[]', self.member, self.member, 10, 0, 'setup'))
                db.execute("INSERT INTO members VALUES(?,?,'live',?,?,?)",
                           (self.member, uuid.uuid4().hex, pid, started, socket))
                self.store.event(db, 'container_setup', {'member': self.member})
        return self._join(reservation, pid, started, socket)

    def _admit(self, member, reservation):
        """Harness-only reservation, including nested launches. Never called by board users."""
        bounded(member, 'member', 128)
        with self.store.operation(active=False) as (db, run):
            if run['status'] not in ('setup', 'running') or (run['status'] == 'running' and run['deadline'] <= time.time()):
                raise SwarmError(f"run is {run['status']}; no new admission")
            prior = db.execute('SELECT * FROM members WHERE id=?', (member,)).fetchone()
            if prior and prior['reservation'] == reservation and prior['status'] != 'dead':
                return dict(prior)
            usage = db.execute("SELECT count(*) FROM members WHERE status IN ('live','reserved')").fetchone()[0]
            if usage >= run['member_limit']:
                raise SwarmError(f"swarm limit {run['member_limit']}, current usage {usage}; reuse the existing pool")
            if prior:
                raise SwarmError('member identity already used; choose a stable new identity')
            db.execute("INSERT INTO members VALUES(?,?,'reserved',NULL,NULL,NULL)", (member, reservation))
            self.store.event(db, 'reserved', {'member': member})
            return {'id': member, 'reservation': reservation, 'status': 'reserved'}

    def _activate(self, member, reservation, pid, started, socket):
        with self.store.operation(active=False) as (db, run):
            if run['status'] not in ('setup', 'running'):
                raise SwarmError('run stopped before activation')
            row = db.execute('SELECT * FROM members WHERE id=?', (member,)).fetchone()
            if not row or row['reservation'] != reservation or row['status'] == 'dead':
                raise SwarmError('unknown or stale launch reservation')
            if row['status'] == 'live' and row['pid'] is not None and (row['pid'], row['started']) != (pid, started):
                raise SwarmError('member already active in a different process')
            db.execute("UPDATE members SET status='live',pid=?,started=?,socket=? WHERE id=?", (pid, started, socket, member))
            self.store.event(db, 'activated', {'member': member, 'pid': pid})

    def _join(self, reservation, pid, started, socket):
        return join_process(self, reservation, pid, started, socket)

    def _socket(self, socket):
        with self.store.operation(active=False) as (db, _):
            db.execute('UPDATE members SET socket=? WHERE id=?', (socket, self.member))

    def _record_launch(self, member, reservation, pid, started):
        with self.store.operation(active=False) as (db, _):
            row = db.execute('SELECT * FROM members WHERE id=? AND reservation=?', (member, reservation)).fetchone()
            if not row or row['status'] == 'dead':
                raise SwarmError('stale launch reservation')
            if row['pid'] is not None and (row['pid'], row['started']) != (pid, started):
                raise SwarmError('conflicting launch identity')
            db.execute('UPDATE members SET pid=?,started=? WHERE id=?', (pid, started, member))

    def _confirmed_dead(self, member):
        # The Rust lifecycle adapter calls this only after observing process
        # death (PID + kernel start time), never on idle, timeout or self-report.
        with self.store.operation(active=False) as (db, _):
            current = db.execute('SELECT status FROM members WHERE id=?', (member,)).fetchone()
            if not current or current['status'] == 'dead':
                return
            db.execute("UPDATE members SET status='dead' WHERE id=?", (member,))
            db.execute("UPDATE tasks SET status='blocked',blocker='worker death confirmed; coordinator recovery required' WHERE owner=? AND status IN ('claimed','blocked','submitted')", (member,))
            db.execute('DELETE FROM files WHERE owner=?', (member,))
            self.store.event(db, 'death_confirmed', {'member': member})

    def send(self, request, recipient, body):
        bounded(body, 'message', 8192)
        with self.store.operation() as (db, _):
            def send():
                target = db.execute('SELECT status FROM members WHERE id=?', (recipient,)).fetchone()
                if not target or target['status'] == 'dead':
                    raise SwarmError('unknown or out-of-swarm recipient')
                count = db.execute("SELECT count(*) FROM messages WHERE recipient=? AND status='accepted'", (recipient,)).fetchone()[0]
                if count >= 100:
                    raise SwarmError('recipient inbox full (100 unconsumed messages)')
                cursor = db.execute("INSERT INTO messages(sender,recipient,body,status) VALUES(?,?,?,'accepted')",
                                    (self.member, recipient, body))
                self.store.event(db, 'message_accepted', {'message': cursor.lastrowid, 'recipient': recipient})
                return {'id': cursor.lastrowid, 'status': 'accepted'}
            return self.store.retry(db, request, ['send', recipient, body], send)

    def inbox(self, include_consumed=False):
        with self.store.operation(active=False) as (db, _):
            return [dict(r) for r in db.execute(
                "SELECT * FROM messages WHERE recipient=? AND (status='accepted' OR ?) ORDER BY id LIMIT 100",
                (self.member, include_consumed))]

    def ack(self, message_id):
        with self.store.operation(active=False) as (db, _):
            row = db.execute('SELECT * FROM messages WHERE id=? AND recipient=?', (message_id, self.member)).fetchone()
            if not row:
                raise SwarmError('unknown message in own inbox')
            if row['status'] != 'consumed':
                db.execute("UPDATE messages SET status='consumed' WHERE id=?", (message_id,))
                self.store.event(db, 'message_consumed', {'message': message_id})

    def evidence(self, criterion, artifact, revision, kind, passed):
        bounded(artifact, 'artifact reference', 2048)
        bounded(revision, 'artifact revision', 256)
        with self.store.operation() as (db, run):
            definition = next((c for c in json.loads(run['criteria']) if c['id'] == criterion), None)
            if not definition or definition['kind'] != kind:
                raise SwarmError('evidence must match a configured criterion and kind')
            accepted = self.member == run['coordinator'] and passed is True
            db.execute('INSERT OR REPLACE INTO evidence VALUES(?,?,?,?,?,?)',
                       (criterion, artifact, revision, kind, self.member, accepted))
            self.store.event(db, 'evidence', {'criterion': criterion, 'artifact': artifact, 'revision': revision, 'accepted': accepted})

    def complete(self, revision):
        with self.store.operation(coordinator=True) as (db, run):
            for criterion in json.loads(run['criteria']):
                if not db.execute('SELECT 1 FROM evidence WHERE criterion=? AND revision=? AND accepted=1',
                                  (criterion['id'], revision)).fetchone():
                    raise SwarmError('completion requires accepted evidence at the current revision for every criterion')
            if db.execute("SELECT 1 FROM tasks WHERE status!='completed'").fetchone() or db.execute('SELECT 1 FROM files').fetchone():
                raise SwarmError('settle outstanding work and file reservations before success')
            for row in db.execute('SELECT evidence FROM tasks'):
                if any(e['revision'] != revision for e in json.loads(row[0])):
                    raise SwarmError('task evidence refers to stale revision')
            db.execute("UPDATE run SET status='succeeded'")
            self.store.event(db, 'completed', {'revision': revision})

    def stop(self, status, reason):
        if status not in ('blocked', 'failed', 'cancelled', 'budget-exhausted'):
            raise SwarmError('invalid non-success outcome')
        bounded(reason, 'stop reason')
        with self.store.operation(active=False, coordinator=True) as (db, run):
            if run['status'] != 'running':
                if run['status'] != status:
                    raise SwarmError(f"run already {run['status']}")
                return
            db.execute('UPDATE run SET status=?', (status,))
            self.store.event(db, 'stop', {'status': status, 'reason': reason})


board = None  # bound by the compiled harness bootstrap, never PYTHONPATH

# Lifecycle entry point is deliberately separate from supported worker methods.
def join_process(context, reservation, pid, started, socket):
    with context.store.transaction() as db:
        run = db.execute('SELECT coordinator FROM run').fetchone()
        if not run:
            raise SwarmError('coordination run missing')
        existing = db.execute('SELECT * FROM members WHERE id=?', (context.member,)).fetchone()
    coordinator = Workbench(context.store.path, context.checkout, run['coordinator'])
    if existing is None:
        reservation = reservation or uuid.uuid4().hex
        coordinator._admit(context.member, reservation)
    elif existing['status'] == 'live' and existing['pid'] == pid and existing['started'] == started:
        return coordinator.summary()
    elif reservation != existing['reservation']:
        raise SwarmError('launch reservation does not match invoking process')
    coordinator._activate(context.member, reservation, pid, started, socket)
    return coordinator.summary()

"""Cooperative swarm API. The harness binds member identity and owns lifecycle.

Use `from swarm import board`. Do not construct clients or edit SQLite directly
in agent programs. These conventions are correctness guarantees, not a security
boundary against arbitrary code running as the same container user.
"""
from __future__ import annotations
import json
import time
import uuid
from swarm_policy import OWNER_IDLE_AFTER, idle_transition
from swarm_repository import lost_after_activation, member_claim_counts
from swarm_store import Store, SwarmError, bounded, encode
from swarm_tasks import Tasks
from swarm_use_cases import Coordination


class Workbench(Tasks):
    def __init__(self, path, checkout, member):
        # One injectable clock: event timestamps, expiry and the loss grace
        # all read `self.coordination.clock` (tests rebind it).
        self.store = Store(path, member, lambda: self.coordination.clock())
        self.checkout, self.member = checkout, member
        self.coordination = Coordination(self.store, member, time.time)

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
        if type(member_limit) is not int or not 1 <= member_limit <= 25:
            raise SwarmError('member limit must be 1 through 25 including coordinator')
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
                db.execute('INSERT INTO run(id,goal,constraints,criteria,coordinator,integrator,member_limit,deadline,status) VALUES(?,?,?,?,?,?,?,?,?)',
                           (uuid.uuid4().hex, goal, encode(constraints), encode(criteria), self.member,
                            self.member, member_limit, deadline, 'running'))
                db.execute("INSERT INTO members(id,reservation,status,pid,started,socket) VALUES(?,?,'live',NULL,NULL,NULL)", (self.member, uuid.uuid4().hex))
            self.store.event(db, 'created', {'goal': goal, 'deadline': deadline, 'contract': {'goal': goal, 'constraints': constraints, 'criteria': criteria}})
        return self.summary()

    def amend(self, goal, constraints, criteria, reason):
        bounded(goal, 'goal')
        bounded(reason, 'amendment reason')
        bounded(encode(constraints), 'constraints')
        with self.store.operation(coordinator=True) as (db, run):
            self._criteria(criteria)
            db.execute('UPDATE run SET goal=?,constraints=?,criteria=?', (goal, encode(constraints), encode(criteria)))
            db.execute('DELETE FROM evidence')
            self.store.event(db, 'amended', {'previous_goal': run['goal'], 'goal': goal, 'reason': reason,
                'before': {'goal': run['goal'], 'constraints': json.loads(run['constraints']), 'criteria': json.loads(run['criteria'])},
                'after': {'goal': goal, 'constraints': constraints, 'criteria': criteria}})

    def usage_budget(self, token_limit, strict_unknown=True):
        return self.coordination.usage_budget(token_limit, strict_unknown)

    def usage_report(self):
        return self.coordination.usage_report()

    def _record_request(self, record):
        return self.coordination.record_request(record)

    def _control_status(self):
        return self.coordination.control_status()

    def _request_admission(self):
        return self.coordination.request_admission()

    def pause(self, reason):
        bounded(reason, 'pause reason')
        return self.coordination.pause(reason)

    def resume(self):
        return self.coordination.resume(external=False)

    def _resume_external(self):
        """Harness-only: the supervisor outside the swarm resumes the run (#1729)."""
        return self.coordination.resume(external=True)

    def _close(self):
        """Harness-only: the supervisor makes the held outcome terminal (#1729)."""
        return self.coordination.close(external=True)

    def _extend_deadline(self, seconds):
        """Harness-only: the supervisor grants wall-clock budget (#1729)."""
        return self.coordination.extend_deadline(seconds, external=True)

    def _accept_wake(self, generation):
        return self.coordination.accept_wake(generation)

    def _notifications(self, with_generation=False):
        return self.coordination.notifications(with_generation)

    def _snapshot(self):
        with self.store.operation(active=False, read_only=True) as (db, run):
            return {'status': run['status'], 'coordinator': run['coordinator'],
                    'outcome': run['outcome'],
                    'control_generation': db.execute("SELECT coalesce(max(id),0) FROM events WHERE action IN ('paused','resumed')").fetchone()[0],
                    'deadline': run['deadline'],
                    'members': [dict(r) for r in db.execute('SELECT * FROM members')]}

    def summary(self, since=None):
        if since is not None and (type(since) is not int or since < 0):
            raise SwarmError('summary cursor must be a nonnegative integer')
        with self.store.operation(active=False, read_only=True) as (db, run):
            cursor = db.execute('SELECT coalesce(max(id),0) FROM events').fetchone()[0]
            next_check, crossed = self._liveness_watch(db, cursor)
            if since == cursor and not crossed:
                return {'unchanged': True, 'event_cursor': cursor, 'status': run['status'],
                        'next_liveness_check_at': next_check}
            run['next_liveness_check_at'] = next_check
            for key in ('constraints', 'criteria'):
                run[key] = json.loads(run[key])
            run['members'] = [dict(r) for r in db.execute('SELECT * FROM members')]
            run['usage'] = sum(m['status'] in ('live', 'reserved') for m in run['members'])
            run['task_count'] = db.execute('SELECT count(*) FROM tasks').fetchone()[0]
            run['tasks'] = self._with_owner_liveness(db, [self._task(db, r[0]) for r in db.execute('SELECT id FROM tasks ORDER BY id LIMIT 50')])
            run['file_count'] = db.execute('SELECT count(*) FROM files').fetchone()[0]
            run['files'] = [dict(r) for r in db.execute('SELECT * FROM files ORDER BY path LIMIT 50')]
            run['evidence'] = [dict(r) for r in db.execute('SELECT * FROM evidence')]
            run['control_generation'] = db.execute("SELECT coalesce(max(id),0) FROM events WHERE action IN ('paused','resumed')").fetchone()[0]
            run['event_cursor'] = db.execute('SELECT coalesce(max(id),0) FROM events').fetchone()[0]
            run['counts'] = {status: 0 for status in ('ready', 'claimed', 'blocked', 'submitted', 'completed')}
            states = {r['id']: dict(r) for r in db.execute('SELECT id,status,dependencies FROM tasks')}
            for task in states.values():
                status = task['status']
                if status == 'ready' and any(states[d]['status'] != 'completed' for d in json.loads(task['dependencies'])):
                    status = 'blocked'
                run['counts'][status] += 1
            run['counts'].update(member_claim_counts(db, run['coordinator']))
            return run

    def _liveness_watch(self, db, cursor):
        """Read side (#1969): `(next_liveness_check_at, crossed)` over every
        owned task. `next_liveness_check_at` is the store-clock instant the
        earliest still-active owner turns idle (None when no owner will), so a
        cursor caller knows when to look again. `crossed` is whether an owner
        is idle now but was not at the time of event `cursor`: the transition
        happened by clock alone, with no event to move the cursor, so an
        unchanged fast path must not hide it. Nothing is written."""
        owned = [dict(r) for r in db.execute(
            "SELECT id, owner, status FROM tasks WHERE owner IS NOT NULL AND status IN ('claimed','blocked','submitted')")]
        if not owned:
            return None, False
        now = self.store.clock()
        row = db.execute('SELECT time FROM events WHERE id=?', (cursor,)).fetchone()
        cursor_time = row['time'] if row else 0.0
        next_check, crossed = None, False
        for last, state in self._owner_views(db, owned, now):
            if state == 'idle' and last + OWNER_IDLE_AFTER > cursor_time:
                crossed = True
            elif state == 'active':
                at = idle_transition(last, now)
                next_check = at if next_check is None else min(next_check, at)
        return next_check, crossed

    def events(self, after=0, limit=25):
        """Read immutable audit history explicitly, using durable event IDs."""
        if type(after) is not int or after < 0 or type(limit) is not int or not 1 <= limit <= 100:
            raise SwarmError('event page requires nonnegative cursor and limit 1 through 100')
        with self.store.operation(active=False, read_only=True) as (db, _):
            rows = [dict(row) for row in db.execute(
                'SELECT * FROM events WHERE id>? ORDER BY id LIMIT ?', (after, limit + 1))]
            page = rows[:limit]
            return {'events': page, 'cursor': page[-1]['id'] if page else after,
                    'has_more': len(rows) > limit}

    def _status(self):
        """Harness-only, membership-free: has a run been created in this container?
        The bootstrap placeholder carries deadline 0; `create` requires a future one.
        Carries the #1969 membership counts (`members_without_claim`, `members_dead`)
        and the run's id, which the supervising session names when it keeps
        a box for an unfinished run (#2033)."""
        with self.store.transaction() as db:
            row = db.execute('SELECT id, status, deadline, coordinator, outcome FROM run').fetchone()
            counts = member_claim_counts(db, row['coordinator'] if row else None)
        return dict(counts, id=row['id'] if row else None,
                    status=row['status'] if row else 'setup', deadline=row['deadline'] if row else 0,
                    coordinator=row['coordinator'] if row else None,
                    outcome=row['outcome'] if row else None)

    def _bootstrap(self, pid, started, socket, reservation=None):
        with self.store.transaction(create=True) as db:
            if not db.execute('SELECT 1 FROM run').fetchone():
                db.execute('INSERT INTO run(id,goal,constraints,criteria,coordinator,integrator,member_limit,deadline,status) VALUES(?,?,?,?,?,?,?,?,?)',
                           (uuid.uuid4().hex, '', '[]', '[]', self.member, self.member, 10, 0, 'setup'))
                db.execute("INSERT INTO members(id,reservation,status,pid,started,socket) VALUES(?,?,'live',?,?,?)",
                           (self.member, uuid.uuid4().hex, pid, started, socket))
                self.store.event(db, 'container_setup', {'member': self.member})
        return self._join(reservation, pid, started, socket)

    def _admit(self, member, reservation):
        """Harness-only reservation, including nested launches. Never called by board users."""
        bounded(member, 'member', 128)
        return self.coordination.reserve_member(member, reservation)

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

    def _release_unlaunched(self, member):
        with self.store.operation(active=False) as (db, _):
            row = db.execute('SELECT * FROM members WHERE id=?', (member,)).fetchone()
            if not row or row['status'] != 'reserved' or row['pid'] is not None:
                raise SwarmError('only an unlaunched reservation may be released')
            db.execute("UPDATE members SET status='dead' WHERE id=?", (member,))
            self.store.event(db, 'launch_abandoned', {'member': member})

    ABRUPT_BLOCKER = 'worker death confirmed (abrupt exit; reservations retained); coordinator recovery required'
    ORDERLY_BLOCKER = 'worker death confirmed; coordinator recovery required'

    def _confirmed_dead(self, member, exit='orderly'):
        """Harness-only (#1961): the harness that launched `member` reaped its
        owned process. The member is dead and its active tasks block for
        `recover`; idle time, a lost socket, a timeout or a self-report never
        confer this authority (those stay `_quarantine`).

        `exit` says how it ended. `orderly` (a delegated kill, a protocol
        shutdown, an exit the harness chose): the member's own teardown ended
        its tool process groups, so its file reservations are released.
        `abrupt` (a signal nobody here sent, or an unobservable end): Bash
        tool children run in their own process groups and may outlive the
        harness, still writing to the reserved paths, so the reservations are
        retained and the tasks say so; only the coordinator frees them, with
        `revoke(id, reason)` or `recover(id, release_files=True)`."""
        if exit not in ('orderly', 'abrupt'):
            raise SwarmError('exit kind must be orderly or abrupt')
        with self.store.operation(active=False) as (db, run):
            current = db.execute('SELECT status FROM members WHERE id=?', (member,)).fetchone()
            if not current or current['status'] == 'dead':
                return
            db.execute("UPDATE members SET status='dead' WHERE id=?", (member,))
            blocker = self.ORDERLY_BLOCKER if exit == 'orderly' else self.ABRUPT_BLOCKER
            db.execute("UPDATE tasks SET status='blocked',blocker=? WHERE owner=? AND status IN ('claimed','blocked','submitted')", (blocker, member))
            retained = db.execute('SELECT count(*) FROM files WHERE owner=?', (member,)).fetchone()[0]
            if exit == 'orderly':
                db.execute('DELETE FROM files WHERE owner=?', (member,))
                retained = 0
            detail = {'member': member, 'exit': exit, 'reservations_retained': retained}
            if retained:
                detail['reason'] = 'abrupt harness exit; orphaned tool processes may still write reserved paths'
            self.store.event(db, 'death_confirmed', detail)
            if member == run['coordinator']:
                self._end_by_loss(db, run, 'coordinator death confirmed')

    def send(self, request: str, recipient: str, body: str, revision: str = None, supersedes: int = None):
        """Send a durable string message (8192 UTF-8 bytes maximum), not a dict.

        `revision` (optional) names the revision the message is about, visible
        in the recipient's inbox without parsing the body. `supersedes`
        (optional) retires your own earlier unread message to the same
        recipient as `superseded` in the same transaction (#1837). Both are
        vocabulary, not a required practice. Sending needs a running run;
        `withdraw` and `ack` are bookkeeping and also work while paused."""
        bounded(body, 'message', 8192)
        if revision is not None:
            bounded(revision, 'message revision', 256)
        if supersedes is not None and (type(supersedes) is not int or supersedes < 1):
            raise SwarmError('supersedes must be a message id')
        with self.store.operation() as (db, _):
            def send():
                target = db.execute('SELECT status FROM members WHERE id=?', (recipient,)).fetchone()
                if not target or target['status'] == 'dead':
                    raise SwarmError('unknown or out-of-swarm recipient')
                if supersedes is not None:
                    self._retire(db, supersedes, recipient, 'superseded')
                count = db.execute("SELECT count(*) FROM messages WHERE recipient=? AND status='accepted'", (recipient,)).fetchone()[0]
                if count >= 100:
                    raise SwarmError('recipient inbox full (100 unconsumed messages)')
                cursor = db.execute("INSERT INTO messages(sender,recipient,body,status,revision,supersedes) VALUES(?,?,?,'accepted',?,?)",
                                    (self.member, recipient, body, revision, supersedes))
                if supersedes is not None:
                    db.execute('UPDATE messages SET superseded_by=? WHERE id=?', (cursor.lastrowid, supersedes))
                    self.store.event(db, 'message_superseded', {'message': supersedes, 'superseded_by': cursor.lastrowid})
                self.store.event(db, 'message_accepted', {'message': cursor.lastrowid, 'recipient': recipient, 'revision': revision})
                return {'id': cursor.lastrowid, 'status': 'accepted'}
            # Keep the pre-#1837 payload shape for plain sends so request keys
            # recorded by an older build still replay against this store.
            payload = ['send', recipient, body]
            if revision is not None or supersedes is not None:
                payload += [revision, supersedes]
            return self.store.retry(db, request, payload, send)

    def withdraw(self, message_id):
        """Withdraw your own unread message: it leaves the recipient's inbox and
        the wake path but stays in the audit as `withdrawn` (#1837). Repeating
        a withdrawal is a no-op, like repeating an acknowledgment."""
        message_id = self._message_id(message_id)
        with self.store.operation(active=False) as (db, _):
            if self._retire(db, message_id, None, 'withdrawn'):
                self.store.event(db, 'message_withdrawn', {'message': message_id})

    @staticmethod
    def _message_id(value):
        if type(value) is not int or value < 1:
            raise SwarmError('message id must be a positive integer')
        return value

    def _retire(self, db, message_id, recipient, status):
        """Move your own unread message to `status`; False when it already is."""
        row = db.execute('SELECT * FROM messages WHERE id=?', (message_id,)).fetchone()
        if not row or row['sender'] != self.member:
            raise SwarmError(f'only your own message can be {status}')
        if recipient is not None and row['recipient'] != recipient:
            raise SwarmError(f'only a message to the same recipient can be {status}')
        if row['status'] == status and status == 'withdrawn':
            return False
        if row['status'] != 'accepted':
            raise SwarmError(f"message {message_id} is already {row['status']}")
        db.execute('UPDATE messages SET status=? WHERE id=?', (status, message_id))
        return True

    def inbox(self, include_consumed=False):
        """Unread messages to you; `include_consumed=True` adds the audit of
        consumed, superseded and withdrawn ones."""
        with self.store.operation(active=False, read_only=True) as (db, _):
            return [dict(r) for r in db.execute(
                "SELECT * FROM messages WHERE recipient=? AND (status='accepted' OR ?) ORDER BY id LIMIT 100",
                (self.member, include_consumed))]

    def ack(self, message_id):
        message_id = self._message_id(message_id)
        with self.store.operation(active=False) as (db, _):
            row = db.execute('SELECT * FROM messages WHERE id=? AND recipient=?', (message_id, self.member)).fetchone()
            if not row:
                raise SwarmError('unknown message in own inbox')
            if row['status'] == 'accepted':
                db.execute("UPDATE messages SET status='consumed' WHERE id=?", (message_id,))
                self.store.event(db, 'message_consumed', {'message': message_id})

    def evidence(self, criterion: str, artifact: str, revision: str, kind: str, passed: bool):
        """Record a worker proposal (accepted=0), or coordinator acceptance if passed=True."""
        bounded(artifact, 'artifact reference', 2048)
        bounded(revision, 'artifact revision', 256)
        with self.store.operation() as (db, run):
            definition = next((c for c in json.loads(run['criteria']) if c['id'] == criterion), None)
            if not definition or definition['kind'] != kind:
                raise SwarmError('evidence must match a configured criterion and kind')
            accepted = self.member == run['coordinator'] and passed is True
            previous = db.execute('SELECT artifact,revision,kind,accepted FROM evidence WHERE criterion=? AND actor=?',
                                  (criterion, self.member)).fetchone()
            if previous is not None and tuple(previous) == (artifact, revision, kind, accepted):
                return
            db.execute('INSERT OR REPLACE INTO evidence VALUES(?,?,?,?,?,?)',
                       (criterion, artifact, revision, kind, self.member, accepted))
            self.store.event(db, 'evidence', {'criterion': criterion, 'artifact': artifact, 'revision': revision, 'accepted': accepted})

    def complete(self, revision):
        return self.coordination.complete(revision)

    def revalidate_task(self, task_id, revision, evidence):
        bounded(encode(evidence), 'evidence references')
        return self.coordination.revalidate_task(task_id, revision, evidence)

    LOSS_GRACE = 10.0

    def _quarantine(self, member):
        """Harness-only: this member's harness observed `member`'s pid gone.

        A missing harness is not proof that its independent execution groups
        stopped, so ownership is kept until the environment is discarded.
        Observer authority (#1961): only the harness that launched `member`
        (its reservation's actor) may record its loss, and it does so only
        after a grace, because its owned-handle reaper normally confirms the
        death first (`_confirmed_dead`) and a confirmed death never pauses
        the run. Another member's reconcile observes but records nothing while
        the launcher lives; once the launcher is itself dead or lost, any
        member may record the loss after the same grace so the store never
        stalls. A member without a launcher (the bootstrapped coordinator)
        is recorded at once, as before (#1924 records the coordinator from
        outside). Idempotent per member: a member already recorded lost or
        confirmed dead never pauses the run again."""
        with self.store.operation(active=False) as (db, run):
            if self._lost(db, member):
                return
            row = db.execute('SELECT launcher FROM members WHERE id=?', (member,)).fetchone()
            launcher = row['launcher'] if row else None
            if launcher is not None:
                if self.member != launcher and not self._lost(db, launcher):
                    return
                if not self._grace_elapsed(db, member):
                    return
            self.store.event(db, 'scope_unknown', {'member': member,
                'reason': 'harness exited; execution scope unconfirmed; discard environment'})
            self._end_by_loss(db, run, 'harness exited; execution scope unconfirmed')

    def _grace_elapsed(self, db, member):
        """Record the first observation of `member`'s vanished harness (one
        `scope_observed` event per observer and member); True once the
        earliest observation is `LOSS_GRACE` seconds old."""
        now = self.coordination.clock()
        first = None
        for event in db.execute("SELECT actor, time, detail FROM events WHERE action='scope_observed' ORDER BY id"):
            if json.loads(event['detail']).get('member') != member:
                continue
            if first is None:
                first = event['time']
            if event['actor'] == self.member:
                break
        else:
            self.store.event(db, 'scope_observed', {'member': member})
            if first is None:
                first = now
        return now - first >= self.LOSS_GRACE

    @staticmethod
    def _lost(db, member):
        """Whether `member` is dead, or was quarantined after its latest activation."""
        row = db.execute('SELECT status FROM members WHERE id=?', (member,)).fetchone()
        if row is None or row['status'] == 'dead':
            return True
        return lost_after_activation(db, member)

    def _lose_coordinator(self):
        """Harness-only (#1924): the supervising session outside the container
        lost this coordinator's socket. In ONE store operation (so the expiry
        block and any concurrent end are observed first): a run that already
        ended, closed or was cancelled is left alone; a running run, or a pause
        holding no outcome, is quarantined as a lost harness. Returns the
        resulting run state and whether a loss was recorded."""
        with self.store.operation(active=False) as (db, run):
            ended = run['status'] not in ('setup', 'running') and not (
                run['status'] == 'paused' and not run.get('outcome'))
            lost = False
            if not ended:
                self.store.event(db, 'scope_unknown', {'member': self.member,
                    'reason': 'harness exited; execution scope unconfirmed; discard environment'})
                self._end_by_loss(db, run, 'harness exited; execution scope unconfirmed')
                lost = True
            row = db.execute('SELECT id, status, outcome, deadline, coordinator FROM run').fetchone()
            return {'id': row['id'], 'status': row['status'], 'outcome': row['outcome'],
                    'deadline': row['deadline'], 'coordinator': row['coordinator'], 'lost': lost}

    def _end_by_loss(self, db, run, reason):
        """A lost harness ends a running run as a pause holding `failed` (#1729);
        the supervisor decides whether to close it. A run already paused keeps
        its pause start (the frozen budget) and any verdict the coordinator had
        proposed; only an outcome-less pause takes `failed`. A setup placeholder
        (no run created) just fails."""
        if run['status'] == 'setup':
            db.execute("UPDATE run SET status='failed'")
            self.store.event(db, 'stop', {'status': 'failed', 'reason': reason})
        elif run['status'] == 'running':
            db.execute("UPDATE run SET status='paused', outcome='failed', outcome_reason=?", (reason,))
            self.store.event(db, 'stop', {'status': 'failed', 'reason': reason})
            self.store.event(db, 'paused', {'reason': reason, 'started': time.time(), 'outcome': 'failed'})
        elif run['status'] == 'paused' and not run.get('outcome'):
            db.execute("UPDATE run SET outcome='failed', outcome_reason=?", (reason,))
            self.store.event(db, 'stop', {'status': 'failed', 'reason': reason})

    def stop(self, status, reason):
        """End the run: every status but `cancelled` is a resumable pause holding
        that outcome for the supervisor outside the swarm (#1729)."""
        bounded(reason, 'stop reason')
        return self.coordination.stop(status, reason)


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

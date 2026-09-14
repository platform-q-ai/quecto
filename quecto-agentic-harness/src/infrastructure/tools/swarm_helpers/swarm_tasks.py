"""Task and cooperative checkout use cases over a short transaction port."""
from __future__ import annotations
import json
import pathlib
import uuid
from swarm_policy import ADDRESSABLE_OWNER_STATES, owner_recovery, owner_state, require_unsubmitted
from swarm_repository import lost_members
from swarm_store import SwarmError, bounded, encode


class Tasks:
    def tasks(self, offset=0, limit=50):
        if type(offset) is not int or offset < 0 or type(limit) is not int or not 1 <= limit <= 100:
            raise SwarmError('task page requires nonnegative offset and limit 1 through 100')
        with self.store.operation(active=False, read_only=True) as (db, _):
            page = [self._task(db, row[0]) for row in db.execute('SELECT id FROM tasks ORDER BY id LIMIT ? OFFSET ?', (limit, offset))]
            return self._with_owner_liveness(db, page)

    def file_owners(self, offset=0, limit=50):
        if type(offset) is not int or offset < 0 or type(limit) is not int or not 1 <= limit <= 100:
            raise SwarmError('file page requires nonnegative offset and limit 1 through 100')
        with self.store.operation(active=False, read_only=True) as (db, _):
            return [dict(row) for row in db.execute('SELECT * FROM files ORDER BY path LIMIT ? OFFSET ?', (limit, offset))]

    def task(self, task_id):
        with self.store.operation(active=False, read_only=True) as (db, _):
            return self._with_owner_liveness(db, [self._task(db, task_id)])[0]

    def _with_owner_liveness(self, db, tasks):
        """Read side only (#1969): a claimed, blocked or submitted task carries
        the store's view of its owner: `owner_last_activity`, seconds since the
        owner's most recent board event by the store clock, and `owner_state`
        (`swarm_policy.owner_state`). An owner `send` would accept (active or
        idle) is named as a recipient in `contact`; otherwise `contact` is
        None and `recovery` says how the coordinator moves the work. Three
        bounded queries per call, never per row: the owners' member rows, one
        grouped scan of their events and one scan of loss/activation events.
        Unowned tasks carry none of these fields. Nothing is written."""
        owned = self.owned_tasks(tasks)
        if not owned:
            return tasks
        now = self.store.clock()
        for task, (last, state) in zip(owned, self._owner_views(db, owned, now)):
            task['owner_last_activity'] = None if last is None else max(0.0, now - last)
            task['owner_state'] = state
            if state in ADDRESSABLE_OWNER_STATES:
                task['contact'] = f"board.send(request, {task['owner']!r}, body)"
            else:
                task['contact'] = None
                task['recovery'] = owner_recovery(state)
        return tasks

    @staticmethod
    def owned_tasks(tasks):
        return [t for t in tasks if t['owner'] is not None and t['status'] in ('claimed', 'blocked', 'submitted')]

    @staticmethod
    def _owner_views(db, owned, now):
        """`(last_activity, owner_state)` per owned task, from bounded queries."""
        owners = sorted({t['owner'] for t in owned})
        marks = ','.join('?' * len(owners))
        status = {r['id']: r['status'] for r in db.execute(f'SELECT id,status FROM members WHERE id IN ({marks})', owners)}
        latest = {r['actor']: r['latest'] for r in db.execute(
            f'SELECT actor, max(time) latest FROM events WHERE actor IN ({marks}) GROUP BY actor', owners)}
        lost = lost_members(db, owners)
        return [(latest.get(t['owner']), owner_state(status.get(t['owner']), t['owner'] in lost, latest.get(t['owner']), now))
                for t in owned]

    @staticmethod
    def _task(db, task_id):
        row = db.execute('SELECT * FROM tasks WHERE id=?', (task_id,)).fetchone()
        if row is None:
            raise SwarmError('unknown task')
        result = dict(row)
        for key in ('acceptance', 'dependencies', 'evidence'):
            result[key] = json.loads(result[key])
        if result['status'] == 'ready' and any(
            db.execute('SELECT status FROM tasks WHERE id=?', (d,)).fetchone()[0] != 'completed'
            for d in result['dependencies']
        ):
            result['status'] = 'blocked'
            result['blocker'] = 'unmet dependencies'
        return result

    @staticmethod
    def _dependencies(db, task_id, dependencies):
        if not isinstance(dependencies, list) or len(dependencies) > 100:
            raise SwarmError('dependencies must be a bounded list')
        graph = {row['id']: json.loads(row['dependencies']) for row in db.execute('SELECT * FROM tasks')}
        if any(type(d) is not int or d not in graph or d == task_id for d in dependencies):
            raise SwarmError('invalid, missing or self dependencies')
        graph[task_id] = dependencies
        visited, active = set(), set()
        stack = [(task_id, False)]
        while stack:
            node, leaving = stack.pop()
            if leaving:
                active.remove(node)
                visited.add(node)
            elif node in active:
                raise SwarmError('cyclic dependencies')
            elif node not in visited:
                active.add(node)
                stack.append((node, True))
                stack.extend((child, False) for child in graph.get(node, []))


    def task_create(self, request: str, title: str, acceptance: list[str], dependencies=None):
        """Create idempotently; acceptance is list[str], dependencies are task IDs."""
        bounded(title, 'title', 1024)
        if not isinstance(acceptance, list) or not acceptance or any(not isinstance(a, str) or not a.strip() for a in acceptance):
            raise SwarmError("task acceptance criteria required: use a nonempty list[str], e.g. ['tests pass']")
        bounded(encode(acceptance), 'acceptance')
        dependencies = dependencies or []
        with self.store.operation() as (db, _):
            def create():
                if db.execute('SELECT count(*) FROM tasks').fetchone()[0] >= 1000:
                    raise SwarmError('task board full (1000); settle existing work')
                cursor = db.execute("INSERT INTO tasks(title,acceptance,dependencies,status,evidence) VALUES(?,?,?,'ready','[]')",
                                    (title, encode(acceptance), encode(dependencies)))
                self._dependencies(db, cursor.lastrowid, dependencies)
                self.store.event(db, 'task_created', {'task': cursor.lastrowid})
                return self._task(db, cursor.lastrowid)
            return self.store.retry(db, request, ['task', title, acceptance, dependencies], create)

    def dependencies(self, task_id, dependencies):
        with self.store.operation() as (db, _):
            task = self._task(db, task_id)
            if task['owner'] is not None or task['status'] not in ('ready', 'blocked'):
                raise SwarmError('dependencies may change only before claiming')
            self._dependencies(db, task_id, dependencies)
            db.execute('UPDATE tasks SET dependencies=? WHERE id=?', (encode(dependencies), task_id))
            self.store.event(db, 'dependencies', {'task': task_id, 'dependencies': dependencies})

    def claim(self, task_id):
        with self.store.operation() as (db, _):
            task = self._task(db, task_id)
            if any(self._task(db, d)['status'] != 'completed' for d in task['dependencies']):
                raise SwarmError('unmet dependencies')
            if task['status'] != 'ready':
                raise SwarmError('task is not ready to claim')
            token = uuid.uuid4().hex
            db.execute("UPDATE tasks SET status='claimed',owner=?,token=? WHERE id=?", (self.member, token, task_id))
            self.store.event(db, 'claimed', {'task': task_id, 'token': token})
            return self._task(db, task_id)

    def _owned(self, db, task_id, token):
        task = self._task(db, task_id)
        if task['token'] != token or task['owner'] != self.member or task['status'] not in ('claimed', 'blocked', 'submitted'):
            raise SwarmError('stale or unowned claim')
        return task

    def release(self, task_id, token):
        with self.store.operation() as (db, _):
            self._owned(db, task_id, token)
            db.execute("UPDATE tasks SET status='ready',owner=NULL,token=NULL,blocker=NULL WHERE id=?", (task_id,))
            db.execute('DELETE FROM files WHERE task=? AND claim=?', (task_id, token))
            self.store.event(db, 'released', {'task': task_id})

    def block(self, task_id, token, reason):
        bounded(reason, 'blocker')
        with self.store.operation() as (db, _):
            task = self._owned(db, task_id, token)
            require_unsubmitted(task)
            if task['status'] == 'blocked' and task['blocker'] == reason:
                return
            db.execute("UPDATE tasks SET status='blocked',blocker=? WHERE id=?", (reason, task_id))
            self.store.event(db, 'blocked', {'task': task_id, 'reason': reason})

    def unblock(self, task_id, token, reason):
        """Resume the existing owner's blocked claim after its blocker is resolved."""
        bounded(reason, 'resolution')
        with self.store.operation() as (db, _):
            task = self._owned(db, task_id, token)
            if task['status'] == 'claimed':
                return
            if task['status'] == 'blocked':
                db.execute("UPDATE tasks SET status='claimed',blocker=NULL WHERE id=?", (task_id,))
                self.store.event(db, 'unblocked', {'task': task_id, 'reason': reason})
                return
            raise SwarmError('only blocked or claimed work may resume')

    def submit(self, task_id: int, token: str, evidence: list[dict]):
        """Submit nonempty artifact/revision references; coordinator verification follows."""
        if not isinstance(evidence, list) or not evidence or any(
            not isinstance(e, dict) or not e.get('artifact') or not e.get('revision') for e in evidence
        ):
            raise SwarmError('artifact and revision evidence required')
        bounded(encode(evidence), 'evidence references')
        with self.store.operation() as (db, _):
            task = self._owned(db, task_id, token)
            if task['status'] == 'submitted' and task['evidence'] == evidence:
                return
            require_unsubmitted(task)
            db.execute("UPDATE tasks SET status='submitted',evidence=?,blocker=NULL WHERE id=?", (encode(evidence), task_id))
            self.store.event(db, 'submitted', {'task': task_id})

    def verify_task(self, task_id, token, revision):
        with self.store.operation(coordinator=True) as (db, _):
            task = self._task(db, task_id)
            if task['token'] != token or task['status'] not in ('submitted', 'completed'):
                raise SwarmError('stale claim or work not submitted')
            if any(e['revision'] != revision for e in task['evidence']):
                raise SwarmError('stale evidence revision')
            if task['status'] == 'completed':
                return
            db.execute("UPDATE tasks SET status='completed' WHERE id=?", (task_id,))
            db.execute('DELETE FROM files WHERE task=? AND claim=?', (task_id, token))
            self.store.event(db, 'verified', {'task': task_id, 'revision': revision})

    def recover(self, task_id, release_files=False):
        """Coordinator only: reopen work whose owner's death the harness confirmed
        (its owned process exited, #1961); `revoke` covers a live owner. After
        an abrupt exit the owner's file reservations were retained (orphaned
        tool processes may still write them): pass `release_files=True` to
        free them explicitly, or `revoke(id, reason)` to record why."""
        with self.store.operation(coordinator=True) as (db, _):
            task = self._task(db, task_id)
            if task['status'] not in ('claimed', 'blocked', 'submitted'):
                raise SwarmError('only abandoned active work can be recovered')
            member = db.execute('SELECT status FROM members WHERE id=?', (task['owner'],)).fetchone()
            if not member or member['status'] != 'dead':
                raise SwarmError('recovery requires confirmed worker death; revoke(id, reason) reassigns a live owner')
            retained = db.execute('SELECT count(*) FROM files WHERE task=?', (task_id,)).fetchone()[0]
            if retained and release_files is not True:
                raise SwarmError('reservations retained after an abrupt exit; recover(id, release_files=True) frees them, or revoke(id, reason)')
            self._reopen(db, task_id)
            self.store.event(db, 'recovered', {'task': task_id, 'reservations_released': retained})

    def revoke(self, task_id, reason):
        """Coordinator only (#1961): take a claim back from an owner that will
        not finish (suspended, hung, silent), whether or not it is alive. The
        task returns to `ready` without owner, token, blocker or evidence, its
        file reservations go, the audit records the reason and previous owner,
        and the previous owner gets a board message so it learns on its next
        wake (its owned calls with the old token fail as stale). Revoking an
        unowned task is a no-op returning the task as it stands."""
        bounded(reason, 'revocation reason')
        with self.store.operation(coordinator=True) as (db, _):
            task = self._task(db, task_id)
            if task['owner'] is None and task['status'] in ('ready', 'blocked'):
                return task
            if task['owner'] is None or task['status'] not in ('claimed', 'blocked', 'submitted'):
                raise SwarmError('only claimed, blocked or submitted work can be revoked')
            previous = task['owner']
            self._reopen(db, task_id)
            self.store.event(db, 'revoked', {'task': task_id, 'reason': reason, 'previous_owner': previous})
            self._notify_revoked(db, previous, task_id, reason)
            return self._task(db, task_id)

    @staticmethod
    def _reopen(db, task_id):
        db.execute('DELETE FROM files WHERE task=?', (task_id,))
        db.execute("UPDATE tasks SET status='ready',owner=NULL,token=NULL,blocker=NULL,evidence='[]' WHERE id=?", (task_id,))

    def _notify_revoked(self, db, previous, task_id, reason):
        """Best effort inside the revoke transaction: a dead owner or a full
        inbox loses the message, never the revocation; the event is the audit."""
        target = db.execute('SELECT status FROM members WHERE id=?', (previous,)).fetchone()
        if not target or target['status'] == 'dead':
            return
        count = db.execute("SELECT count(*) FROM messages WHERE recipient=? AND status='accepted'", (previous,)).fetchone()[0]
        if count >= 100:
            return
        body = f'claim on task {task_id} revoked by the coordinator: {reason}'
        cursor = db.execute("INSERT INTO messages(sender,recipient,body,status) VALUES(?,?,?,'accepted')",
                            (self.member, previous, body))
        self.store.event(db, 'message_accepted', {'message': cursor.lastrowid, 'recipient': previous, 'revision': None})

    def reserve(self, task_id: int, token: str, paths: list[str]):
        """Atomically reserve 1–100 paths inside the checkout for this claim."""
        if not isinstance(paths, list) or not paths or len(paths) > 100:
            raise SwarmError('reserve 1 through 100 paths together')
        root = pathlib.Path(self.checkout).resolve()
        normalized = set()
        for path in paths:
            bounded(path, 'path', 4096)
            try:
                normalized.add(str((root / path).resolve().relative_to(root)))
            except (ValueError, OSError, RuntimeError) as error:
                raise SwarmError('file must resolve inside the shared checkout') from error
        with self.store.operation() as (db, _):
            self._owned(db, task_id, token)
            if db.execute('SELECT count(*) FROM files').fetchone()[0] + len(normalized) > 1000:
                raise SwarmError('file reservation board full (1000); release settled work')
            for path in normalized:
                if db.execute('SELECT 1 FROM files WHERE path=?', (path,)).fetchone():
                    raise SwarmError(f'file already reserved: {path}; acquire the entire set or release and retry')
            ownership = uuid.uuid4().hex
            db.executemany('INSERT INTO files VALUES(?,?,?,?,?)',
                           [(path, task_id, self.member, token, ownership) for path in normalized])
            self.store.event(db, 'files_reserved', {'task': task_id, 'paths': sorted(normalized)})
            return {'token': ownership, 'paths': sorted(normalized)}

    def release_files(self, task_id, token, reservation):
        with self.store.operation(active=False) as (db, _):
            self._owned(db, task_id, token)
            db.execute('DELETE FROM files WHERE task=? AND owner=? AND claim=? AND token=?',
                       (task_id, self.member, token, reservation))
            self.store.event(db, 'files_released', {'task': task_id, 'token': reservation})

"""Bounded cooperative SQLite coordination spike; standard library only.

This module never launches agents, executes evidence, or accesses reserved files.
All identities, usage reports and completion attestations are caller supplied.
"""
import concurrent.futures
from contextlib import contextmanager
import json
import math
import os
import sqlite3
import threading
import time
import uuid


class SwarmError(ValueError):
    """Invalid request or terminal/expired swarm."""


def _require(condition, message):
    if not condition:
        raise SwarmError(message)


def _text(value, name):
    _require(isinstance(value, str) and bool(value.strip()), name + ' must be nonempty text')


def _integer(value, name):
    _require(type(value) is int and value >= 0, name + ' must be a nonnegative integer')


class Swarm:
    def __init__(self, path, clock=time.time):
        self.path = os.fspath(path)
        self.clock = clock

    @classmethod
    def create(cls, path, *, goal, done, deadline, coordinator, members, token_budget, clock=time.time):
        _text(goal, 'goal')
        _text(done, 'done')
        _text(coordinator, 'coordinator')
        _integer(token_budget, 'token_budget')
        _require(isinstance(deadline, (int, float)) and not isinstance(deadline, bool)
                 and math.isfinite(deadline) and deadline > clock(), 'deadline must be in the future')
        _require(isinstance(members, (list, tuple)) and 1 <= len(members) <= 10, 'need 1..10 fixed members')
        for member in members:
            _text(member, 'member')
        _require(len(set(members)) == len(members) and coordinator in members, 'members must be unique and include coordinator')
        path = os.fspath(path)
        try:
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        except FileExistsError as exc:
            raise SwarmError('database already exists') from exc
        os.close(fd)
        swarm = cls(path, clock)
        try:
            with swarm._db() as db:
                db.executescript('''
                    CREATE TABLE swarm (id INTEGER PRIMARY KEY CHECK(id=1), goal TEXT NOT NULL,
                        done TEXT NOT NULL, deadline REAL NOT NULL, coordinator TEXT NOT NULL,
                        token_budget INTEGER NOT NULL, tokens_used INTEGER NOT NULL DEFAULT 0,
                        tokens_reserved INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL DEFAULT 'active',
                        completion_evidence TEXT, cancel_reason TEXT);
                    CREATE TABLE members (name TEXT PRIMARY KEY);
                    CREATE TABLE tasks (task_id TEXT PRIMARY KEY, description TEXT NOT NULL,
                        token_limit INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'pending',
                        owner TEXT, claim_token TEXT, tokens_used INTEGER NOT NULL DEFAULT 0);
                    CREATE TABLE dependencies (task_id TEXT NOT NULL REFERENCES tasks(task_id),
                        prerequisite TEXT NOT NULL REFERENCES tasks(task_id), PRIMARY KEY(task_id, prerequisite));
                    CREATE TABLE messages (id INTEGER PRIMARY KEY, sender TEXT NOT NULL,
                        recipient TEXT, body TEXT NOT NULL);
                    CREATE TABLE reservations (path TEXT PRIMARY KEY, owner TEXT NOT NULL);
                    CREATE TABLE evidence (id INTEGER PRIMARY KEY, task_id TEXT NOT NULL,
                        author TEXT NOT NULL, payload TEXT NOT NULL);
                ''')
                db.execute('INSERT INTO swarm(id,goal,done,deadline,coordinator,token_budget) VALUES(1,?,?,?,?,?)',
                           (goal, done, deadline, coordinator, token_budget))
                db.executemany('INSERT INTO members VALUES(?)', [(m,) for m in members])
        except Exception:
            os.unlink(path)
            raise
        return swarm

    @contextmanager
    def _db(self):
        # mode=rw avoids silently creating an empty database on a typo.
        from pathlib import Path
        uri = Path(self.path).absolute().as_uri() + '?mode=rw'
        db = sqlite3.connect(uri, uri=True, timeout=10, isolation_level=None)
        db.row_factory = sqlite3.Row
        db.execute('PRAGMA foreign_keys=ON')
        try:
            db.execute('BEGIN IMMEDIATE')
            yield db
            db.commit()
        except sqlite3.IntegrityError as exc:
            db.rollback()
            raise SwarmError(str(exc)) from exc
        except BaseException:
            db.rollback()
            raise
        finally:
            db.close()

    def _authorize(self, db, actor, *, coordinator=False, allow_expired=False):
        state = db.execute('SELECT * FROM swarm').fetchone()
        _require(db.execute('SELECT 1 FROM members WHERE name=?', (actor,)).fetchone(), 'unknown member')
        if coordinator:
            _require(actor == state['coordinator'], 'coordinator required')
        _require(state['status'] == 'active', 'swarm is terminal')
        _require(allow_expired or self.clock() < state['deadline'], 'deadline exceeded')
        return state

    def add_task(self, actor, task_id, description, *, dependencies=(), token_limit=0):
        _text(task_id, 'task_id')
        _text(description, 'description')
        _integer(token_limit, 'token_limit')
        _require(isinstance(dependencies, (list, tuple)), 'dependencies must be a list or tuple')
        for dep in dependencies:
            _text(dep, 'dependency')
        _require(len(set(dependencies)) == len(dependencies), 'duplicate dependency')
        with self._db() as db:
            self._authorize(db, actor, coordinator=True)
            # Edges only point to previously created tasks: cycles are impossible.
            for dep in dependencies:
                _require(db.execute('SELECT 1 FROM tasks WHERE task_id=?', (dep,)).fetchone(), 'unknown dependency')
            db.execute('INSERT INTO tasks(task_id,description,token_limit) VALUES(?,?,?)', (task_id, description, token_limit))
            db.executemany('INSERT INTO dependencies VALUES(?,?)', [(task_id, d) for d in dependencies])
        return task_id

    def claim(self, actor, task_id=None):
        with self._db() as db:
            state = self._authorize(db, actor)
            task = db.execute('''SELECT * FROM tasks t WHERE status='pending'
                AND (? IS NULL OR task_id=?) AND token_limit <= ?
                AND NOT EXISTS (SELECT 1 FROM dependencies d JOIN tasks p ON p.task_id=d.prerequisite
                    WHERE d.task_id=t.task_id AND p.status!='done') ORDER BY task_id LIMIT 1''',
                (task_id, task_id, state['token_budget'] - state['tokens_used'] - state['tokens_reserved'])).fetchone()
            if task is None:
                return None
            token = uuid.uuid4().hex
            db.execute("UPDATE tasks SET status='claimed',owner=?,claim_token=? WHERE task_id=?", (actor, token, task['task_id']))
            db.execute('UPDATE swarm SET tokens_reserved=tokens_reserved+?', (task['token_limit'],))
            return dict(db.execute('SELECT * FROM tasks WHERE task_id=?', (task['task_id'],)).fetchone())

    def finish(self, actor, task_id, claim_token, *, evidence, tokens_used):
        _integer(tokens_used, 'tokens_used')
        _require(isinstance(evidence, list) and len(evidence) > 0, 'nonempty evidence list required')
        try:
            payload = json.dumps(evidence, sort_keys=True, allow_nan=False)
        except (ValueError, TypeError) as exc:
            raise SwarmError('evidence must be finite JSON') from exc
        with self._db() as db:
            self._authorize(db, actor)
            task = db.execute('SELECT * FROM tasks WHERE task_id=?', (task_id,)).fetchone()
            _require(task is not None and task['status'] == 'claimed' and task['owner'] == actor
                     and task['claim_token'] == claim_token, 'invalid claim')
            _require(tokens_used <= task['token_limit'], 'task token limit exceeded')
            db.execute("UPDATE tasks SET status='done',tokens_used=?,claim_token=NULL WHERE task_id=?", (tokens_used, task_id))
            db.execute('UPDATE swarm SET tokens_reserved=tokens_reserved-?,tokens_used=tokens_used+?', (task['token_limit'], tokens_used))
            db.execute('INSERT INTO evidence(task_id,author,payload) VALUES(?,?,?)', (task_id, actor, payload))

    def message(self, actor, recipient, body):
        _text(body, 'body')
        with self._db() as db:
            self._authorize(db, actor)
            _require(recipient is None or db.execute('SELECT 1 FROM members WHERE name=?', (recipient,)).fetchone(), 'unknown recipient')
            return db.execute('INSERT INTO messages(sender,recipient,body) VALUES(?,?,?)', (actor, recipient, body)).lastrowid

    def messages(self, actor, *, after=0):
        _integer(after, 'after')
        with self._db() as db:
            _require(db.execute('SELECT 1 FROM members WHERE name=?', (actor,)).fetchone(), 'unknown member')
            return [dict(r) for r in db.execute('SELECT * FROM messages WHERE id>? AND (recipient=? OR recipient IS NULL) ORDER BY id', (after, actor))]

    @staticmethod
    def _path(path):
        _text(path, 'path')
        _require(not path.startswith('/') and '\\' not in path and ':' not in path and '\x00' not in path,
                 'use workspace-relative POSIX paths')
        parts = path.split('/')
        _require('..' not in parts, 'parent traversal forbidden')
        normalized = '/'.join(p for p in parts if p not in ('', '.'))
        _require(bool(normalized), 'empty path')
        return normalized

    def reserve(self, actor, path):
        path = self._path(path)
        with self._db() as db:
            self._authorize(db, actor)
            row = db.execute('SELECT owner FROM reservations WHERE path=?', (path,)).fetchone()
            if row:
                return row['owner'] == actor
            db.execute('INSERT INTO reservations VALUES(?,?)', (path, actor))
            return True

    def release(self, actor, path):
        path = self._path(path)
        with self._db() as db:
            self._authorize(db, actor)
            row = db.execute('SELECT owner FROM reservations WHERE path=?', (path,)).fetchone()
            _require(row is not None and row['owner'] == actor, 'reservation owner required')
            db.execute('DELETE FROM reservations WHERE path=?', (path,))

    def complete(self, actor, evidence):
        _text(evidence, 'completion evidence')
        with self._db() as db:
            self._authorize(db, actor, coordinator=True)
            _require(db.execute('SELECT COUNT(*) FROM tasks').fetchone()[0] > 0, 'no tasks')
            _require(db.execute("SELECT COUNT(*) FROM tasks WHERE status!='done'").fetchone()[0] == 0, 'unfinished tasks')
            db.execute("UPDATE swarm SET status='completed',completion_evidence=?", (evidence,))
            db.execute('DELETE FROM reservations')

    def cancel(self, actor, reason):
        _text(reason, 'reason')
        with self._db() as db:
            self._authorize(db, actor, coordinator=True, allow_expired=True)
            db.execute("UPDATE swarm SET status='cancelled',cancel_reason=?,tokens_reserved=0", (reason,))
            db.execute("UPDATE tasks SET status='cancelled',claim_token=NULL WHERE status!='done'")
            db.execute('DELETE FROM reservations')

    def summary(self):
        with self._db() as db:
            result = dict(db.execute('SELECT * FROM swarm').fetchone())
            result['deadline_exceeded'] = self.clock() >= result['deadline']
            result['members'] = [r[0] for r in db.execute('SELECT name FROM members ORDER BY name')]
            for table, order in [('tasks', 'task_id'), ('dependencies', 'task_id,prerequisite'),
                                 ('messages', 'id'), ('reservations', 'path'), ('evidence', 'id')]:
                result[table] = [dict(r) for r in db.execute('SELECT * FROM ' + table + ' ORDER BY ' + order)]
            # Claim tokens are handles, not authentication credentials; omit from diagnostic output.
            for task in result['tasks']:
                task.pop('claim_token')
            return result


class Worker:
    """Member-bound convenience API, not an agent loop or security boundary."""
    def __init__(self, board, member):
        self.board = board if isinstance(board, Swarm) else Swarm(board)
        _text(member, 'member')
        _require(member in self.board.summary()['members'], 'unknown member')
        self.member = member

    def claim(self, task_id=None):
        return self.board.claim(self.member, task_id)

    def finish(self, task_id, claim_token, *, evidence, tokens_used):
        return self.board.finish(self.member, task_id, claim_token,
                                 evidence=evidence, tokens_used=tokens_used)

    def message(self, recipient, body):
        return self.board.message(self.member, recipient, body)

    def messages(self, *, after=0):
        return self.board.messages(self.member, after=after)

    def reserve(self, path):
        return self.board.reserve(self.member, path)

    def release(self, path):
        return self.board.release(self.member, path)

    def summary(self):
        return self.board.summary()


def worker(board, member):
    """Bind a fixed member to a Swarm instance or existing SQLite path."""
    return Worker(board, member)


def demo(path):
    """Safe FAKE-agent demo: two concurrent threads, arithmetic only, new DB only."""
    swarm = Swarm.create(path, goal='Compute sum of squares 1..5 using FAKE workers',
                         done='Coordinator independently verifies every square and total 55',
                         deadline=time.time() + 60, coordinator='coordinator',
                         members=['coordinator', 'fake-left', 'fake-right'], token_budget=0)
    partitions = [('left', 'fake-left', [1, 2, 3]), ('right', 'fake-right', [4, 5])]
    for task, actor, values in partitions:
        swarm.add_task('coordinator', task, 'FAKE arithmetic worker')
        swarm.message('coordinator', actor, json.dumps(values))
    swarm.add_task('coordinator', 'total', 'Verify and sum', dependencies=['left', 'right'])
    barrier = threading.Barrier(2)

    def fake_worker(partition):
        task, actor, values = partition
        worker = Swarm(path)
        claim = worker.claim(actor, task)
        _require(claim is not None, 'demo claim failed')
        barrier.wait(timeout=10)
        assignment = json.loads(worker.messages(actor)[0]['body'])
        _require(assignment == values, 'assignment mismatch')
        evidence = [{'n': n, 'square': n * n} for n in assignment]
        worker.finish(actor, task, claim['claim_token'], evidence=evidence, tokens_used=0)
        worker.message(actor, 'coordinator', 'FAKE worker finished ' + task)

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        list(pool.map(fake_worker, partitions))
    records = {r['task_id']: json.loads(r['payload']) for r in swarm.summary()['evidence']}
    for task, _, values in partitions:
        _require(records[task] == [{'n': n, 'square': n ** 2} for n in values], 'arithmetic verification failed')
    total = sum(item['square'] for rows in records.values() for item in rows)
    _require(total == 55, 'total verification failed')
    claim = swarm.claim('coordinator', 'total')
    swarm.finish('coordinator', 'total', claim['claim_token'], evidence=[{'sum_of_squares': total}], tokens_used=0)
    swarm.complete('coordinator', 'Independently checked n**2 for exactly 1..5; total=55; FAKE workers only')
    return dict(swarm.summary(), fake_agents=True, verified_result=total)

"""SQLite adapter. Transactions never span execution or lifecycle callbacks."""
import contextlib
import json
import sqlite3
import time


from swarm_policy import SwarmError
from swarm_use_cases import Coordination
from swarm_repository import Transaction


SCHEMA = '''
CREATE TABLE IF NOT EXISTS run (id TEXT PRIMARY KEY, goal TEXT, constraints TEXT, criteria TEXT,
 coordinator TEXT, integrator TEXT, member_limit INTEGER, deadline REAL, status TEXT);
CREATE TABLE IF NOT EXISTS members (id TEXT PRIMARY KEY, reservation TEXT UNIQUE, status TEXT,
 pid INTEGER, started TEXT, socket TEXT);
CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, title TEXT, acceptance TEXT, dependencies TEXT,
 status TEXT, owner TEXT, token TEXT, evidence TEXT, blocker TEXT);
CREATE TABLE IF NOT EXISTS files (path TEXT PRIMARY KEY, task INTEGER, owner TEXT, claim TEXT, token TEXT);
CREATE TABLE IF NOT EXISTS messages (id INTEGER PRIMARY KEY, sender TEXT, recipient TEXT, body TEXT, status TEXT);
CREATE TABLE IF NOT EXISTS evidence (criterion TEXT, artifact TEXT, revision TEXT, kind TEXT,
 actor TEXT, accepted INTEGER, PRIMARY KEY(criterion, actor));
CREATE TABLE IF NOT EXISTS requests (actor TEXT, request TEXT, payload TEXT, result TEXT,
 PRIMARY KEY(actor, request));
CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY, actor TEXT, time REAL, action TEXT, detail TEXT);
CREATE INDEX IF NOT EXISTS events_by_actor ON events(actor,id);
CREATE TABLE IF NOT EXISTS notification_cursors (actor TEXT PRIMARY KEY, event INTEGER);
'''
# Columns added after the first release, migrated in place on every open:
# #1729 a paused run may hold the outcome the coordinator proposed; #1837 a
# message may name a revision and supersede an earlier one.
ADDED_COLUMNS = {
    'run': (('outcome', 'TEXT'), ('outcome_reason', 'TEXT')),
    'messages': (('revision', 'TEXT'), ('supersedes', 'INTEGER'), ('superseded_by', 'INTEGER')),
}


def ensure_columns(db):
    for table, columns in ADDED_COLUMNS.items():
        present = {row[1] for row in db.execute(f'PRAGMA table_info({table})')}
        for column, kind in columns:
            if column not in present:
                db.execute(f'ALTER TABLE {table} ADD COLUMN {column} {kind}')


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'))


def bounded(value, label, maximum=8192):
    if not isinstance(value, str) or not value.strip() or len(value.encode()) > maximum:
        raise SwarmError(f'{label} must be nonempty and at most {maximum} bytes')
    return value


class Store:
    def __init__(self, path, actor):
        self.path, self.actor = path, actor

    @contextlib.contextmanager
    def transaction(self, create=False):
        db = None
        try:
            # mode=rw avoids fabricating a fresh board when the store is lost.
            import pathlib
            uri = pathlib.Path(self.path).absolute().as_uri() + ('?mode=rwc' if create else '?mode=rw')
            db = sqlite3.connect(uri, uri=True, timeout=0.5, isolation_level=None)
            db.row_factory = sqlite3.Row
            db.execute('PRAGMA foreign_keys=ON')
            db.execute('BEGIN IMMEDIATE')
            if create:
                for statement in SCHEMA.split(';'):
                    if statement.strip():
                        db.execute(statement)
            ensure_columns(db)
            yield db
            db.commit()
        except sqlite3.Error as error:
            if db:
                db.rollback()
            raise SwarmError(f'coordination store unavailable or contended: {error}') from error
        except BaseException:
            if db:
                db.rollback()
            raise
        finally:
            if db:
                db.close()

    def event(self, db, action, detail):
        db.execute('INSERT INTO events(actor,time,action,detail) VALUES(?,?,?,?)',
                   (self.actor, time.time(), action, encode(detail)))

    @contextlib.contextmanager
    def atomic(self):
        with self.transaction() as db:
            yield Transaction(self, db)

    @contextlib.contextmanager
    def operation(self, active=True, coordinator=False, read_only=False):
        # Compatibility for the SQL-facing dispatch adapters. Policy and atomic
        # use cases see only CoordinationTransaction, never this connection.
        with Coordination(self, self.actor, time.time).operation(active, coordinator, read_only) as tx:
            yield tx.connection, tx.run()

    def retry(self, db, request, payload, action):
        bounded(request, 'request id', 128)
        payload = encode(payload)
        old = db.execute('SELECT * FROM requests WHERE actor=? AND request=?',
                         (self.actor, request)).fetchone()
        if old:
            if old['payload'] != payload:
                raise SwarmError('request id reused with different payload')
            return json.loads(old['result'])
        if db.execute('SELECT count(*) FROM requests').fetchone()[0] >= 10000:
            raise SwarmError('coordination request ledger full (10000)')
        result = action()
        db.execute('INSERT INTO requests VALUES(?,?,?,?)',
                   (self.actor, request, payload, encode(result)))
        return result

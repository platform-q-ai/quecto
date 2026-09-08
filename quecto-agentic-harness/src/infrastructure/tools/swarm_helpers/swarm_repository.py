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

    def event(self, action, detail):
        self.store.event(self.connection, action, detail)

    def members(self):
        return [dict(r) for r in self.connection.execute('SELECT * FROM members')]

    def notification_events(self, actor):
        cursor = self.connection.execute('SELECT event FROM notification_cursors WHERE actor=?', (actor,)).fetchone()
        for row in self.connection.execute('SELECT action,detail FROM events WHERE actor=? AND id>? ORDER BY id',
                                           (actor, cursor[0] if cursor else 0)):
            yield {'action': row['action'], 'detail': json.loads(row['detail'])}

    def advance_notifications(self, actor):
        self.connection.execute('INSERT OR REPLACE INTO notification_cursors VALUES(?,(SELECT coalesce(max(id),0) FROM events))', (actor,))

"""Behavioral contract for the packaged swarm workbench, using real SQLite."""
import concurrent.futures
from contextlib import closing
import json
import importlib.util
import pathlib
import sqlite3
import sys
import tempfile
import time
import unittest

HELPERS = pathlib.Path(__file__).resolve().parents[1] / 'src/infrastructure/tools/swarm_helpers'
sys.path.insert(0, str(HELPERS))
sys.path.insert(0, str(HELPERS.parents[2] / "domain"))
sys.path.insert(0, str(HELPERS.parents[2] / "application"))
from swarm import Workbench, SwarmError


class WorkbenchBehavior(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.db = self.root / 'coordination.sqlite'
        self.parent = self.client('coordinator')
        self.parent.create('ship feature', ['clean architecture'], [
            {'id': 'tests', 'kind': 'command', 'description': 'acceptance tests pass'},
            {'id': 'review', 'kind': 'review', 'description': 'independent review'},
        ], 3, time.time() + 300)
        self.parent._admit('worker', 'reservation-w')
        self.parent._activate('worker', 'reservation-w', 12345, 'start-w', '/tmp/w.sock')
        self.worker = self.client('worker')

    def client(self, member):
        return Workbench(str(self.db), str(self.root), member)

    def task(self, request='task', dependencies=None):
        return self.worker.task_create(request, 'implement behavior', ['tests pass'], dependencies or [])

    def test_reviewed_submission_cannot_be_replaced_under_the_same_claim(self):
        claim = self.worker.claim(self.task()['id'])
        a = [{'artifact': 'reviewed-A', 'revision': 'R1'}]
        b = [{'artifact': 'unreviewed-B', 'revision': 'R1'}]
        self.worker.submit(claim['id'], claim['token'], a)
        reviewed = self.parent.task(claim['id'])
        self.worker.submit(claim['id'], claim['token'], a)  # safe retry
        with self.assertRaisesRegex(SwarmError, 'submitted'):
            self.worker.submit(claim['id'], claim['token'], b)
        with self.assertRaisesRegex(SwarmError, 'submitted'):
            self.worker.block(claim['id'], claim['token'], 'replace via blocked')
        self.parent.verify_task(reviewed['id'], reviewed['token'], 'R1')
        self.assertEqual(self.parent.task(claim['id'])['evidence'], a)

    def test_released_submission_requires_a_new_review_claim(self):
        claim = self.worker.claim(self.task()['id'])
        self.worker.submit(claim['id'], claim['token'], [{'artifact': 'A', 'revision': 'R1'}])
        self.worker.release(claim['id'], claim['token'])
        replacement = self.worker.claim(claim['id'])
        self.worker.submit(claim['id'], replacement['token'], [{'artifact': 'B', 'revision': 'R1'}])
        with self.assertRaisesRegex(SwarmError, 'stale'):
            self.parent.verify_task(claim['id'], claim['token'], 'R1')
        self.parent.verify_task(claim['id'], replacement['token'], 'R1')

    def test_amendment_preserves_the_entire_original_contract(self):
        original = self.parent.summary()
        changed = [{'id': 'tests', 'kind': 'command', 'description': 'replacement test'}]
        self.parent.amend(original['goal'], ['replacement constraint'], changed, 'approved change')
        events = self.parent.summary()['events']
        created = json.loads(next(e['detail'] for e in events if e['action'] == 'created'))
        amended = json.loads(next(e['detail'] for e in events if e['action'] == 'amended'))
        before = {key: original[key] for key in ('goal', 'constraints', 'criteria')}
        self.assertEqual(created['contract'], before)
        self.assertEqual(amended['before'], before)
        self.assertEqual(amended['after'], {'goal': original['goal'], 'constraints': ['replacement constraint'], 'criteria': changed})
        self.assertEqual(amended['reason'], 'approved change')

    def test_dependent_tasks_can_be_revalidated_at_final_revision(self):
        a = self.worker.claim(self.task('a')['id'])
        self.worker.submit(a['id'], a['token'], [{'artifact': 'a-tests', 'revision': 'R1'}])
        self.parent.verify_task(a['id'], a['token'], 'R1')
        b = self.worker.claim(self.task('b', [a['id']])['id'])
        self.worker.submit(b['id'], b['token'], [{'artifact': 'b-tests', 'revision': 'R2'}])
        self.parent.verify_task(b['id'], b['token'], 'R2')
        for criterion, kind in [('tests', 'command'), ('review', 'review')]:
            self.parent.evidence(criterion, 'final-check', 'R2', kind, True)
        with self.assertRaisesRegex(SwarmError, 'stale revision'):
            self.parent.complete('R2')
        evidence = [{'artifact': 'a-rerun-at-R2', 'revision': 'R2'}]
        with self.assertRaisesRegex(SwarmError, 'coordinator'):
            self.worker.revalidate_task(a['id'], 'R2', evidence)
        with self.assertRaisesRegex(SwarmError, 'revision'):
            self.parent.revalidate_task(a['id'], 'R2', a['evidence'] or [{'artifact': 'old', 'revision': 'R1'}])
        self.parent.revalidate_task(a['id'], 'R2', evidence)
        self.parent.complete('R2')
        self.assertEqual(self.parent.summary()['status'], 'succeeded')

    def test_wake_hints_are_targeted_deduplicated_and_terminal_safe(self):
        # Drain any startup hints before the behavior under test.
        self.parent._notifications()
        self.worker._notifications()
        message = self.worker.send('question', 'coordinator', 'Review my result')
        hints = self.worker._notifications()
        self.assertEqual([m['id'] for m in hints], ['coordinator'])
        self.assertEqual(self.worker._notifications(), [])
        self.parent.ack(message['id'])
        self.assertEqual(self.parent._notifications(), [], 'acknowledgment must not wake the pool')
        self.worker.send('late-question', 'coordinator', 'Late result')
        self.parent.stop('blocked', 'report partial progress')
        self.assertEqual(self.worker._notifications(), [], 'terminal state must suppress queued work hints')

    def test_an_observers_read_does_not_broadcast_another_members_changes(self):
        self.parent._notifications()
        self.worker.task_create('new', 'work', ['pass'])
        self.parent.summary()
        self.assertEqual(self.parent._notifications(), [])
        self.assertEqual([m['id'] for m in self.worker._notifications()], ['coordinator'])

    def test_task_acceptance_errors_teach_the_required_type(self):
        for invalid in ('tests pass', [], [42], ['']):
            with self.subTest(invalid=invalid), self.assertRaisesRegex(SwarmError, r'list\[str\]'):
                self.worker.task_create('invalid', 'work', invalid)

    def test_concurrent_notification_claims_do_not_duplicate_hints(self):
        self.worker.send('one', 'coordinator', 'Please review')
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(lambda _: self.client('worker')._notifications(), range(2)))
        self.assertEqual(sum(len(hints) for hints in results), 1)
        self.assertEqual(self.client('worker')._notifications(), [])

    def test_goal_is_durable_and_workers_cannot_amend_it(self):
        self.assertEqual(self.client('worker').summary()['goal'], 'ship feature')
        with self.assertRaisesRegex(SwarmError, 'coordinator'):
            self.worker.amend('wrong', [], [], 'silent change')
        self.parent.amend('new goal', [], [{'id': 'tests', 'kind': 'command', 'description': 'pass'}], 'scope agreed')
        self.assertEqual(self.worker.summary()['goal'], 'new goal')
        self.assertEqual(self.worker.summary()['events'][-1]['actor'], 'coordinator')

    def test_invalid_limits_and_deadlines_are_rejected(self):
        for limit in (0, 11, True):
            with self.subTest(limit=limit), self.assertRaises(SwarmError):
                self.client('coordinator').create('x', [], [{'id':'t','kind':'command','description':'pass'}], limit, time.time() + 1)
        with self.assertRaises(SwarmError):
            self.client('coordinator').create('x', [], [{'id':'t','kind':'command','description':'pass'}], 1, time.time() - 1)

    def test_concurrent_admission_includes_idle_and_reserved_members(self):
        def admit(i):
            try:
                return self.client('worker')._admit(f'child-{i}', f'reserve-{i}')
            except SwarmError:
                return None
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            results = list(pool.map(admit, range(12)))
        self.assertEqual(sum(x is not None for x in results), 1)
        self.assertEqual(self.parent.summary()['usage'], 3)
        with self.assertRaisesRegex(SwarmError, '3.*3.*reuse'):
            self.worker._admit('nested', 'nested')

    def test_claims_are_atomic_and_dependencies_block_claims(self):
        task = self.task()
        dependent = self.task('dependent', [task['id']])
        with self.assertRaisesRegex(SwarmError, 'dependencies'):
            self.worker.claim(dependent['id'])
        def claim(_):
            try:
                return self.client('worker').claim(task['id'])
            except SwarmError:
                return None
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            claims = list(pool.map(claim, range(8)))
        self.assertEqual(sum(c is not None for c in claims), 1)

    def test_dependencies_reject_missing_self_and_cycles(self):
        first = self.task()
        second = self.task('second', [first['id']])
        for deps in ([99999], [first['id']], [second['id']]):
            with self.subTest(deps=deps), self.assertRaises(SwarmError):
                self.worker.dependencies(first['id'], deps)

    def test_request_retries_are_idempotent_but_payload_conflicts_fail(self):
        first = self.task()
        self.assertEqual(self.task()['id'], first['id'])
        self.assertEqual(len(self.worker.summary()['tasks']), 1)
        with self.assertRaisesRegex(SwarmError, 'request'):
            self.worker.task_create('task', 'different', ['pass'], [])

    def test_submission_is_not_completion_and_requires_current_token(self):
        task = self.task()
        claim = self.worker.claim(task['id'])
        with self.assertRaisesRegex(SwarmError, 'claim'):
            self.worker.submit(task['id'], 'stale', [{'artifact': 'report', 'revision': 'abc'}])
        self.worker.submit(task['id'], claim['token'], [{'artifact': 'report', 'revision': 'abc'}])
        self.assertEqual(self.worker.task(task['id'])['status'], 'submitted')
        with self.assertRaisesRegex(SwarmError, 'coordinator'):
            self.worker.verify_task(task['id'], claim['token'], 'abc')
        self.parent.verify_task(task['id'], claim['token'], 'abc')
        self.parent.verify_task(task['id'], claim['token'], 'abc')
        self.assertEqual(self.worker.task(task['id'])['status'], 'completed')

    def test_stale_claim_cannot_modify_recovered_work(self):
        task = self.task()
        old = self.worker.claim(task['id'])
        with self.assertRaisesRegex(SwarmError, 'death'):
            self.parent.recover(task['id'])
        self.parent._confirmed_dead('worker')
        self.parent.recover(task['id'])
        new = self.parent.claim(task['id'])
        self.assertNotEqual(old['token'], new['token'])
        with self.assertRaises(SwarmError):
            self.worker.release(task['id'], old['token'])

    def test_messages_are_durable_bounded_and_idempotent(self):
        message = self.worker.send('m1', 'coordinator', 'blocked on schema')
        self.assertEqual(message, self.worker.send('m1', 'coordinator', 'blocked on schema'))
        self.assertEqual(self.client('coordinator').inbox()[0]['sender'], 'worker')
        self.parent.ack(message['id'])
        self.assertEqual(self.parent.inbox(), [])
        self.assertEqual(self.parent.inbox(include_consumed=True)[0]['status'], 'consumed')
        for recipient, body in [('unknown', 'hello'), ('coordinator', 'x' * 8193)]:
            with self.assertRaises(SwarmError):
                self.worker.send('bad', recipient, body)
        for i in range(100):
            self.worker.send(f'fill-{i}', 'coordinator', 'hello')
        with self.assertRaisesRegex(SwarmError, 'full'):
            self.worker.send('overflow', 'coordinator', 'hello')

    def test_file_reservations_are_atomic_normalized_and_token_owned(self):
        first = self.task()
        second = self.parent.task_create('second', 'other', ['pass'], [])
        a = self.worker.claim(first['id'])
        b = self.parent.claim(second['id'])
        lease = self.worker.reserve(first['id'], a['token'], ['src/../a.rs'])
        with self.assertRaisesRegex(SwarmError, 'reserved'):
            self.parent.reserve(second['id'], b['token'], ['free.rs', './a.rs'])
        self.assertEqual([x['path'] for x in self.parent.summary()['files']], ['a.rs'])
        with self.assertRaises(SwarmError):
            self.worker.release_files(first['id'], 'stale', lease['token'])
        self.worker.release_files(first['id'], a['token'], lease['token'])
        self.assertEqual(self.worker.summary()['files'], [])
        with self.assertRaises(SwarmError):
            self.worker.reserve(first['id'], a['token'], ['../escape'])

    def test_symlink_alias_cannot_bypass_reservation(self):
        (self.root / 'real').mkdir()
        (self.root / 'alias').symlink_to(self.root / 'real', target_is_directory=True)
        a = self.worker.claim(self.task()['id'])
        self.worker.reserve(a['id'], a['token'], ['real/new'])
        b = self.parent.claim(self.parent.task_create('b', 'other', ['pass'], [])['id'])
        with self.assertRaises(SwarmError):
            self.parent.reserve(b['id'], b['token'], ['alias/new'])

    def test_empty_queue_and_submissions_do_not_prove_success(self):
        with self.assertRaisesRegex(SwarmError, 'evidence'):
            self.parent.complete('abc')
        task = self.task()
        c = self.worker.claim(task['id'])
        self.worker.submit(task['id'], c['token'], [{'artifact': 'tests.log', 'revision': 'abc'}])
        self.parent.evidence('tests', 'tests.log', 'abc', 'command', True)
        self.parent.evidence('review', 'review.md', 'abc', 'review', True)
        with self.assertRaisesRegex(SwarmError, 'outstanding'):
            self.parent.complete('abc')
        self.parent.verify_task(task['id'], c['token'], 'abc')
        with self.assertRaisesRegex(SwarmError, 'evidence'):
            self.parent.complete('changed')
        self.parent.complete('abc')
        self.assertEqual(self.parent.summary()['status'], 'succeeded')

    def test_workers_cannot_accept_overall_completion(self):
        with self.assertRaisesRegex(SwarmError, 'coordinator'):
            self.worker.complete('abc')
        self.worker.evidence('tests', 'tests.log', 'abc', 'command', True)
        self.assertFalse(self.parent.summary()['evidence'][0]['accepted'])

    def test_cancellation_and_expiry_prevent_new_work(self):
        self.parent.stop('cancelled', 'user request')
        with self.assertRaisesRegex(SwarmError, 'cancelled'):
            self.task()
        with self.assertRaises(SwarmError):
            self.worker._admit('child', 'r')
        self.assertEqual(self.parent.summary()['status'], 'cancelled')

    def test_budget_expiry_is_distinct_and_keeps_partial_progress(self):
        task = self.task()
        with closing(sqlite3.connect(self.db)) as db:
            db.execute('UPDATE run SET deadline=?', (time.time() - 1,))
            db.commit()
        with self.assertRaisesRegex(SwarmError, 'budget-exhausted'):
            self.worker.claim(task['id'])
        summary = self.parent.summary()
        self.assertEqual(summary['status'], 'budget-exhausted')
        self.assertEqual(len(summary['tasks']), 1)

    def test_coordinator_death_leaves_readable_failed_progress(self):
        self.task()
        self.worker._confirmed_dead('coordinator')
        summary = self.parent.summary()
        self.assertEqual(summary['status'], 'failed')
        self.assertEqual(summary['task_count'], 1)
        with self.assertRaises(SwarmError):
            self.task('after-parent-death')

    def test_summary_is_bounded_and_counts_include_later_pages(self):
        for i in range(55):
            self.task(f'task-{i}')
        summary = self.parent.summary()
        self.assertEqual(len(summary['tasks']), 50)
        self.assertEqual(summary['task_count'], 55)
        self.assertEqual(summary['counts']['ready'], 55)
        self.assertEqual(len(self.worker.tasks(offset=50)), 5)
        self.assertEqual(self.worker.file_owners(), [])
        with self.assertRaises(SwarmError):
            self.worker.tasks(limit=1000)

    def test_contention_and_corruption_fail_explicitly(self):
        with closing(sqlite3.connect(self.db)) as db:
            db.execute('BEGIN IMMEDIATE')
            with self.assertRaisesRegex(SwarmError, 'coordination'):
                self.task()
        self.db.write_bytes(b'corrupt')
        with self.assertRaisesRegex(SwarmError, 'coordination'):
            self.parent.summary()


if __name__ == '__main__':
    unittest.main()

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

    def test_notification_batch_carries_atomic_board_generation(self):
        self.parent.send('wake-batch', 'worker', 'action')
        batch = self.parent._notifications(True)
        self.assertEqual(batch['generation'], self.parent.summary()['event_cursor'])
        self.assertTrue(any(member['id'] == 'worker' for member in batch['members']))
        self.assertEqual(self.parent._notifications(True)['members'], [])

    def test_rejected_request_without_attempt_does_not_pause_usage_budget(self):
        self.parent.usage_budget(100, strict_unknown=True)
        self.worker._record_request({'request_id': 'rejected', 'instrumented_attempts': 0,
                                     'outcome': 'rejected'})
        self.assertEqual(self.parent.usage_report()['totals']['unknown_usage_requests'], 0)
        self.assertEqual(self.parent.summary()['status'], 'running')

    def test_request_redelivery_accepts_runtime_digest_becoming_available(self):
        record = {'request_id': 'digest', 'instrumented_attempts': 1, 'outcome': 'failed',
                  'runtime': {'process_instance_id': 'same', 'executable_digest_pending': True}}
        self.worker._record_request(record)
        record['runtime'] = {'process_instance_id': 'same', 'executable_digest_pending': False,
                             'executable_sha256': 'abc'}
        self.worker._record_request(record)
        self.assertEqual(self.parent.usage_report()['totals']['requests'], 1)
        record['runtime']['executable_sha256'] = 'different'
        with self.assertRaises(SwarmError):
            self.worker._record_request(record)
        record['runtime']['executable_sha256'] = 'abc'
        record['runtime']['process_instance_id'] = 'different'
        with self.assertRaises(SwarmError):
            self.worker._record_request(record)

    def test_usage_budget_is_idempotent_warns_once_and_pauses_without_losing_claims(self):
        task = self.task()
        claim = self.worker.claim(task['id'])
        self.parent.usage_budget(100)
        first = {'request_id': 'r1', 'context_input_tokens': 70, 'input_tokens': 50,
                 'output_tokens': 10, 'cache_read_tokens': 20, 'cache_write_tokens': 0,
                 'instrumented_attempts': 2, 'outcome': 'succeeded'}
        self.worker._record_request(first)
        self.client('worker')._record_request(first)
        report = self.parent.usage_report()
        self.assertEqual(report['totals']['observed_tokens'], 80)
        self.assertEqual(report['totals']['requests'], 1)
        self.assertEqual(report['totals']['attempts'], 2)
        warnings = [event for event in self.parent.events(limit=100)['events'] if event['action'] == 'usage-warning']
        self.assertEqual(len(warnings), 1)
        self.worker._record_request(dict(first, request_id='r2', context_input_tokens=20, output_tokens=5))
        paused = self.parent.summary()
        self.assertEqual((paused['status'], paused['outcome']), ('paused', 'budget-exhausted'))
        self.assertEqual(self.worker.task(task['id'])['token'], claim['token'])
        with self.assertRaisesRegex(SwarmError, 'token budget'):
            self.parent._resume_external()
        self.assertEqual(self.parent._request_admission()['status'], 'paused')
        self.parent.usage_budget(200)
        self.parent._resume_external()
        admission = self.parent._request_admission()
        self.assertEqual((admission['status'], admission['outcome']), ('running', None))

    def test_unknown_usage_is_explicit_and_strict_budget_suspends(self):
        self.parent.usage_budget(100, strict_unknown=True)
        self.worker._record_request({'request_id': 'unknown', 'context_input_tokens': None,
                                     'input_tokens': None, 'output_tokens': None,
                                     'instrumented_attempts': 1, 'outcome': 'failed'})
        report = self.parent.usage_report()
        self.assertEqual(report['totals']['unknown_usage_requests'], 1)
        self.assertEqual(report['totals']['observed_tokens'], 0)
        self.assertEqual(self.parent.summary()['status'], 'paused')
        with self.assertRaises(SwarmError):
            self.worker.usage_budget(200)

    def test_pause_is_durable_freezes_budget_and_rejects_mutation(self):
        from unittest.mock import patch
        now = time.time()
        deadline = self.parent.summary()['deadline']
        self.parent.coordination.clock = lambda: time.time()
        with patch('time.time', return_value=now):
            self.parent.pause('operator requested preservation')
        paused = self.client('coordinator')
        paused.coordination.clock = lambda: time.time()
        with patch('time.time', return_value=deadline + 500):
            self.assertEqual(paused.summary()['status'], 'paused')
            self.assertEqual(self.worker._notifications(), [])
            with self.assertRaises(SwarmError):
                self.task('cannot start during pause')
            with self.assertRaises(SwarmError):
                self.worker.resume()
            with self.assertRaisesRegex(SwarmError, 'outside the swarm'):
                paused.resume()
            paused._resume_external()
            resumed = paused.summary()
            self.assertEqual(resumed['status'], 'running')
            self.assertAlmostEqual(resumed['deadline'] - (deadline + 500), deadline - now, places=3)

    def test_summary_cursor_avoids_unchanged_payload_and_history_cursor_is_validated(self):
        cursor = self.parent.summary()['event_cursor']
        self.assertEqual(self.parent.summary(since=cursor), {'unchanged': True, 'event_cursor': cursor, 'status': 'running'})
        self.task('new change')
        self.assertFalse(self.parent.summary(since=cursor).get('unchanged', False))
        for invalid in [-1, True, '1']:
            with self.assertRaises(SwarmError):
                self.parent.events(after=invalid)

    def test_wake_generation_is_consumed_once_and_rechecks_current_work(self):
        message = self.worker.send('wake', 'coordinator', 'review this')
        generation = self.parent.summary()['event_cursor']
        self.assertTrue(self.parent._accept_wake(generation))
        self.assertFalse(self.client('coordinator')._accept_wake(generation))
        self.parent.ack(message['id'])
        self.assertFalse(self.parent._accept_wake(self.parent.summary()['event_cursor']))
        self.worker.send('wake-2', 'coordinator', 'new work')
        generation = self.parent.summary()['event_cursor']
        self.parent.pause('pause takes priority over queued hint')
        self.assertFalse(self.parent._accept_wake(generation))

    def test_cancelled_attempt_does_not_keep_a_strict_budget_paused(self):
        self.parent.usage_budget(100, strict_unknown=True)
        self.worker._record_request({'request_id': 'cancelled', 'instrumented_attempts': 1,
                                     'outcome': 'cancelled'})
        self.assertEqual(self.parent.usage_report()['totals']['unknown_usage_requests'], 0)
        self.parent.pause('supervisor pause')
        self.parent._resume_external()
        self.assertEqual(self.parent.summary()['status'], 'running')
        self.worker._record_request({'request_id': 'hidden', 'instrumented_attempts': 1,
                                     'outcome': 'failed'})
        self.assertEqual(self.parent.usage_report()['totals']['unknown_usage_requests'], 1)
        self.assertEqual(self.parent.summary()['status'], 'paused')

    def test_paused_run_retains_the_wake_frontier_until_resume(self):
        self.worker.send('wake-paused', 'coordinator', 'work')
        generation = self.parent.summary()['event_cursor']
        self.parent.pause('hold')
        self.assertFalse(self.parent._accept_wake(generation))
        self.parent._resume_external()
        self.assertTrue(self.parent._accept_wake(generation), 'the generation was not consumed while paused')
        with self.assertRaises(SwarmError):
            self.parent._accept_wake(self.parent.summary()['event_cursor'] + 1)

    def test_request_diagnostic_ledger_bounds_are_explicit(self):
        self.worker._record_request({'request_id': 'first', 'instrumented_attempts': 1, 'outcome': 'failed'})
        with self.assertRaises(SwarmError):
            self.worker._record_request({'request_id': 'huge', 'instrumented_attempts': 1, 'outcome': 'failed',
                                         'error_class': 'x' * 32768})
        with closing(sqlite3.connect(self.db)) as db:
            db.executemany('INSERT INTO request_usage VALUES(?,?,?,?,?,?,?,?,?,?)',
                           [(f'fill-{index}', 'worker', '{}', 0, 0, 1, None, None, None, None) for index in range(9999)])
            db.commit()
        with self.assertRaises(SwarmError) as refused:
            self.worker._record_request({'request_id': 'overflow', 'instrumented_attempts': 1, 'outcome': 'failed'})
        self.assertIn('ledger full', str(refused.exception))
        self.assertEqual(self.parent.summary()['status'], 'running', 'a full ledger never blocks the run itself')

    def test_default_summary_does_not_replay_historical_contract_payloads(self):
        original = self.parent.summary()
        for index in range(12):
            self.parent.amend(original['goal'], ['x' * 6000], original['criteria'], f'change {index}')
        summary = self.parent.summary()
        self.assertLess(len(json.dumps(summary)), 20000)
        self.assertNotIn('events', summary)
        self.assertGreater(summary['event_cursor'], 0)
        page = self.parent.events(after=0, limit=2)
        self.assertEqual(len(page['events']), 2)
        self.assertTrue(page['has_more'])
        next_page = self.parent.events(after=page['cursor'], limit=2)
        self.assertGreater(next_page['events'][0]['id'], page['cursor'])
        self.assertEqual(self.parent.events(after=summary['event_cursor'])['events'], [])

    def test_resolved_blocker_resumes_original_claim_without_releasing_files(self):
        claim = self.worker.claim(self.task()['id'])
        self.worker.block(claim['id'], claim['token'], 'awaiting approval')
        self.worker.reserve(claim['id'], claim['token'], ['owned.rs'])
        self.worker.unblock(claim['id'], claim['token'], 'approval received')
        resumed = self.worker.task(claim['id'])
        self.assertEqual(resumed['status'], 'claimed')
        self.assertEqual(resumed['token'], claim['token'])
        self.assertIsNone(resumed['blocker'])
        self.assertEqual(self.worker.file_owners()[0]['path'], 'owned.rs')
        with self.assertRaises(SwarmError):
            self.parent.unblock(claim['id'], claim['token'], 'not the owner')
        self.worker.submit(claim['id'], claim['token'], [{'artifact': 'tests', 'revision': 'R1'}])
        with self.assertRaises(SwarmError):
            self.worker.unblock(claim['id'], claim['token'], 'cannot reopen submitted evidence')

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
        events = self.parent.events(limit=100)['events']
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
        self.assertEqual((self.parent.summary()['status'], self.parent.summary()['outcome']), ('paused', 'succeeded'))

    def test_unchanged_blocker_does_not_repeat_idle_wake_cycles(self):
        task = self.task()
        claim = self.worker.claim(task['id'])
        self.worker.block(task['id'], claim['token'], 'awaiting approval')
        self.assertEqual([m['id'] for m in self.worker._notifications()], ['coordinator'])
        for _ in range(5):
            self.parent.summary()
            self.assertEqual(self.parent.inbox(), [])
            self.worker.block(task['id'], claim['token'], 'awaiting approval')
            self.assertEqual(self.worker._notifications(), [])
        self.worker.block(task['id'], claim['token'], 'new blocker')
        self.assertEqual([m['id'] for m in self.worker._notifications()], ['coordinator'])

    def test_identical_evidence_does_not_repeat_review_wake(self):
        for expected in (['coordinator'], []):
            self.worker.evidence('tests', 'tests.log', 'R1', 'command', True)
            self.assertEqual([m['id'] for m in self.worker._notifications()], expected)

    def test_dependency_work_only_wakes_when_claimable(self):
        first = self.task('dependency')
        claim = self.worker.claim(first['id'])
        self.worker._notifications()
        self.task('dependent', [first['id']])
        self.assertEqual(self.worker._notifications(), [])
        self.worker.submit(first['id'], claim['token'], [{'artifact':'tests.log','revision':'R1'}])
        self.parent.verify_task(first['id'], claim['token'], 'R1')
        self.assertEqual(self.worker._notifications(), [], 'already verified submission is stale')
        self.assertEqual([m['id'] for m in self.parent._notifications()], ['worker'])

    def test_consumed_work_does_not_emit_stale_wake_hints(self):
        self.parent._notifications()
        self.worker._notifications()
        message = self.worker.send('already-read', 'coordinator', 'Please review')
        self.parent.ack(message['id'])
        self.assertEqual(self.worker._notifications(), [])

    def test_claimed_work_does_not_wake_idle_peers(self):
        self.parent._notifications()
        self.worker._notifications()
        task = self.task()
        self.worker.claim(task['id'])
        self.assertEqual(self.worker._notifications(), [])

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
        self.assertEqual(self.worker._notifications(), [], 'an ended run must suppress queued work hints')

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
        self.assertEqual(self.worker.events(limit=100)['events'][-1]['actor'], 'coordinator')

    def test_member_limit_accepts_upper_boundary(self):
        with tempfile.TemporaryDirectory() as directory:
            client = Workbench(str(pathlib.Path(directory) / 'coordination.sqlite'), directory, 'coordinator')
            client.create('x', [], [{'id':'t','kind':'command','description':'pass'}], 25, time.time() + 1)
            self.assertEqual(client.summary()['member_limit'], 25)

    def test_invalid_limits_and_deadlines_are_rejected(self):
        for limit in (0, 26, True):
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
        # Superseding one of the hundred makes room for its replacement.
        full = self.parent.inbox()[0]['id']
        self.worker.send('replace', 'coordinator', 'newer', supersedes=full)

    def test_messages_carry_a_revision_and_can_be_superseded_or_withdrawn(self):
        self.worker._notifications()
        first = self.worker.send('r1', 'coordinator', 'review head one', revision='abc1')
        second = self.worker.send('r2', 'coordinator', 'review head two', revision='abc2', supersedes=first['id'])
        inbox = self.parent.inbox()
        self.assertEqual([(m['id'], m['revision'], m['supersedes']) for m in inbox], [(second['id'], 'abc2', first['id'])])
        audit = {m['id']: m for m in self.parent.inbox(include_consumed=True)}
        self.assertEqual((audit[first['id']]['status'], audit[first['id']]['superseded_by']), ('superseded', second['id']))
        self.assertEqual([m['id'] for m in self.worker._notifications()], ['coordinator'], 'one hint for the live message')
        events = self.parent.events(limit=100)['events']
        self.assertEqual([e['action'] for e in events[-3:]], ['message_accepted', 'message_superseded', 'message_accepted'])
        self.assertEqual(json.loads(events[-1]['detail'])['revision'], 'abc2')
        # Idempotent replay returns the original receipt; a changed field is a mismatch.
        self.assertEqual(self.worker.send('r2', 'coordinator', 'review head two', revision='abc2', supersedes=first['id']), second)
        with self.assertRaisesRegex(SwarmError, 'different payload'):
            self.worker.send('r2', 'coordinator', 'review head two', revision='abc3', supersedes=first['id'])
        # Only your own unread message to the same recipient can be superseded.
        for supersedes, sender, reason in [(first['id'], self.worker, 'already superseded'),
                                           (999, self.worker, 'only your own'),
                                           (second['id'], self.parent, 'only your own')]:
            with self.subTest(supersedes=supersedes), self.assertRaisesRegex(SwarmError, reason):
                sender.send(f'bad-{supersedes}', 'coordinator', 'x', supersedes=supersedes)
        # A message to another recipient cannot be superseded by this one.
        to_self = self.worker.send('self', 'worker', 'note to self')
        with self.assertRaisesRegex(SwarmError, 'same recipient'):
            self.worker.send('cross', 'coordinator', 'x', supersedes=to_self['id'])
        # The id must be an int: SQLite would happily coerce a numeric string.
        with self.assertRaisesRegex(SwarmError, 'message id'):
            self.worker.send('bad-type', 'coordinator', 'x', supersedes=str(second['id']))
        self.assertEqual([m['id'] for m in self.parent.inbox()], [second['id']], 'the refused send changed nothing')
        with self.assertRaisesRegex(SwarmError, 'message revision'):
            self.worker.send('bad-rev', 'coordinator', 'x', revision='')
        # Acknowledging a superseded message is a no-op; the live one is consumed.
        self.parent.ack(first['id'])
        after = {m['id']: m['status'] for m in self.parent.inbox(include_consumed=True)}
        self.assertEqual(after[first['id']], 'superseded', 'ack must not consume a retired message')
        self.assertNotEqual(self.parent.events(limit=100)['events'][-1]['action'], 'message_consumed')
        self.parent.ack(second['id'])
        self.assertEqual(self.parent.inbox(), [])

    def test_plain_sends_replay_request_keys_recorded_before_message_revisions(self):
        message = self.worker.send('legacy', 'coordinator', 'hello')
        with closing(sqlite3.connect(self.db)) as db:
            db.execute("UPDATE requests SET payload=? WHERE actor='worker' AND request='legacy'",
                       (json.dumps(['send', 'coordinator', 'hello'], sort_keys=True, separators=(',', ':')),))
            db.commit()
        self.assertEqual(self.worker.send('legacy', 'coordinator', 'hello'), message, 'an old-shape ledger row still replays')
        with self.assertRaisesRegex(SwarmError, 'different payload'):
            self.worker.send('legacy', 'coordinator', 'hello', revision='abc1')

    def test_message_columns_are_migrated_into_an_older_store(self):
        path = self.root / 'older.sqlite'
        with closing(sqlite3.connect(path)) as db:
            db.executescript('''
                CREATE TABLE run (id TEXT PRIMARY KEY, goal TEXT, constraints TEXT, criteria TEXT,
                 coordinator TEXT, integrator TEXT, member_limit INTEGER, deadline REAL, status TEXT);
                CREATE TABLE members (id TEXT PRIMARY KEY, reservation TEXT UNIQUE, status TEXT, pid INTEGER, started TEXT, socket TEXT);
                CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, acceptance TEXT, dependencies TEXT, status TEXT, owner TEXT, token TEXT, evidence TEXT, blocker TEXT);
                CREATE TABLE files (path TEXT PRIMARY KEY, task INTEGER, owner TEXT, claim TEXT, token TEXT);
                CREATE TABLE messages (id INTEGER PRIMARY KEY, sender TEXT, recipient TEXT, body TEXT, status TEXT);
                CREATE TABLE evidence (criterion TEXT, artifact TEXT, revision TEXT, kind TEXT, actor TEXT, accepted INTEGER, PRIMARY KEY(criterion, actor));
                CREATE TABLE requests (actor TEXT, request TEXT, payload TEXT, result TEXT, PRIMARY KEY(actor, request));
                CREATE TABLE events (id INTEGER PRIMARY KEY, actor TEXT, time REAL, action TEXT, detail TEXT);
            ''')
        older = Workbench(str(path), str(self.root), 'coordinator')
        older.create('old store', [], [{'id': 'tests', 'kind': 'command', 'description': 'pass'}], 2, time.time() + 300)
        sent = older.send('m', 'coordinator', 'to self', revision='abc1')
        self.assertEqual(older.inbox()[0]['revision'], 'abc1')
        older.withdraw(sent['id'])
        self.assertEqual(older.inbox(include_consumed=True)[0]['status'], 'withdrawn')

    def test_withdraw_and_ack_take_only_message_ids(self):
        message = self.worker.send('w', 'coordinator', 'hello')
        for bad in ['1', True, 0, None]:
            with self.subTest(bad=bad), self.assertRaisesRegex(SwarmError, 'message id'):
                self.worker.withdraw(bad)
            with self.subTest(bad=bad), self.assertRaisesRegex(SwarmError, 'message id'):
                self.parent.ack(bad)
        self.assertEqual([m['id'] for m in self.parent.inbox()], [message['id']], 'nothing was withdrawn or acknowledged')
        with self.assertRaisesRegex(SwarmError, 'only your own message'):
            self.parent.withdraw(message['id'])

    def test_a_withdrawn_message_leaves_the_inbox_and_wakes_nobody(self):
        self.worker._notifications()
        message = self.worker.send('w1', 'coordinator', 'never mind', revision='abc1')
        self.worker.withdraw(message['id'])
        self.assertEqual(self.parent.inbox(), [])
        self.assertEqual(self.parent.inbox(include_consumed=True)[-1]['status'], 'withdrawn')
        self.assertEqual(self.worker._notifications(), [], 'a withdrawn message must not wake its recipient')
        self.assertEqual(self.parent.events(limit=100)['events'][-1]['action'], 'message_withdrawn')
        self.worker.withdraw(message['id'])  # repeating a withdrawal is a no-op, like ack
        events = [e['action'] for e in self.parent.events(limit=100)['events']]
        self.assertEqual(events.count('message_withdrawn'), 1)
        with self.assertRaisesRegex(SwarmError, 'own message'):
            self.parent.withdraw(message['id'])
        self.parent.ack(message['id'])
        self.assertEqual(self.parent.inbox(include_consumed=True)[-1]['status'], 'withdrawn', 'ack cannot revive a withdrawn message')

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
        self.assertEqual((self.parent.summary()['status'], self.parent.summary()['outcome']), ('paused', 'succeeded'))

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
        self.assertEqual((summary['status'], summary['outcome'], summary['outcome_reason']),
                         ('paused', 'budget-exhausted', 'deadline'))
        self.assertEqual(len(summary['tasks']), 1)
        self.assertEqual(self.worker.summary()['members'][1]['status'], 'live', 'expiry kills nobody')
        with self.assertRaisesRegex(SwarmError, 'extend the deadline'):
            self.parent._resume_external()
        with self.assertRaises(SwarmError):
            self.parent._extend_deadline(0)
        with closing(sqlite3.connect(self.db)) as db:
            db.execute('UPDATE run SET deadline=1')  # long expired: the grant is future time
            db.commit()
        self.parent._extend_deadline(600)
        self.parent._resume_external()
        resumed = self.parent.summary()
        self.assertEqual((resumed['status'], resumed['outcome']), ('running', None))
        self.assertGreater(resumed['deadline'], time.time() + 500)
        self.assertLess(resumed['deadline'], time.time() + 700)
        self.worker.claim(task['id'])

    def test_coordinator_death_leaves_readable_failed_progress(self):
        self.task()
        self.worker._confirmed_dead('coordinator')
        summary = self.parent.summary()
        self.assertEqual((summary['status'], summary['outcome']), ('paused', 'failed'))
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

    def test_coordinator_stop_is_a_resumable_pause_only_the_supervisor_lifts(self):
        task = self.task()
        claim = self.worker.claim(task['id'])
        self.parent.stop('blocked', 'needs a decision from the master')
        ended = self.parent.summary()
        self.assertEqual((ended['status'], ended['outcome'], ended['outcome_reason']),
                         ('paused', 'blocked', 'needs a decision from the master'))
        self.assertEqual([m['status'] for m in ended['members']], ['live', 'live'], 'nobody is terminated')
        self.assertEqual(self.worker.task(task['id'])['token'], claim['token'], 'claims survive the end')
        self.assertEqual(self.parent._control_status()['outcome'], 'blocked')
        with self.assertRaisesRegex(SwarmError, 'no new work'):
            self.task('nothing new while ended')
        with self.assertRaisesRegex(SwarmError, 'outside the swarm'):
            self.parent.resume()
        with self.assertRaisesRegex(SwarmError, 'outside the swarm'):
            self.worker.resume()
        self.parent.stop('blocked', 'needs a decision from the master')  # idempotent
        with self.assertRaisesRegex(SwarmError, 'already paused'):
            self.parent.stop('failed', 'a different verdict while ended')
        self.parent._resume_external()
        resumed = self.parent.summary()
        self.assertEqual((resumed['status'], resumed['outcome']), ('running', None))
        self.assertEqual(self.worker.task(task['id'])['token'], claim['token'])
        actions = [e['action'] for e in self.parent.events(limit=100)['events']]
        self.assertEqual(actions[-3:], ['stop', 'paused', 'resumed'])

    def test_completion_holds_success_until_the_supervisor_closes_it(self):
        self.parent.evidence('tests', 'ci.log', 'R1', 'command', True)
        self.parent.evidence('review', 'review.md', 'R1', 'review', True)
        with self.assertRaisesRegex(SwarmError, 'without a proposed outcome'):
            self.parent._close()
        self.parent.pause('a plain pause holds nothing to close')
        with self.assertRaisesRegex(SwarmError, 'without a proposed outcome'):
            self.parent._close()
        self.parent._resume_external()
        self.parent.complete('R1')
        held = self.parent.summary()
        self.assertEqual((held['status'], held['outcome']), ('paused', 'succeeded'))
        self.assertEqual([m['status'] for m in held['members']], ['live', 'live'])
        with self.assertRaisesRegex(SwarmError, 'outside the swarm'):
            self.parent.coordination.close()
        self.parent._close()
        closed = self.parent.summary()
        self.assertEqual((closed['status'], closed['outcome']), ('succeeded', 'succeeded'))
        with self.assertRaisesRegex(SwarmError, 'succeeded'):
            self.task('after close')
        with self.assertRaisesRegex(SwarmError, 'only a paused run'):
            self.parent._resume_external()
        self.assertEqual(self.parent.events(limit=100)['events'][-1]['action'], 'closed')

    def test_cancellation_stays_terminal_even_while_ended(self):
        self.parent.stop('failed', 'unrecoverable')
        with self.assertRaisesRegex(SwarmError, r'paused \(failed'):
            self.parent.stop('blocked', 'a second verdict while ended')
        self.parent.stop('cancelled', 'user gave up')
        cancelled = self.parent.summary()
        self.assertEqual((cancelled['status'], cancelled['outcome']), ('cancelled', None))
        with self.assertRaisesRegex(SwarmError, 'only a paused run'):
            self.parent._resume_external()
        with self.assertRaises(SwarmError):
            self.parent._extend_deadline(60)

    def test_a_closed_run_cannot_be_cancelled_over(self):
        self.parent.evidence('tests', 'ci.log', 'R1', 'command', True)
        self.parent.evidence('review', 'review.md', 'R1', 'review', True)
        self.parent.complete('R1')
        self.parent._close()
        with self.assertRaisesRegex(SwarmError, 'already succeeded'):
            self.parent.stop('cancelled', 'too late')
        self.assertEqual(self.parent.summary()['status'], 'succeeded')
        placeholder = self.client('other')
        with self.assertRaisesRegex(SwarmError, 'unknown'):
            placeholder.stop('cancelled', 'not a member')
        fresh = Workbench(str(self.root / 'fresh.sqlite'), str(self.root), 'boot')
        fresh._bootstrap(1, 'start-b', '/tmp/b.sock')
        with self.assertRaisesRegex(SwarmError, 'not created yet'):
            fresh.stop('cancelled', 'nothing to cancel')
        self.assertEqual(fresh._status()['status'], 'setup')

    def test_a_lost_coordinator_ends_the_run_as_a_failed_pause_even_while_paused(self):
        self.parent.pause('hold')
        self.worker._confirmed_dead('coordinator')
        held = self.worker.summary()
        self.assertEqual((held['status'], held['outcome']), ('paused', 'failed'))
        self.assertIn('coordinator death', held['outcome_reason'])
        with self.assertRaises(SwarmError):
            self.task('after-parent-death')
        actions = [e['action'] for e in self.worker.events(limit=100)['events']]
        self.assertEqual(actions.count('paused'), 1, 'a loss during a pause does not restart the pause clock')

    def test_a_loss_during_a_pause_keeps_the_frozen_budget(self):
        from unittest.mock import patch
        now = time.time()
        deadline = self.parent.summary()['deadline']
        self.parent.coordination.clock = lambda: time.time()
        with patch('time.time', return_value=now):
            self.parent.pause('hold')
        with patch('time.time', return_value=now + 100):
            self.parent._quarantine('worker')
        held = self.parent.summary()
        self.assertEqual((held['status'], held['outcome']), ('paused', 'failed'))
        with patch('time.time', return_value=now + 500):
            self.parent._resume_external()
            self.assertAlmostEqual(self.parent.summary()['deadline'], deadline + 500, places=3)

    def test_a_worker_loss_keeps_the_verdict_the_coordinator_already_proposed(self):
        self.parent.evidence('tests', 'ci.log', 'R1', 'command', True)
        self.parent.evidence('review', 'review.md', 'R1', 'review', True)
        self.parent.complete('R1')
        self.parent._quarantine('worker')
        held = self.parent.summary()
        self.assertEqual((held['status'], held['outcome']), ('paused', 'succeeded'))
        self.assertEqual(self.parent.events(limit=100)['events'][-1]['action'], 'scope_unknown')
        self.parent._close()
        self.assertEqual(self.parent.summary()['status'], 'succeeded')

    def test_deadline_extension_is_capped_at_seven_days_ahead(self):
        with self.assertRaisesRegex(SwarmError, 'seven days'):
            self.parent._extend_deadline(604800)
        self.parent._extend_deadline(3600)
        self.assertGreater(self.parent.summary()['deadline'], time.time() + 3000)


if __name__ == '__main__':
    unittest.main()

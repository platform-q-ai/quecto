"""Deterministic contract tests; run: python3 -m unittest discover -s <this directory>."""
import concurrent.futures
import json
import os
import sqlite3
import tempfile
import unittest

from swarm import Swarm, SwarmError, demo, worker


class SwarmTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.tmp.name, 'state.sqlite')
        self.now = 100
        self.s = Swarm.create(self.path, goal='sum squares', done='verified sum',
                              deadline=200, coordinator='lead', members=['lead', 'a', 'b'],
                              token_budget=30, clock=lambda: self.now)

    def tearDown(self):
        self.tmp.cleanup()

    def task(self, name='one', **kwargs):
        return self.s.add_task('lead', name, 'calculate', **kwargs)

    def test_validation_and_fixed_membership(self):
        with self.assertRaises(SwarmError):
            Swarm.create(os.path.join(self.tmp.name, 'bad'), goal='x', done='y',
                         deadline=200, coordinator='lead', members=['lead'] * 11,
                         token_budget=1, clock=lambda: 100)
        with self.assertRaises(SwarmError):
            self.s.message('stranger', 'lead', 'hello')
        with self.assertRaises(SwarmError):
            self.s.add_task('a', 'x', 'not coordinator')
        with self.assertRaises(SwarmError):
            Swarm.create(self.path, goal='x', done='y', deadline=200,
                         coordinator='lead', members=['lead'], token_budget=1)
        self.assertEqual(self.s.summary()['members'], ['a', 'b', 'lead'])

    def test_dependencies_tokens_evidence_and_completion(self):
        self.task(token_limit=10)
        self.task('two', dependencies=['one'], token_limit=20)
        self.assertIsNone(self.s.claim('b', 'two'))
        claim = self.s.claim('a', 'one')
        self.assertEqual(claim['task_id'], 'one')
        with self.assertRaises(SwarmError):
            self.s.finish('a', 'one', claim['claim_token'], evidence=[], tokens_used=4)
        self.s.finish('a', 'one', claim['claim_token'], evidence=[{'result': 4}], tokens_used=4)
        second = self.s.claim('b', 'two')
        self.s.finish('b', 'two', second['claim_token'], evidence=[{'result': 9}], tokens_used=5)
        with self.assertRaises(SwarmError):
            self.s.complete('a', 'done')
        self.s.complete('lead', 'checked both results')
        summary = self.s.summary()
        self.assertEqual(summary['status'], 'completed')
        self.assertEqual(summary['tokens_used'], 9)
        self.assertEqual(summary['tokens_reserved'], 0)
        self.assertEqual(len(summary['evidence']), 2)
        self.assertEqual(summary['completion_evidence'], 'checked both results')

    def test_claim_is_atomic_between_connections(self):
        self.task(token_limit=5)
        def attempt(actor):
            return Swarm(self.path, clock=lambda: 100).claim(actor, 'one')
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            claims = list(pool.map(attempt, ['a', 'b']))
        self.assertEqual(sum(c is not None for c in claims), 1)
        self.assertEqual(self.s.summary()['tokens_reserved'], 5)

    def test_budget_and_claim_token_are_atomic(self):
        self.task(token_limit=20)
        self.task('two', token_limit=20)
        claim = self.s.claim('a', 'one')
        self.assertIsNone(self.s.claim('b', 'two'))
        with self.assertRaises(SwarmError):
            self.s.finish('b', 'one', claim['claim_token'], evidence=[1], tokens_used=2)
        with self.assertRaises(SwarmError):
            self.s.finish('a', 'one', 'wrong', evidence=[1], tokens_used=2)
        with self.assertRaises(SwarmError):
            self.s.finish('a', 'one', claim['claim_token'], evidence=[1], tokens_used=21)
        self.assertEqual(self.s.summary()['tokens_reserved'], 20)
        self.s.finish('a', 'one', claim['claim_token'], evidence=[1], tokens_used=3)
        self.assertIsNotNone(self.s.claim('b', 'two'))

    def test_dependency_validation_and_completion_gate(self):
        with self.assertRaises(SwarmError):
            self.task(dependencies=['missing'])
        self.assertEqual(self.s.summary()['tasks'], [])
        self.task()
        with self.assertRaises(SwarmError):
            self.task('one')
        with self.assertRaises(SwarmError):
            self.s.complete('lead', 'premature')

    def test_messages_have_stable_ids_and_visibility(self):
        first = self.s.message('a', 'b', 'ready')
        second = self.s.message('lead', None, 'broadcast')
        self.assertLess(first, second)
        self.assertEqual([m['body'] for m in self.s.messages('b')], ['ready', 'broadcast'])
        self.assertEqual([m['body'] for m in self.s.messages('lead')], ['broadcast'])
        self.assertEqual(len(self.s.messages('b', after=first)), 1)

    def test_reservations_are_normalized_and_cooperative(self):
        self.assertTrue(self.s.reserve('a', 'src/./math.py'))
        self.assertTrue(self.s.reserve('a', 'src/math.py'))
        self.assertFalse(self.s.reserve('b', 'src/math.py'))
        with self.assertRaises(SwarmError):
            self.s.release('b', 'src/math.py')
        for path in ['../secret', '/etc/passwd', 'src/../secret', 'C:\\secret', '']:
            with self.assertRaises(SwarmError):
                self.s.reserve('a', path)
        self.s.release('a', 'src/math.py')
        self.assertTrue(self.s.reserve('b', 'src/math.py'))

    def test_cancel_is_terminal_and_releases_reservations(self):
        self.task(token_limit=7)
        claim = self.s.claim('a', 'one')
        self.s.reserve('a', 'x')
        with self.assertRaises(SwarmError):
            self.s.cancel('a', 'stop')
        self.s.cancel('lead', 'stop')
        summary = self.s.summary()
        self.assertEqual(summary['status'], 'cancelled')
        self.assertEqual(summary['tokens_reserved'], 0)
        self.assertEqual(summary['reservations'], [])
        self.assertEqual(summary['tasks'][0]['status'], 'cancelled')
        with self.assertRaises(SwarmError):
            self.s.finish('a', 'one', claim['claim_token'], evidence=[1], tokens_used=1)
        with self.assertRaises(SwarmError):
            self.s.message('a', 'lead', 'late mutation')

    def test_deadline_prevents_mutations_but_allows_cancel_and_read(self):
        self.task()
        self.now = 200
        with self.assertRaises(SwarmError):
            self.s.claim('a')
        self.assertTrue(self.s.summary()['deadline_exceeded'])
        self.s.cancel('lead', 'deadline reached')
        self.assertEqual(self.s.summary()['status'], 'cancelled')

    def test_bound_worker_accepts_board_or_path(self):
        self.task(token_limit=5)
        w = worker(self.s, 'a')
        claim = w.claim('one')
        w.message('lead', 'working')
        self.assertTrue(w.reserve('result.txt'))
        w.release('result.txt')
        w.finish('one', claim['claim_token'], evidence=[{'result': 4}], tokens_used=2)
        self.assertEqual(w.summary()['tasks'][0]['status'], 'done')
        self.s.message('lead', 'a', 'thanks')
        self.assertEqual(w.messages()[0]['body'], 'thanks')
        self.assertEqual(worker(self.path, 'b').summary()['members'], ['a', 'b', 'lead'])
        with self.assertRaises(SwarmError):
            worker(self.s, 'outsider')

    def test_demo_uses_concurrent_fake_workers_and_verified_evidence(self):
        path = os.path.join(self.tmp.name, 'demo.sqlite')
        result = demo(path)
        self.assertEqual(result['status'], 'completed')
        self.assertEqual(result['verified_result'], 55)
        self.assertTrue(result['fake_agents'])
        self.assertEqual(len(result['members']), 3)
        self.assertEqual(len(result['messages']), 4)
        self.assertEqual({t['owner'] for t in result['tasks']}, {'fake-left', 'fake-right', 'coordinator'})
        self.assertEqual(json.loads(result['evidence'][-1]['payload']), [{'sum_of_squares': 55}])
        with self.assertRaises(SwarmError):
            demo(path)  # Never overwrite an existing database.


if __name__ == '__main__':
    unittest.main()

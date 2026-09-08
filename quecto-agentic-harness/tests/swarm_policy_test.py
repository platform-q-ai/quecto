"""Pure policy/use-case tests: no SQLite, interpreter child, sockets or processes."""
from contextlib import contextmanager
from copy import deepcopy
import pathlib
import sys
import unittest

SRC = pathlib.Path(__file__).resolve().parents[1] / 'src'
sys.path[:0] = [str(SRC / 'domain'), str(SRC / 'application')]
from swarm_policy import SwarmError, authorize, completion, admission
from swarm_use_cases import Coordination


class MemoryRepository:
    def __init__(self):
        self.state = {'run': {'status': 'running', 'coordinator': 'parent', 'deadline': 100, 'member_limit': 2},
                      'members': {'parent': {'id': 'parent', 'status': 'live', 'reservation': 'p'}},
                      'criteria': [{'id': 'test', 'kind': 'command'}],
                      'evidence': [{'criterion': 'test', 'kind': 'command', 'revision': 'R2', 'accepted': True}],
                      'tasks': [{'id': 1, 'status': 'completed', 'evidence': [{'artifact': 'log', 'revision': 'R1'}]}],
                      'events': [], 'has_reservations': False}
        self.inside = False

    @contextmanager
    def atomic(self):
        assert not self.inside
        old = deepcopy(self.state)
        self.inside = True
        try:
            yield self
        except BaseException:
            self.state = old
            raise
        finally:
            self.inside = False

    def run(self): return self.state['run']
    def member(self, identity): return self.state['members'].get(identity)
    def usage(self): return sum(m['status'] != 'dead' for m in self.state['members'].values())
    def reserve_member(self, identity, reservation):
        assert self.inside
        self.state['members'][identity] = {'id': identity, 'status': 'reserved', 'reservation': reservation}
    def completion_state(self): return self.state
    def task(self, identity): return deepcopy(next(t for t in self.state['tasks'] if t['id'] == identity))
    def replace_task_evidence(self, identity, evidence):
        assert self.inside
        next(t for t in self.state['tasks'] if t['id'] == identity)['evidence'] = evidence
    def set_outcome(self, status):
        assert self.inside
        self.state['run']['status'] = status
    def event(self, action, detail):
        assert self.inside
        self.state['events'].append((action, detail))


class PolicyContract(unittest.TestCase):
    def test_completion_revalidation_and_transition_without_storage(self):
        repo = MemoryRepository()
        service = Coordination(repo, 'parent', lambda: 50)
        with self.assertRaisesRegex(SwarmError, 'stale revision'):
            service.complete('R2')
        service.revalidate_task(1, 'R2', [{'artifact': 'rerun', 'revision': 'R2'}])
        service.complete('R2')
        self.assertEqual(repo.run()['status'], 'succeeded')
        self.assertEqual(repo.state['events'][-2][1]['previous_evidence'][0]['revision'], 'R1')

    def test_completion_rejects_each_unsatisfied_requirement(self):
        baseline = MemoryRepository().state
        baseline['tasks'][0]['evidence'][0]['revision'] = 'R2'
        variants = [('criteria', []), ('evidence', []), ('has_reservations', True),
                    ('tasks', [{'status': 'submitted', 'evidence': []}]),
                    ('tasks', [{'status': 'completed', 'evidence': []}])]
        for key, value in variants:
            state = deepcopy(baseline)
            state[key] = value
            with self.subTest(key=key), self.assertRaises(SwarmError):
                completion(state['criteria'], state['evidence'], state['tasks'], state['has_reservations'], 'R2')
        self.assertEqual(completion(baseline['criteria'], baseline['evidence'], baseline['tasks'], False, 'R2'), 'succeeded')

    def test_authorization_distinguishes_reporting_from_mutation(self):
        run = MemoryRepository().run()
        for actor, member, options in [('worker', {'status':'live'}, {'coordinator':True}),
                                       ('parent', None, {}), ('parent', {'status':'dead'}, {})]:
            with self.subTest(actor=actor, member=member), self.assertRaises(SwarmError):
                authorize(run, actor, member, **options)
        authorize(run, 'parent', {'status':'dead'}, read_only=True)

    def test_deadline_transition_commits_before_rejected_mutation(self):
        repo = MemoryRepository()
        service = Coordination(repo, 'parent', lambda: 100)
        with self.assertRaisesRegex(SwarmError, 'budget-exhausted'):
            service.complete('R2')
        self.assertEqual(repo.run()['status'], 'budget-exhausted')
        self.assertEqual(repo.state['events'], [('stop', {'status':'budget-exhausted','reason':'deadline'})])

    def test_admission_retries_and_capacity_use_same_atomic_port(self):
        repo = MemoryRepository()
        service = Coordination(repo, 'parent', lambda: 50)
        first = service.reserve_member('worker', 'token')
        self.assertEqual(service.reserve_member('worker', 'token'), first)
        with self.assertRaisesRegex(SwarmError, 'reuse'):
            service.reserve_member('third', 'token3')
        self.assertEqual(repo.usage(), 2)
        self.assertEqual(len(repo.state['events']), 1)
        with self.assertRaisesRegex(SwarmError, 'no new admission'):
            admission(repo.run(), first, 'token', 2, 100)


if __name__ == '__main__':
    unittest.main()

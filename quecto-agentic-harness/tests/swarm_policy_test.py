"""Pure policy/use-case tests: no SQLite, interpreter child, sockets or processes."""
from contextlib import contextmanager
from copy import deepcopy
import pathlib
import sys
import unittest

SRC = pathlib.Path(__file__).resolve().parents[1] / 'src'
sys.path[:0] = [str(SRC / 'domain'), str(SRC / 'application')]
from swarm_policy import (SwarmError, ADDRESSABLE_OWNER_STATES, OWNER_IDLE_AFTER, OWNER_STATES, admission, authorize,
                          completion, idle_transition, notification_targets, owner_recovery, owner_state)
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
    def propose_outcome(self, outcome, reason):
        assert self.inside
        self.state['run'].update(status='paused', outcome=outcome, outcome_reason=reason)
    def clear_outcome(self):
        assert self.inside
        self.state['run'].update(outcome=None, outcome_reason=None)
    def event(self, action, detail):
        assert self.inside
        self.state['events'].append((action, detail))


class PolicyContract(unittest.TestCase):
    def test_notification_policy_ignores_bookkeeping_and_terminal_work(self):
        run = MemoryRepository().run()
        members = [{'id':'parent','status':'live'}, {'id':'worker','status':'live'}]
        state = {'tasks':[{'id':1,'status':'submitted','dependencies':[]}], 'messages':[{'id':1}]}
        events = [{'action':'message_consumed','detail':{}}, {'action':'claimed','detail':{}},
                  {'action':'files_reserved','detail':{}}]
        self.assertEqual(notification_targets(run, 'worker', members, events, state), [])
        events += [{'action':'submitted','detail':{'task':1}}, {'action':'message_accepted','detail':{'message':1,'recipient':'parent'}}]
        self.assertEqual([m['id'] for m in notification_targets(run, 'worker', members, events, state)], ['parent'])
        # A message that is no longer unread (superseded or withdrawn, #1837) wakes nobody.
        retired = {'tasks': [], 'messages': []}
        accepted_only = [{'action':'message_accepted','detail':{'message':1,'recipient':'parent'}}]
        self.assertEqual(notification_targets(run, 'worker', members, accepted_only, retired), [])
        run['status'] = 'succeeded'
        self.assertEqual(notification_targets(run, 'worker', members, events, state), [])

    def test_ready_work_wakes_only_members_free_to_take_it(self):
        # #2127: a member that submitted its task (parked), or is busy with a
        # claimed or blocked one, is not woken because other work changed; a
        # free member is. A message addressed to anyone always wakes them.
        run = MemoryRepository().run()
        members = [{'id': m, 'status': 'live'} for m in ('parent', 'free', 'parked', 'busy', 'stuck')]
        state = {'tasks': [
            {'id': 1, 'status': 'ready', 'dependencies': [], 'owner': None},
            {'id': 2, 'status': 'submitted', 'dependencies': [], 'owner': 'parked'},
            {'id': 3, 'status': 'claimed', 'dependencies': [], 'owner': 'busy'},
            {'id': 4, 'status': 'blocked', 'dependencies': [], 'owner': 'stuck'},
        ], 'messages': [{'id': 9}]}
        verified_other = [{'action': 'verified', 'detail': {'task': 5}}]
        woken = [m['id'] for m in notification_targets(run, 'parent', members, verified_other, state)]
        self.assertEqual(woken, ['free'])
        message = [{'action': 'message_accepted', 'detail': {'message': 9, 'recipient': 'parked'}}]
        woken = [m['id'] for m in notification_targets(run, 'parent', members, message, state)]
        self.assertEqual(woken, ['parked'])

    def test_parked_members_take_new_work_when_no_worker_is_free(self):
        # Every worker has submitted: ready work would otherwise wake nobody
        # until a review lands, so the parked ones are woken; busy ones not.
        run = MemoryRepository().run()
        members = [{'id': m, 'status': 'live'} for m in ('parent', 'parked', 'busy')]
        state = {'tasks': [
            {'id': 1, 'status': 'ready', 'dependencies': [], 'owner': None},
            {'id': 2, 'status': 'submitted', 'dependencies': [], 'owner': 'parked'},
            {'id': 3, 'status': 'claimed', 'dependencies': [], 'owner': 'busy'},
        ], 'messages': []}
        created = [{'action': 'task_created', 'detail': {'task': 1}}]
        woken = [m['id'] for m in notification_targets(run, 'parent', members, created, state)]
        self.assertEqual(woken, ['parked'])

    def test_no_ready_work_wakes_nobody_and_a_member_holding_two_tasks_stays_busy(self):
        run = MemoryRepository().run()
        members = [{'id': m, 'status': 'live'} for m in ('parent', 'free', 'double')]
        tasks = [{'id': 1, 'status': 'completed', 'dependencies': [], 'owner': 'free'},
                 {'id': 2, 'status': 'submitted', 'dependencies': [], 'owner': 'double'},
                 {'id': 3, 'status': 'claimed', 'dependencies': [], 'owner': 'double'}]
        verified = [{'action': 'verified', 'detail': {'task': 1}}]
        state = {'tasks': tasks, 'messages': []}
        self.assertEqual(notification_targets(run, 'parent', members, verified, state), [])
        state['tasks'] = tasks + [{'id': 4, 'status': 'ready', 'dependencies': [], 'owner': None}]
        woken = [m['id'] for m in notification_targets(run, 'parent', members, verified, state)]
        self.assertEqual(woken, ['free'])

    def test_sender_and_receiver_agree_a_releasing_worker_is_free(self):
        # A worker that releases its task is free to retry it: the sender does
        # not fall back to parked members, and neither does the receiver's
        # accept_wake re-check, which evaluates with actor ''.
        run = MemoryRepository().run()
        members = [{'id': m, 'status': 'live'} for m in ('parent', 'releaser', 'parked')]
        state = {'tasks': [
            {'id': 1, 'status': 'ready', 'dependencies': [], 'owner': None},
            {'id': 2, 'status': 'submitted', 'dependencies': [], 'owner': 'parked'},
        ], 'messages': []}
        released = [{'action': 'released', 'detail': {'task': 1}}]
        sender = [m['id'] for m in notification_targets(run, 'releaser', members, released, state)]
        receiver = [m['id'] for m in notification_targets(run, '', members, released, state)]
        self.assertEqual(sender, ['parent'])
        self.assertEqual(receiver, ['parent', 'releaser'])
        self.assertNotIn('parked', sender + receiver)

    def test_the_fallback_wakes_every_parked_member_but_the_actor(self):
        # One unavailable parked member must not starve the rest, and the
        # actor (here a parked member that released a claim) is not woken by
        # its own event; the receiver's accept_wake (actor '') agrees.
        run = MemoryRepository().run()
        members = [{'id': m, 'status': 'live'} for m in ('parent', 'p1', 'p2', 'p3')]
        state = {'tasks': [{'id': 1, 'status': 'ready', 'dependencies': [], 'owner': None}] + [
            {'id': 10 + n, 'status': 'submitted', 'dependencies': [], 'owner': f'p{n}'} for n in (1, 2, 3)],
            'messages': []}
        released = [{'action': 'released', 'detail': {'task': 1}}]
        sender = [m['id'] for m in notification_targets(run, 'p1', members, released, state)]
        receiver = [m['id'] for m in notification_targets(run, '', members, released, state)]
        self.assertEqual(sender, ['p2', 'p3', 'parent'])
        self.assertEqual(receiver, ['p1', 'p2', 'p3', 'parent'])

    def test_a_claim_that_leaves_ready_work_wakes_members_free_to_take_it(self):
        # The sender woke w1 for task 2; w1 claimed task 1 instead and declines
        # the wake. Its claim must hand the remaining work to the parked p1.
        run = MemoryRepository().run()
        members = [{'id': m, 'status': 'live'} for m in ('parent', 'w1', 'p1')]
        tasks = [{'id': 1, 'status': 'claimed', 'dependencies': [], 'owner': 'w1'},
                 {'id': 2, 'status': 'ready', 'dependencies': [], 'owner': None},
                 {'id': 3, 'status': 'submitted', 'dependencies': [], 'owner': 'p1'}]
        claimed = [{'action': 'claimed', 'detail': {'task': 1}}]
        woken = [m['id'] for m in notification_targets(run, 'w1', members, claimed, {'tasks': tasks, 'messages': []})]
        self.assertEqual(woken, ['p1', 'parent'])
        tasks[1]['status'] = 'claimed'
        self.assertEqual(notification_targets(run, 'w1', members, claimed, {'tasks': tasks, 'messages': []}), [])

    def test_ready_work_with_unmet_dependencies_wakes_nobody(self):
        run = MemoryRepository().run()
        members = [{'id': m, 'status': 'live'} for m in ('parent', 'free')]
        state = {'tasks': [{'id': 1, 'status': 'claimed', 'dependencies': [], 'owner': 'parent'},
                           {'id': 2, 'status': 'ready', 'dependencies': [1], 'owner': None}],
                 'messages': []}
        created = [{'action': 'task_created', 'detail': {'task': 2}}]
        self.assertEqual(notification_targets(run, 'parent', members, created, state), [])

    def test_the_fallback_never_wakes_a_busy_worker(self):
        run = MemoryRepository().run()
        members = [{'id': m, 'status': 'live'} for m in ('parent', 'busy')]
        state = {'tasks': [
            {'id': 1, 'status': 'ready', 'dependencies': [], 'owner': None},
            {'id': 2, 'status': 'claimed', 'dependencies': [], 'owner': 'busy'},
        ], 'messages': []}
        created = [{'action': 'task_created', 'detail': {'task': 1}}]
        self.assertEqual(notification_targets(run, 'parent', members, created, state), [])

    def test_an_amended_contract_wakes_every_live_member_including_parked(self):
        run = MemoryRepository().run()
        members = [{'id': m, 'status': 'live'} for m in ('parent', 'free', 'parked')]
        state = {'tasks': [{'id': 2, 'status': 'submitted', 'dependencies': [], 'owner': 'parked'}],
                 'messages': []}
        amended = [{'action': 'amended', 'detail': {}}]
        woken = [m['id'] for m in notification_targets(run, 'parent', members, amended, state)]
        self.assertEqual(woken, ['free', 'parked'])

    def test_completion_revalidation_and_transition_without_storage(self):
        repo = MemoryRepository()
        service = Coordination(repo, 'parent', lambda: 50)
        with self.assertRaisesRegex(SwarmError, 'stale revision'):
            service.complete('R2')
        service.revalidate_task(1, 'R2', [{'artifact': 'rerun', 'revision': 'R2'}])
        service.complete('R2')
        self.assertEqual((repo.run()['status'], repo.run()['outcome']), ('paused', 'succeeded'))
        self.assertEqual([e[0] for e in repo.state['events'][-3:]], ['completed', 'stop', 'paused'])
        self.assertEqual(repo.state['events'][-4][1]['previous_evidence'][0]['revision'], 'R1')

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
        self.assertEqual((repo.run()['status'], repo.run()['outcome']), ('paused', 'budget-exhausted'))
        self.assertEqual(repo.state['events'][0], ('stop', {'status':'budget-exhausted','reason':'deadline'}))
        self.assertEqual(repo.state['events'][1][0], 'paused')
        self.assertEqual(repo.state['events'][1][1]['outcome'], 'budget-exhausted')

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


class OwnerStateBehavior(unittest.TestCase):
    def test_owner_state_is_affirmative_over_every_member_status(self):
        now = 1000.0
        recent, stale = now - OWNER_IDLE_AFTER + 1, now - OWNER_IDLE_AFTER
        self.assertEqual(owner_state('live', False, recent, now), 'active')
        self.assertEqual(owner_state('live', False, stale, now), 'idle')
        self.assertEqual(owner_state('live', False, None, now), 'unknown')
        self.assertEqual(owner_state('live', True, recent, now), 'lost')
        self.assertEqual(owner_state('reserved', False, recent, now), 'reserved')
        self.assertEqual(owner_state('reserved', True, recent, now), 'lost')
        self.assertEqual(owner_state('dead', False, recent, now), 'dead')
        self.assertEqual(owner_state('dead', True, None, now), 'dead')
        for status in (None, '', 'lost', 'suspended', 'LIVE', 'future-status', 7):
            self.assertEqual(owner_state(status, False, recent, now), 'unknown', status)
            self.assertEqual(owner_state(status, True, recent, now), 'unknown', status)
        self.assertEqual(owner_state('live', False, 0.0, 20.0, idle_after=30.0), 'active')
        self.assertNotIn('suspended', OWNER_STATES)
        self.assertEqual(ADDRESSABLE_OWNER_STATES, ('active', 'idle'))

    def test_owner_recovery_names_the_coordinator_move_per_state(self):
        self.assertEqual(owner_recovery('dead'), 'recover(task) or revoke(task, reason)')
        self.assertIn('resume the run', owner_recovery('lost'))
        for state in ('reserved', 'unknown'):
            self.assertEqual(owner_recovery(state), 'revoke(task, reason)')

    def test_idle_transition_is_the_future_instant_or_none(self):
        self.assertEqual(idle_transition(100.0, 150.0), 100.0 + OWNER_IDLE_AFTER)
        self.assertIsNone(idle_transition(100.0, 100.0 + OWNER_IDLE_AFTER))
        self.assertIsNone(idle_transition(None, 150.0))
        self.assertEqual(idle_transition(100.0, 105.0, idle_after=10.0), 110.0)


if __name__ == '__main__':
    unittest.main()

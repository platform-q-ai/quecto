"""Coordination use cases. Repository transactions and clock are injected ports.

The transaction yields domain operations, never a connection or SQL cursor.
All decisions and writes occur within the same atomic transaction.
"""
from contextlib import contextmanager
from typing import Protocol
from swarm_policy import authorize, expired, require_budget, admission, completion, revalidation, notification_targets


class CoordinationTransaction(Protocol):
    def run(self): ...
    def member(self, identity): ...
    def usage(self): ...
    def reserve_member(self, identity, reservation): ...
    def completion_state(self): ...
    def task(self, identity): ...
    def replace_task_evidence(self, identity, evidence): ...
    def set_outcome(self, status): ...
    def event(self, action, detail): ...
    def members(self): ...
    def notification_events(self, actor): ...
    def notification_state(self): ...
    def advance_notifications(self, actor): ...


class CoordinationRepository(Protocol):
    def atomic(self): ...


class Coordination:
    def __init__(self, repository: CoordinationRepository, actor, clock):
        self.repository, self.actor, self.clock = repository, actor, clock

    @contextmanager
    def operation(self, active=True, coordinator=False, read_only=False):
        # Expiry must commit even when a following mutation is rejected.
        with self.repository.atomic() as tx:
            run = tx.run()
            authorize(run, self.actor, tx.member(self.actor), read_only=True)
            if expired(run, self.clock()):
                tx.set_outcome('budget-exhausted')
                tx.event('stop', {'status': 'budget-exhausted', 'reason': 'deadline'})
        with self.repository.atomic() as tx:
            run = tx.run()
            authorize(run, self.actor, tx.member(self.actor), active, coordinator, read_only)
            if active:
                require_budget(run, self.clock())
            yield tx

    def reserve_member(self, identity, reservation):
        with self.operation(active=False) as tx:
            if admission(tx.run(), tx.member(identity), reservation, tx.usage(), self.clock()):
                tx.reserve_member(identity, reservation)
                tx.event('reserved', {'member': identity})
            return tx.member(identity)

    def complete(self, revision):
        with self.operation(coordinator=True) as tx:
            state = tx.completion_state()
            outcome = completion(state['criteria'], state['evidence'], state['tasks'],
                                 state['has_reservations'], revision)
            tx.set_outcome(outcome)
            tx.event('completed', {'revision': revision})

    def revalidate_task(self, identity, revision, evidence):
        with self.operation(coordinator=True) as tx:
            task = tx.task(identity)
            evidence = revalidation(task, revision, evidence)
            tx.replace_task_evidence(identity, evidence)
            tx.event('revalidated', {'task': identity, 'revision': revision,
                                    'previous_evidence': task['evidence'], 'evidence': evidence})

    def notifications(self):
        with self.operation(active=False, read_only=True) as tx:
            targets = notification_targets(tx.run(), self.actor, tx.members(), tx.notification_events(self.actor), tx.notification_state())
            # Hints are best-effort, not durable delivery. Advance atomically
            # before sending so concurrent/background calls cannot duplicate them.
            tx.advance_notifications(self.actor)
            return targets

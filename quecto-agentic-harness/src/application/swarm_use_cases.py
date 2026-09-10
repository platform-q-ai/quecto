"""Coordination use cases. Repository transactions and clock are injected ports.

The transaction yields domain operations, never a connection or SQL cursor.
All decisions and writes occur within the same atomic transaction.
"""
from contextlib import contextmanager
from typing import Protocol
from swarm_policy import (SwarmError, PROPOSED_OUTCOMES, STOP_STATUSES, describe, authorize, expired, require_budget, admission, completion,
                          revalidation, notification_targets, usage_budget_decision, request_measurement, resume_blockers,
                          validate_extension)


class CoordinationTransaction(Protocol):
    def run(self): ...
    def member(self, identity): ...
    def usage(self): ...
    def reserve_member(self, identity, reservation): ...
    def completion_state(self): ...
    def task(self, identity): ...
    def replace_task_evidence(self, identity, evidence): ...
    def set_outcome(self, status): ...
    def propose_outcome(self, outcome, reason): ...
    def clear_outcome(self): ...
    def event(self, action, detail): ...
    def members(self): ...
    def pause_started(self): ...
    def control_receipt(self): ...
    def claim_wake_events(self, actor, generation): ...
    def set_deadline(self, deadline): ...
    def notification_events(self, actor): ...
    def notification_state(self): ...
    def advance_notifications(self, actor): ...
    def usage_report(self): ...
    def configure_usage_budget(self, limit, strict_unknown): ...
    def record_request(self, actor, record, measurement): ...
    def mark_usage_warning(self): ...


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
                self._end(tx, 'budget-exhausted', 'deadline')
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

    def _end(self, tx, outcome, reason):
        """End the run as a resumable pause holding `outcome` (#1729)."""
        tx.propose_outcome(outcome, reason)
        tx.event('stop', {'status': outcome, 'reason': reason})
        tx.event('paused', {'reason': reason, 'started': self.clock(), 'outcome': outcome})

    def complete(self, revision):
        with self.operation(coordinator=True) as tx:
            state = tx.completion_state()
            outcome = completion(state['criteria'], state['evidence'], state['tasks'],
                                 state['has_reservations'], revision)
            tx.event('completed', {'revision': revision})
            self._end(tx, outcome, f'completed at {revision}')

    def stop(self, status, reason):
        if status not in STOP_STATUSES:
            raise SwarmError('invalid non-success outcome')
        with self.operation(active=False, coordinator=True) as tx:
            run = tx.run()
            if status == 'cancelled':
                if run['status'] == 'cancelled':
                    return tx.control_receipt()
                if run['status'] == 'setup':
                    raise SwarmError('run not created yet; nothing to cancel')
                if run['status'] not in ('running', 'paused'):
                    raise SwarmError(f"run already {describe(run)}")
                tx.clear_outcome()
                tx.set_outcome('cancelled')
                tx.event('stop', {'status': status, 'reason': reason})
                return tx.control_receipt()
            if run['status'] == 'paused' and run.get('outcome') == status:
                return tx.control_receipt()
            if run['status'] != 'running':
                raise SwarmError(f"run already {describe(run)}; only the supervisor can resume or close it")
            self._end(tx, status, reason)
            return tx.control_receipt()

    def revalidate_task(self, identity, revision, evidence):
        with self.operation(coordinator=True) as tx:
            task = tx.task(identity)
            evidence = revalidation(task, revision, evidence)
            tx.replace_task_evidence(identity, evidence)
            tx.event('revalidated', {'task': identity, 'revision': revision,
                                    'previous_evidence': task['evidence'], 'evidence': evidence})

    def pause(self, reason):
        with self.operation(active=False, coordinator=True) as tx:
            if tx.run()['status'] == 'paused':
                return tx.control_receipt()
            authorize(tx.run(), self.actor, tx.member(self.actor), active=True)
            tx.set_outcome('paused')
            tx.event('paused', {'reason': reason, 'started': self.clock()})
            return tx.control_receipt()

    def resume(self, external=False):
        """Only the supervisor outside the swarm resumes a paused run (#1729)."""
        if not external:
            raise SwarmError('a paused run is resumed only by the supervisor outside the swarm '
                             '(agent_cmd swarm_control resume); members cannot resume it')
        with self.operation(active=False, coordinator=True) as tx:
            run = tx.run()
            if run['status'] == 'running':
                return tx.control_receipt()
            if run['status'] != 'paused':
                raise SwarmError('only a paused run may resume')
            now = self.clock()
            elapsed = max(0, now - tx.pause_started())
            deadline = run['deadline'] + elapsed
            report = tx.usage_report()
            blockers = resume_blockers(run, deadline, now, usage_budget_decision(report['budget'], report['totals']))
            if blockers:
                raise SwarmError('resume would pause again at once: ' + '; '.join(blockers))
            tx.set_deadline(deadline)
            tx.clear_outcome()
            tx.set_outcome('running')
            tx.event('resumed', {'paused_seconds': elapsed, 'outcome': run.get('outcome')})
            return tx.control_receipt()

    def close(self, external=False):
        """Make the outcome a paused run holds terminal; supervisor only (#1729)."""
        if not external:
            raise SwarmError('a run is closed only by the supervisor outside the swarm (agent_cmd swarm_control close)')
        with self.operation(active=False, coordinator=True) as tx:
            run = tx.run()
            if run['status'] != 'paused' or run.get('outcome') not in PROPOSED_OUTCOMES:
                raise SwarmError(f"run is {run['status']} without a proposed outcome; resume it or cancel the run")
            tx.set_outcome(run['outcome'])
            tx.event('closed', {'status': run['outcome'], 'reason': run.get('outcome_reason')})
            return tx.control_receipt()

    def extend_deadline(self, seconds, external=False):
        """Grant wall-clock budget to a paused run; supervisor only (#1729)."""
        if not external:
            raise SwarmError('a deadline is extended only by the supervisor outside the swarm (agent_cmd swarm_control extend)')
        seconds = validate_extension(seconds)
        with self.operation(active=False, coordinator=True) as tx:
            run = tx.run()
            if run['status'] not in ('running', 'paused'):
                raise SwarmError(f"run is {run['status']}; nothing to extend")
            # A paused run gets `seconds` of budget from the moment it resumes:
            # resume adds the paused interval back, so grant from the later of
            # the old deadline and the pause start, not from a stale deadline.
            base = run['deadline']
            if run['status'] == 'paused':
                base = max(base, tx.pause_started())
            if base + seconds > self.clock() + 604800:
                raise SwarmError('deadline may be at most seven days ahead, as at creation')
            tx.set_deadline(base + seconds)
            tx.event('extended', {'seconds': seconds, 'deadline': base + seconds})
            return tx.control_receipt()

    def accept_wake(self, generation):
        from swarm_policy import SwarmError
        if type(generation) is not int or generation < 0:
            raise SwarmError('wake generation must be a nonnegative integer')
        with self.operation(active=False, read_only=True) as tx:
            events = tx.claim_wake_events(self.actor, generation)
            return any(member['id'] == self.actor for member in notification_targets(
                tx.run(), '', tx.members(), events, tx.notification_state()))

    def notifications(self, with_generation=False):
        with self.operation(active=False, read_only=True) as tx:
            targets = notification_targets(tx.run(), self.actor, tx.members(), tx.notification_events(self.actor), tx.notification_state())
            # Hints are best-effort, not durable delivery. Advance atomically
            # before sending so concurrent/background calls cannot duplicate them.
            generation = tx.advance_notifications(self.actor)
            return {'members': targets, 'generation': generation} if with_generation else targets

    def usage_budget(self, limit, strict_unknown=True):
        from swarm_policy import SwarmError
        if (limit is None or (type(limit) is int and 0 < limit <= 2**63 - 1)) and type(strict_unknown) is bool:
            with self.operation(active=False, coordinator=True) as tx:
                tx.configure_usage_budget(limit, strict_unknown)
                self._apply_usage_budget(tx)
                return tx.usage_report()
        raise SwarmError('token limit must be positive or None, strict_unknown must be boolean')

    def usage_report(self):
        with self.operation(active=False, read_only=True) as tx:
            return tx.usage_report()

    def record_request(self, record):
        measurement = request_measurement(record)
        with self.operation(active=False, read_only=True) as tx:
            tx.record_request(self.actor, record, measurement)
            self._apply_usage_budget(tx)
            return tx.control_receipt()

    def request_admission(self):
        with self.operation(active=False, read_only=True) as tx:
            self._apply_usage_budget(tx)
            run = tx.run()
            return {'status': run['status'], 'coordinator': run['coordinator'],
                    'deadline': run['deadline'], 'members': tx.members(),
                    'outcome': run.get('outcome'),
                    'control_generation': tx.control_receipt()['generation']}

    def _apply_usage_budget(self, tx):
        report = tx.usage_report()
        decision = usage_budget_decision(report['budget'], report['totals'])
        if decision in ('warn', 'pause') and report['budget']['warned'] is False:
            tx.mark_usage_warning()
            tx.event('usage-warning', {'decision': decision, 'observed_tokens': report['totals']['observed_tokens'],
                                      'token_limit': report['budget']['token_limit'], 'unknown_usage_requests': report['totals']['unknown_usage_requests']})
        if decision == 'pause' and tx.run()['status'] == 'running':
            self._end(tx, 'budget-exhausted', 'observed usage budget or unavailable measurement')

    def control_status(self):
        with self.operation(active=False, read_only=True) as tx:
            return tx.control_receipt()

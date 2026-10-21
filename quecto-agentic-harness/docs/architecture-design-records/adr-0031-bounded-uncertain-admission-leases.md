# ADR-0031: Uncertain admission leases clear at a bound

**Status:** Accepted
**Date:** 2026-10-20
**Scope:** #2664; narrowly supersedes [ADR-0026](adr-0026-shared-inference-admission.md)'s "no expiry-based free slot" outcome for client death and restart orphans, and its rejection of lease-expiry reclamation.

## Context

ADR-0026 keeps an attempt whose client vanished as uncertain occupancy that
quarantines its whole quota group until the same capability completes it or an
operator resets the authority. In practice nothing completes it: a root whose
connection drops re-registers a new scope and a child has no reconnect, so one
client killed mid-request (an outer `timeout`, SIGTERM) wedged every session on
the host for that group until `quecto admission-broker reset`, which revokes
every capability. Restart orphans had the same unbounded lifetime.

ADR-0026 also fixes what the bound covers: locally owned outbound transports,
not remote computation a provider may continue after disconnection. Every grant
carries an attempt deadline. At that deadline the authority tells a connected
owner to stop its transport; an owner whose authority connection closes stops
its transport on the close; a dead owner has no transport.

## Decision

An uncertain lease clears by itself a fixed margin
(`UNCERTAIN_LEASE_MARGIN_MS`, 60 s) for owner teardown after the later of its
attempt's deadline and the instant its owner was lost, so an owner lost long
after its deadline (stopped, starved) still gets the whole margin to tear its
transport down. An attempt still owned by a connected client never clears on
time alone, even past its deadline. Before then the
group stays quarantined exactly as before; completion by the same capability
and an operator reset still clear it sooner. A late completion of a cleared
lease is acknowledged and frees nothing a second time.

The durable ledger records each outstanding attempt's remaining time to that
bound (an additive, optional field; the format number is unchanged). A restart
turns outstanding work into orphans that clear when their remaining time has
passed on the new clock; an attempt whose owner was still connected at the
last write counts its margin from that write at the earliest. The ledger is
written when a lease or orphan clears on time and when the authority stops
gracefully, besides every grant, completion and reset, so a graceful restart
lengthens a lease only by its downtime; after a crash the uptime since the
last write is served again. An entry without
a recorded bound (written before this decision) or with one beyond the group's
attempt timeout plus the margin is bounded by the attempt timeout plus the
margin from the restart.

A scope left holding only cleared work and no bound connection is released,
as a scope that disconnects idle already is, so lost clients do not leak scope
slots.

A wait that expires while uncertain leases hold its group is answered with the
group, the number of leases, when they clear and the reset command, extending
the plain queue-timeout reason so older clients still read a queue timeout.

## Consequences

One killed client blocks its group for at most its attempt timeout plus the
margin after its grant, or, for a client lost after its deadline, the margin
after it was lost (plus downtime, and after a crash the uptime since the last
write); not until a reset. A provider that keeps computing after the local
transport is gone can briefly exceed the configured concurrency after the
lease clears; ADR-0026 already excludes that remote work from the bound.
Process-identity (pid) tracking of owners was rejected: the broker would have
to trust reusable identifiers, and the deadline already bounds the transport.

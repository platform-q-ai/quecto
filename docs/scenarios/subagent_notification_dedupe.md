# Subagent notification dedupe

## Scenario: a completed subagent summary is injected once

Given a subagent emits a completion notification with a monotonic notification id
When the parent dispatch loop observes the same notification around a prompt boundary
Then the parent enqueues the completion summary only once
And the master agent receives a single follow-up containing that summary.

## Scenario: subsequent completions from the same subagent are still delivered

Given a subagent completes one prompt and later completes another prompt
When each completion has a larger monotonic notification id
Then the parent injects both summaries exactly once.

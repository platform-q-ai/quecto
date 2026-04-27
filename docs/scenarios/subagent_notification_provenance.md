# Subagent notification provenance

## Scenario: subagent completions are not injected as human user messages

Given a spawned subagent completes asynchronously
When the parent receives its completion notification
Then the parent records the notification as a system message with explicit subagent/tool provenance
And the master agent can distinguish it from text typed by the human user.

## Scenario: delayed subagent notifications remain provider-safe

Given provider APIs require tool results to be adjacent to their tool calls
When a subagent completion arrives after later conversation turns
Then quecto does not replay it as a delayed `role=tool` message
And instead uses a provider-safe system notification wrapper.

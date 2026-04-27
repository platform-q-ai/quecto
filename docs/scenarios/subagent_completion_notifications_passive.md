# Scenario: Subagent completion notifications are passive UI events

Given a main agent spawns multiple asynchronous subagents
And those subagents later become idle, completed, failed, or exited
When the subagent monitor reports completion notifications to the parent UDS session
Then QuEcto should emit those notifications to connected clients for human visibility
But it should not enqueue pending prompts for the main agent
And the main agent should not automatically produce one acknowledgement per completed subagent

When the main agent needs subagent results
Then the `spawn` tool description should instruct it to use `agent_cmd` with `command: "await"`
And then inspect output explicitly with `get_messages_tail` or `get_messages`
And aggregate/summarize results only after collecting the relevant subagent outputs.

Notes:
- Completion notifications are operational events, not user prompts.
- `agent_cmd await` remains the tool-originated synchronization point for model-visible subagent state.

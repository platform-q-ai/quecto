@done
Feature: Context pruning via sliding window and tool-call collapse

  Tool outputs remain in full context until either (a) the number of tool
  calls in the session exceeds context_collapse_after_tool_calls (default
  50), at which point the oldest tool results are collapsed to compact
  recall() stubs, or (b) they are dropped by the sliding window when the
  conversation exceeds the token budget. The collapse trigger counts tool
  calls cumulatively across prompts within a session rather than turns
  elapsed. Collapse can be disabled entirely. Spill-to-disk still occurs at
  creation time so recall() can retrieve collapsed or dropped outputs.

  Background:
    Given a configured agent with context pruning enabled

  # --- Tool-result collapse triggers on tool-call count (#1017) ---

  Scenario: Tool outputs collapse after the configured number of tool calls
    Given context_collapse_after_tool_calls is set to 50
    When the agent has executed 51 tool calls in the session
    Then the oldest tool result is collapsed to a recall() stub
    And the 50 most recent tool results remain in full context

  Scenario: Collapse count is cumulative across prompts within a session
    Given context_collapse_after_tool_calls is set to 50
    And the agent has already executed 30 tool calls in an earlier prompt
    When the agent executes 25 more tool calls in a later prompt
    Then 5 tool results are collapsed to recall() stubs

  Scenario: Collapse can be disabled
    Given context collapse is disabled
    When the agent has executed 100 tool calls in the session
    Then no tool results are collapsed

  Scenario: Default collapse threshold is 50 tool calls
    Given a default agent configuration
    Then the context_collapse_after_tool_calls default is 50

  # --- Tool results stay in full context ---

  Scenario: Tool results remain in full context regardless of age
    When the agent executes a bash tool on turn 1
    And the agent completes turn 2
    And the agent completes turn 3
    And the agent completes turn 4
    And the agent completes turn 10
    And the agent completes turn 20
    Then the tool result from turn 1 is still in full context

  Scenario: User and assistant messages are never collapsed
    When the agent processes 20 turns of mixed tool and text messages
    Then all user messages remain in full context
    And all assistant messages remain in full context
    And no tool messages are collapsed

  Scenario: System messages are never collapsed
    Given a system prompt in the conversation
    When the agent processes 20 turns
    Then the system [message] remains in full context

  # --- Spill-to-disk still works at creation time ---

  Scenario: Full tool output is spilled to disk on creation
    When the agent executes a bash tool on turn 1
    Then the spill file contains an entry with id "turn1:bash:0"
    And the spill entry content matches the original tool output

  Scenario: Recall retrieves spilled output
    Given a spilled tool result with id "turn5:bash:0"
    When the agent calls recall with id "turn5:bash:0"
    Then the recall result contains the full original output

  Scenario: Recall with unknown ID returns error
    When the agent calls recall with id "nonexistent:id:0"
    Then the recall result is an error containing "No spilled output found"

  Scenario: Recall list returns full index
    Given 5 spilled tool results
    When the agent calls recall with id "list"
    Then the result contains all 5 spill entry IDs
    And the result contains tool names and token counts
    And the result does not contain full content

  Scenario: Repeated recall emits diagnostic warning
    Given a spilled tool result with id "turn5:bash:0"
    When the agent calls recall with id "turn5:bash:0" three times
    Then a warning is logged with target "context_prune"
    And the warning contains "repeated recall"
    And the warning contains recall_count 3

  # --- Spill manifest ---

  Scenario: Spill manifest is injected after first spill
    Given no spill entries exist
    When the agent executes a bash tool on turn 1
    Then a pinned manifest [message] appears in context
    And the manifest contains "1 spilled entries via recall()"

  Scenario: Spill manifest shows last 10 entries
    Given 25 spilled tool results
    Then the manifest lists the 10 most recent entries
    And the manifest shows total count as 25
    And the manifest shows the oldest and latest entry IDs

  Scenario: Spill manifest survives sliding window
    Given max_context_tokens is set to 500
    And 20 spilled tool results
    When the sliding window drops messages to fit budget
    Then the manifest [message] remains in context
    And the manifest is pinned

  Scenario: Spill manifest is updated in-place
    When the agent executes tools on turns 1 through 5
    Then only one manifest [message] exists in context
    And it reflects all 5 spill entries

  Scenario: No manifest when no spill entries exist
    When the agent processes 3 turns with no tool calls
    Then no manifest [message] exists in context

  @done
  Scenario: Spill store caches index in memory after append
    When 3 spill entries are appended to the store
    Then list_entries returns 3 entries without re-reading disk

  @done
  Scenario: recall only deserializes the matching spill entry
    When recall is called for the 5th entry in a 10-entry spill file
    Then the correct entry is returned with full content

  # --- Sliding window enforcement ---

  Scenario: Sliding window drops oldest messages when over budget
    Given max_context_tokens is set to 1000
    When the agent accumulates 2000 tokens of messages
    Then the oldest non-pinned messages are dropped
    And total context is under 1000 tokens

  Scenario: System messages are never dropped by sliding window
    Given max_context_tokens is set to 500
    And a system prompt consuming 200 tokens
    When the agent accumulates 800 tokens of messages
    Then the system [message] remains in full context
    And non-system messages are dropped to fit

  Scenario: First user message is pinned
    Given max_context_tokens is set to 500
    When the agent accumulates 800 tokens across 5 user messages
    Then the first user [message] remains in context
    And later user messages may be dropped

  # --- #951: spill + recall conversation messages, not just tool outputs ---

  Scenario: Budget-dropped assistant message is spilled before dropping
    Given max_context_tokens is set to 200
    And recent-turn pinning is set to 2 turns
    And an old assistant message of 500 tokens on turn 1
    When the spilling sliding window drops messages to fit budget
    Then the spill file contains an entry with id "turn1:msg:assistant"
    And the spill entry content matches the original assistant text

  Scenario: Budget-dropped assistant message is recallable
    Given max_context_tokens is set to 200
    And recent-turn pinning is set to 2 turns
    And an old assistant message of 500 tokens on turn 1
    And the spilling sliding window has dropped messages to fit budget
    When the agent calls recall with id "turn1:msg:assistant"
    Then the recall result contains the full original assistant text

  Scenario: Budget-dropped user message is spilled with its role in the id
    Given max_context_tokens is set to 200
    And recent-turn pinning is set to 2 turns
    And an old user message of 500 tokens on turn 1
    When the spilling sliding window drops messages to fit budget
    Then the spill file contains an entry with id "turn1:msg:user"

  Scenario: Message spills are distinguishable from tool spills in the manifest
    Given max_context_tokens is set to 200
    And recent-turn pinning is set to 2 turns
    And a spilled tool result with id "turn1:bash:0"
    And an old assistant message of 500 tokens on turn 1
    When the spilling sliding window drops messages to fit budget
    Then the manifest contains "turn1:bash:0"
    And the manifest contains "turn1:msg:assistant"

  Scenario: Manifest reflects a message spill on a turn with no tool calls
    Given max_context_tokens is set to 200
    And recent-turn pinning is set to 2 turns
    And an old assistant message of 500 tokens on turn 1
    When the agent completes a prompt with no tool calls
    Then a pinned manifest [message] appears in context
    And the manifest contains "turn1:msg:assistant"

  Scenario: System prompt and manifest are never dropped by the spilling sliding window
    Given max_context_tokens is set to 10
    And recent-turn pinning is set to 2 turns
    And a system prompt in the conversation
    And a spilled tool result with id "turn1:bash:0"
    And an old assistant message of 500 tokens on turn 1
    When the spilling sliding window drops messages to fit budget
    Then the system [message] remains in full context
    And the manifest [message] remains in context

  Scenario: Most-recent turns are never dropped by the sliding window
    Given max_context_tokens is set to 10
    And recent-turn pinning is set to 2 turns
    And messages from turns 1 through 4 each exceeding the budget
    When the spilling sliding window drops messages to fit budget
    Then messages from the most recent 2 turns remain in context
    And messages from older turns are dropped

  Scenario: Current user prompt is never dropped by the sliding window
    Given max_context_tokens is set to 50
    And recent-turn pinning is set to 2 turns
    And a user prompt exceeding the budget
    When the spilling sliding window drops messages to fit budget
    Then the current user prompt remains in context

  # --- #1046: spill conversation messages at creation + count-based collapse ---

  Scenario: Conversation messages are spilled at creation and immediately recallable
    When the agent completes a text-only prompt on turn 1
    Then the spill file contains an entry with id "turn1:msg:assistant"
    And the spill entry for "turn1:msg:assistant" matches the assistant reply

  Scenario: Creation-time message spill ids never collide across prompts
    Given the agent has completed a text-only prompt on turn 1
    When the agent completes another text-only prompt on turn 1
    Then the spill file contains an entry with id "turn1:msg:assistant"
    And the spill file contains an entry with id "turn1:msg:assistant:2"

  Scenario: Ephemeral sessions spill conversation messages at creation
    Given the session is ephemeral
    When the agent completes a text-only prompt on turn 1
    Then the ephemeral session spill contains a recallable entry with id "turn1:msg:assistant"

  Scenario: Ephemeral sessions still spill tool output at creation
    Given the session is ephemeral
    When the agent runs a bash tool
    Then the ephemeral session spill contains a recallable entry whose tool is "bash"

  Scenario: Rewinding past collapsed conversation messages leaves no empty turns
    Given context_collapse_after_messages is set to 0
    And 4 old conversation messages
    And an in-flight user prompt
    And the old conversation messages have been collapsed to recall stubs
    When the conversation is rewound to the in-flight user prompt
    Then the collapsed conversation messages survive the rewind with non-empty content

  Scenario: Oldest conversation messages collapse once the message count exceeds the threshold
    Given context_collapse_after_messages is set to 3
    And 4 old conversation messages
    And an in-flight user prompt
    When the agent trims old conversation messages
    Then 1 conversation message is collapsed to a recall stub
    And the oldest conversation message is a one-line recall stub

  Scenario: Message collapse triggers at one past the threshold, not at it
    Given context_collapse_after_messages is set to 3
    And 3 old conversation messages
    And an in-flight user prompt
    When the agent trims old conversation messages
    Then 0 conversation messages are collapsed to recall stubs

  Scenario: Assistant and user messages share one combined collapse count
    Given context_collapse_after_messages is set to 3
    And 2 old assistant messages and 2 old user messages
    And an in-flight user prompt
    When the agent trims old conversation messages
    Then 1 conversation message is collapsed to a recall stub

  Scenario: Tool results are excluded from the message collapse count
    Given context_collapse_after_messages is set to 3
    And 3 old conversation messages
    And an in-flight user prompt
    And 10 un-collapsed tool results in the session
    When the agent trims old conversation messages
    Then 0 conversation messages are collapsed to recall stubs
    And no tool results are collapsed by the message trigger

  Scenario: Message collapse is cumulative across prompts
    Given context_collapse_after_messages is set to 3
    And 4 old conversation messages from an earlier prompt
    And 4 old conversation messages
    And an in-flight user prompt
    When the agent trims old conversation messages
    Then 5 conversation messages are collapsed to recall stubs

  Scenario: The system prompt is never collapsed by the message trigger
    Given context_collapse_after_messages is set to 0
    And a system prompt in the conversation
    And 2 old conversation messages
    And an in-flight user prompt
    When the agent trims old conversation messages
    Then the system prompt is not collapsed
    And at least 1 conversation message is collapsed to a recall stub

  Scenario: The spill manifest is never collapsed by the message trigger
    Given context_collapse_after_messages is set to 0
    And a pinned manifest message in the conversation
    And 2 old conversation messages
    And an in-flight user prompt
    When the agent trims old conversation messages
    Then the manifest message is not collapsed
    And at least 1 conversation message is collapsed to a recall stub

  Scenario: The in-flight user prompt is never collapsed by the message trigger
    Given context_collapse_after_messages is set to 0
    And 2 old conversation messages
    And an in-flight user prompt
    When the agent trims old conversation messages
    Then the in-flight user prompt is not collapsed
    And at least 1 conversation message is collapsed to a recall stub

  Scenario: Messages within the pinned recent-turn tail are never collapsed by the message trigger
    Given context_collapse_after_messages is set to 0
    And recent-turn pinning is set to 1 turns
    And 2 old conversation messages
    And an in-flight user prompt
    And a conversation message within the pinned recent-turn tail
    When the agent trims old conversation messages
    Then the tail-pinned conversation message is not collapsed
    And at least 1 conversation message is collapsed to a recall stub

  Scenario: Message collapse can be disabled with the sentinel
    Given message collapse is disabled
    And 100 old conversation messages
    And an in-flight user prompt
    When the agent trims old conversation messages
    Then 0 conversation messages are collapsed to recall stubs

  Scenario: Collapsed message stubs count toward the token budget
    Given context_collapse_after_messages is set to 0
    And 2 old conversation messages
    And an in-flight user prompt
    When the agent trims old conversation messages
    Then each collapsed message stub has a nonzero token estimate
    And the stub token estimate is below the original message estimate

  Scenario: Budget pressure collapses messages to stubs before dropping anything
    Given max_context_tokens is set to 150
    And recent-turn pinning is set to 1 turns
    And 4 old conversation messages
    And an in-flight user prompt
    When the agent enforces the context ceiling
    Then at least 1 old message is reduced to a recall stub by the ceiling
    And no messages are removed from the conversation
    And total context is under 150 tokens

  Scenario: Budget pressure removes stubs entirely only when stubbing is not enough
    Given max_context_tokens is set to 5
    And recent-turn pinning is set to 0 turns
    And 4 old conversation messages
    And an in-flight user prompt
    When the agent enforces the context ceiling
    Then at least 1 message is removed from the conversation
    And no full un-collapsed conversation message was removed before stubbing

  # --- #1045: configurable pin_recent_turns ---

  Scenario: pin_recent_turns defaults to 2 and message collapse defaults to disabled
    Given a default agent configuration
    Then the configured pin_recent_turns is 2
    And the configured context_collapse_after_messages is disabled

  Scenario: A non-default pin_recent_turns changes pinning behaviour
    Given max_context_tokens is set to 10
    And recent-turn pinning is set to 3 turns
    And messages from turns 1 through 4 each exceeding the budget
    When the spilling sliding window drops messages to fit budget
    Then messages from the most recent 3 turns remain in context
    And messages from older turns are dropped

  # --- #1044: observable over-budget + window-aware clamp ---

  Scenario: The demotion ladder reports an unmeetable budget
    Given max_context_tokens is set to 5
    And recent-turn pinning is set to 0 turns
    And a user prompt exceeding the budget
    When the agent enforces the context ceiling
    Then the context budget is reported as unmet

  Scenario: An unmeetable ceiling is reflected in the ContextPruned audit event
    Given max_context_tokens is set to 5
    When the agent completes a prompt exceeding the budget
    Then the ContextPruned audit event records the budget as unmet

  Scenario: Effective context budget derives from the model window when known
    Given a configured agent with max_context_tokens 200000
    And the active model has a known context window of 100000 tokens
    When the agent derives its effective context budget
    Then the effective context budget is 100000

  Scenario: Config max_context_tokens overrides a larger model window
    Given a configured agent with max_context_tokens 200000
    And the active model has a known context window of 1000000 tokens
    When the agent derives its effective context budget
    Then the effective context budget is 200000

  Scenario: Unknown model windows fall back to the configured budget
    Given a configured agent with max_context_tokens 200000
    And the active model has no known context window
    When the agent derives its effective context budget
    Then the effective context budget is 200000

  # --- #305: Improved token estimation heuristic ---

  Scenario: Token estimation uses 4 chars per token for ASCII prose
    Given a string of 400 ASCII characters
    Then the estimated token count should be 100

  Scenario: Token estimation applies ceiling division for short strings
    Given a string of 2 ASCII characters
    Then the estimated token count should be 1

  Scenario: Token estimation for CJK text uses 1 token per character
    Given a string of 100 CJK characters
    Then the estimated token count should be 100

  # --- Default max context tokens is 200,000 ---

  @done
  Scenario: Default max context tokens is 200000
    Given a default agent configuration
    Then the max_context_tokens is 200000

  # --- Session persistence ---

  Scenario: Pruning metadata survives session save and load
    When the agent executes a bash tool on turn 1
    When the [session] is saved and reloaded from disk
    Then the tool result from turn 1 still has turn 1
    And the tool result from turn 1 still has tool_name "bash"
    And the tool result from turn 1 still has spill_id "turn1:bash:0"

  Scenario: Manifest is not duplicated after session save and reload
    When the agent executes tools on turns 1 through 5
    Then only one manifest [message] exists in context
    When the [session] is saved and reloaded from disk
    And the spill manifest is updated
    Then only one manifest [message] exists in context
    And exactly one system [message] contains "spilled entries via recall()"

  Scenario: Tool results remain uncollapsed after session round-trip
    When the agent executes a bash tool on turn 1
    And the agent completes turn 4
    Then the tool result from turn 1 is still in full context
    When the [session] is saved and reloaded from disk
    And the agent completes turn 5
    Then the tool result from turn 1 is still in full context

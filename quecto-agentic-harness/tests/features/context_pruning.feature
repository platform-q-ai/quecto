@done
Feature: Context management: the watermark pass, the emergency ladder, spill and recall

  The watermark pass is the only context mode (#2414): the context only
  grows at its end, and once a request reaches the high mark one cut takes
  it down to the low mark, archiving what it drops to session memory. No
  earlier message is edited in place between cuts. Only when no cut can
  bring a request under the ceiling does the emergency ladder run: it stubs,
  then drops, the oldest messages, never the system prompt, the spill
  manifest, the in-flight prompt or the pinned recent turns. Every tool
  result and conversation message is spilled at creation, so recall()
  retrieves whatever was archived, stubbed or dropped.

  Background:
    Given a configured agent with context pruning enabled

  # --- Phase 1: context subsystem boundary ---

  @phase-1-hardening
  Scenario: Recent turns remain in provider context when older turns exceed the budget
    Given max_context_tokens is set to 10
    And recent-turn pinning is set to 2 turns
    And messages from turns 1 through 4 each exceeding the budget
    When the next provider context is prepared
    Then messages from the most recent 2 turns remain in context
    And messages from older turns are dropped

  @phase-1-hardening
  Scenario: Removing obsolete spill guidance refreshes the next provider context
    Given max_context_tokens is set to 1000
    And recent-turn pinning is set to 0 turns
    And no spill entries exist
    And a pinned manifest message in the conversation
    When the next provider context is prepared
    Then no manifest [message] exists in context

  @phase-1-hardening
  Scenario: Provider-reported context usage remains stable when local estimates change
    Given provider truth reports 1000 context tokens at local estimate 100
    When the local context estimate changes to 80 tokens
    Then the user-facing context gauge reports 980 tokens

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

  @issue-1118
  Scenario: Static spill guidance is injected after first spill
    Given no spill entries exist
    When the agent executes a bash tool on turn 1
    Then a pinned manifest [message] appears in context
    And the manifest contains "full session-memory index"
    And the manifest does not contain "turn1:bash:0"

  @issue-1118
  Scenario: Spill guidance stays static as the index grows
    Given 25 spilled tool results
    When the spill manifest is updated
    Then the manifest contains "full session-memory index"
    And the manifest does not contain "25 spilled entries"
    And the manifest does not contain "turn25:bash:0"

  Scenario: Spill manifest survives sliding window
    Given max_context_tokens is set to 500
    And 20 spilled tool results
    When the sliding window drops messages to fit budget
    Then the manifest [message] remains in context
    And the manifest is pinned

  Scenario: Spill guidance is maintained in-place
    When the agent executes tools on turns 1 through 5
    Then only one manifest [message] exists in context
    And the manifest contains "full session-memory index"
    And the manifest does not contain "turn5:bash:0"

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

  @issue-1118
  Scenario: Tool and message spill IDs stay out of static prompt guidance
    Given max_context_tokens is set to 200
    And recent-turn pinning is set to 2 turns
    And a spilled tool result with id "turn1:bash:0"
    And an old assistant message of 500 tokens on turn 1
    When the spilling sliding window drops messages to fit budget
    Then the manifest does not contain "turn1:bash:0"
    And the manifest does not contain "turn1:msg:assistant"

  @issue-1118
  Scenario: Message spill on a turn with no tool calls creates static guidance
    Given max_context_tokens is set to 200
    And recent-turn pinning is set to 2 turns
    And an old assistant message of 500 tokens on turn 1
    When the agent completes a prompt with no tool calls
    Then a pinned manifest [message] appears in context
    And the manifest contains "full session-memory index"
    And the manifest does not contain "turn1:msg:assistant"

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

  # --- #1046: spill conversation messages at creation ---

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

  # The three AC3 exemption scenarios drive the emergency ladder under an
  # unmeetable budget. Its second rung drops any non-exempt message, so each
  # protected message survives only because of its exemption — deleting the
  # exemption fails the scenario (falsifiability, PR #1048 round-2 review).
  Scenario: The system prompt is never collapsed or dropped
    Given max_context_tokens is set to 5
    And recent-turn pinning is set to 0 turns
    And a system prompt in the conversation
    And 2 old conversation messages
    And an in-flight user prompt
    When the agent enforces the context ceiling
    Then the system prompt is not collapsed
    And at least 1 message is removed from the conversation

  Scenario: The spill manifest is never collapsed or dropped
    Given max_context_tokens is set to 5
    And recent-turn pinning is set to 0 turns
    And a pinned manifest message in the conversation
    And 2 old conversation messages
    And an in-flight user prompt
    When the agent enforces the context ceiling
    Then the manifest message is not collapsed
    And at least 1 message is removed from the conversation

  Scenario: The in-flight user prompt is never collapsed or dropped
    Given max_context_tokens is set to 5
    And recent-turn pinning is set to 0 turns
    And 2 old conversation messages
    And an in-flight user prompt already spilled at creation
    When the agent enforces the context ceiling
    Then the in-flight user prompt is not collapsed
    And at least 1 message is removed from the conversation

  Scenario: Budget pressure collapses messages to stubs before dropping anything
    Given max_context_tokens is set to 150
    And recent-turn pinning is set to 1 turns
    And 4 old conversation messages
    And an in-flight user prompt
    When the agent enforces the context ceiling
    Then at least 1 old message is reduced to a recall stub by the ceiling
    And no messages are removed from the conversation
    And total context is under 150 tokens

  # #2213: crossing the ceiling demotes down to the low-water mark (75% of
  # the budget, rounded up), not just under the ceiling, so the next turns
  # append without rewriting the cached prompt prefix.
  # (#2212: the stubs' ids and counts are digit runs, priced at the dense
  # rate, so the budget leaves the mark above what four stubs cost.)
  Scenario: Crossing the ceiling demotes down to the low-water mark
    Given max_context_tokens is set to 185
    And recent-turn pinning is set to 0 turns
    And 4 old conversation messages
    And an in-flight user prompt
    When the agent enforces the context ceiling
    Then at least 1 old message is reduced to a recall stub by the ceiling
    And no messages are removed from the conversation
    And total context is under 139 tokens

  Scenario: Budget pressure removes stubs entirely only when stubbing is not enough
    Given max_context_tokens is set to 5
    And recent-turn pinning is set to 0 turns
    And 4 old conversation messages
    And an in-flight user prompt
    When the agent enforces the context ceiling
    Then at least 1 message is removed from the conversation
    And no full un-collapsed conversation message was removed before stubbing

  # --- #1045: configurable pin_recent_turns ---

  Scenario: pin_recent_turns defaults to 2 and the watermark marks to 256000 and 70000
    Given a default agent configuration
    Then the configured pin_recent_turns is 2
    And the configured watermark marks cut at 256000 down to 70000

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

  # #2405: the window less the agent's 1024-token reply reserve.
  Scenario: Effective context budget derives from the model window less the reply when known
    Given a configured agent with max_context_tokens 200000
    And the active model has a known context window of 100000 tokens
    When the agent derives its effective context budget
    Then the effective context budget is 98976

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

  # --- #2212: the ceiling counts dense output at its tokenised size ---

  # Digits, hex and log columns tokenise at about 2 characters a token,
  # prose at about 4. The estimate prices each at its own rate, and once a
  # provider reports its prompt size, the ceiling decides on that figure.
  @issue-2212
  Scenario: Numeric tool output is counted at its tokenised size
    Given a configured agent with a mock LLM
    And a spilled conversation history of 6 prior turns of seq output
    And the pruning agent context budget is 4000 tokens
    And the LLM returns a plain text response "final answer"
    When the user sends "go" through the pruning agent
    Then some pre-run messages are archived or stubbed
    And a flat four-characters-per-token count of the pre-run history fits the budget

  @issue-2212
  Scenario: A provider-reported size over the budget prunes the next request of the run
    Given a configured agent with a mock LLM
    And a spilled conversation history of 6 prior turns of prose
    And the pruning agent context budget is 4000 tokens
    And the LLM returns a tool call for "bulk" reporting 8000 context tokens
    And the tool "bulk" returns "tool-output-payload"
    And the LLM returns a plain text response "final answer"
    When the user sends "go" through the pruning agent
    Then some pre-run messages are archived or stubbed

  @issue-2212
  Scenario: Without provider usage prose that fits the budget is kept
    Given a configured agent with a mock LLM
    And a spilled conversation history of 6 prior turns of prose
    And the pruning agent context budget is 4000 tokens
    And the LLM returns a tool call for "bulk" reporting no usage
    And the tool "bulk" returns "tool-output-payload"
    And the LLM returns a plain text response "final answer"
    When the user sends "go" through the pruning agent
    Then no pre-run message is archived or stubbed

  # --- Default max context tokens is 300,000 ---

  @done
  Scenario: Default max context tokens is 300000
    Given a default agent configuration
    Then the max_context_tokens is 300000

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
    And exactly one system [message] contains "Session memory is available via recall()"

  Scenario: Tool results remain uncollapsed after session round-trip
    When the agent executes a bash tool on turn 1
    And the agent completes turn 4
    Then the tool result from turn 1 is still in full context
    When the [session] is saved and reloaded from disk
    And the agent completes turn 5
    Then the tool result from turn 1 is still in full context

  # --- #1072: mid-run pruning vs positional watermarks ---

  @issue-1072
  Scenario: A run that shrinks history below its pre-run length reports exactly its appended messages
    Given a configured agent with a mock LLM
    And a spilled conversation history of 8 prior turns each exceeding the pruning budget
    And the pruning agent context budget is 700 tokens
    And the LLM returns a tool call for "bulk" with args:
      | key | value |
    And the tool "bulk" returns "tool-output-payload"
    And the LLM returns a plain text response "final answer"
    When the user sends "go" through the pruning agent
    Then fewer pre-run messages survive than were present before the run
    And the run's appended messages are exactly the assistant tool call, the tool result and the final reply "final answer"
    And the agent result marks the durable prefix dirty

  # #2414: the emergency ladder stubs in place only when no cut fits: the
  # brief, which every cut keeps, is alone over the budget here.
  @issue-1072
  Scenario: In-place stub demotion alone marks the durable prefix dirty
    Given a configured agent with a mock LLM
    And a spilled brief exceeding the pruning budget
    And a spilled conversation history of 2 further small prior turns
    And the pruning agent context budget is 300 tokens
    And the LLM returns a plain text response "ok"
    When the user sends "hi" through the pruning agent
    Then the oversized pre-run messages are collapsed to recall stubs in place
    And no pre-run message is removed from the conversation
    And the agent result marks the durable prefix dirty

  @issue-1072
  Scenario: Malformed-request feedback appears in the run's appended messages
    Given a configured agent with a mock LLM
    And the LLM returns a tool call for "echo" with args:
      | key | value |
    And the tool "echo" returns "echoed"
    And the provider rejects the next request as malformed
    And the LLM returns a plain text response "recovered"
    When the user sends "go" through the pruning agent
    Then the run's appended messages include the malformed-request feedback

  # --- #2342: a swarm member's ceiling ---

  @issue-2342
  Scenario: A swarm member prunes its context at its swarm ceiling
    Given a configured agent with a mock LLM
    And a spilled conversation history of 6 prior turns each exceeding the pruning budget
    And the member's swarm ceiling is 2000 tokens
    And the member reads the board summary 1 times, then replies "done"
    When the user sends "go" through the swarm member agent
    Then some pre-run messages are archived or stubbed
    And the member's context is cut at its swarm ceiling of 2000 tokens


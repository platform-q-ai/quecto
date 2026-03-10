@wip
Feature: Context pruning via sliding window (no tool-result collapse)

  Tool outputs remain in full context until dropped by the sliding window
  when the conversation exceeds the token budget. The old 3-turn collapse
  behaviour is disabled by default — tool results age naturally alongside
  all other messages. Users can re-enable collapse by setting
  context_collapse_after_turns to a lower value. Spill-to-disk still
  occurs at creation time so recall() can retrieve outputs that have been
  dropped by the sliding window.

  Background:
    Given a configured agent with context pruning enabled

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
    Then the system message remains in full context

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
    Then a pinned manifest message appears in context
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
    Then the manifest message remains in context
    And the manifest is pinned

  Scenario: Spill manifest is updated in-place
    When the agent executes tools on turns 1 through 5
    Then only one manifest message exists in context
    And it reflects all 5 spill entries

  Scenario: No manifest when no spill entries exist
    When the agent processes 3 turns with no tool calls
    Then no manifest message exists in context

  @done
  Scenario: Spill store caches index in memory after append
    When 3 spill entries are appended to the store
    Then list_entries returns 3 entries without re-reading disk

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
    Then the system message remains in full context
    And non-system messages are dropped to fit

  Scenario: First user message is pinned
    Given max_context_tokens is set to 500
    When the agent accumulates 800 tokens across 5 user messages
    Then the first user message remains in context
    And later user messages may be dropped

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

  # --- Default max context tokens is 190,000 ---

  Scenario: Default max context tokens is 190000
    Given a default agent configuration
    Then the max_context_tokens is 190000

  # --- Session persistence ---

  Scenario: Pruning metadata survives session save and load
    When the agent executes a bash tool on turn 1
    When the session is saved and reloaded from disk
    Then the tool result from turn 1 still has turn 1
    And the tool result from turn 1 still has tool_name "bash"
    And the tool result from turn 1 still has spill_id "turn1:bash:0"

  Scenario: Manifest is not duplicated after session save and reload
    When the agent executes tools on turns 1 through 5
    Then only one manifest message exists in context
    When the session is saved and reloaded from disk
    And the spill manifest is updated
    Then only one manifest message exists in context
    And exactly one system message contains "spilled entries via recall()"

  Scenario: Tool results remain uncollapsed after session round-trip
    When the agent executes a bash tool on turn 1
    And the agent completes turn 4
    Then the tool result from turn 1 is still in full context
    When the session is saved and reloaded from disk
    And the agent completes turn 5
    Then the tool result from turn 1 is still in full context

Feature: End-of-turn events reference messages instead of re-carrying full content (ADR-0008 part 2)
  As a UDS peer (TUI client, sub-agent parent monitor, extension process)
  I want end-of-turn events to identify the turn's messages by stable references
  So that event size stays bounded regardless of how much content the turn produced,
  clients that already hold the stream never re-fetch, and partial observers can
  still converge to the full conversation on demand

  # Scenarios pin protocol behaviour, not byte layout (ADR-0010). Framing is
  # unchanged (#1059); history paging is out of scope (part 3 / #1050). Shrink
  # machinery may remain until part 4 — these events simply must not need it.
  #
  # Recovery design (mandatory, from abandoned PR #1075):
  #   - Common streamed case: non-empty refs, zero fetches, zero duplicates.
  #   - Miss/partial: fetch only missing content; reconcile/replace at the turn.
  #   - Request-id gating; reconstruct all roles (assistant, tool-call, tool result).
  #   - Busy-path on-demand lookup; non-empty-ref tests in both directions.
  #   - Large-turn size assertion uses real non-empty content.

  # ─── Bounded end-of-turn size (AC1, AC5) ───────────────────────────────────

  @issue-1060 @adr-0008-part2
  @done
  Scenario: A large real assistant turn keeps end-of-turn events well under the frame size limit
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response larger than the event line cap
    When I start the UDS agent with no [session]
    And I send prompt "generate a huge answer"
    And I close the UDS connection
    Then the turn_end event should stay well under the frame size limit
    And the agent_end event should stay well under the frame size limit
    And the turn_end event should stay under the hard event line cap
    And the agent_end event should stay under the hard event line cap

  @issue-1060 @adr-0008-part2
  @done
  Scenario: End-of-turn events do not re-carry full assistant content for a large turn
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response larger than the event line cap
    When I start the UDS agent with no [session]
    And I send prompt "generate a huge answer"
    And I close the UDS connection
    Then the turn_end event should not re-carry the full assistant content
    And the agent_end event should not re-carry full message content
    And the turn_end event should identify the turn messages by non-empty message refs
    And the agent_end event should identify the run messages by non-empty message refs

  @issue-1060 @adr-0008-part2
  @done
  Scenario: A large tool-using turn keeps end-of-turn events well under the frame size limit
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a tool call with arguments larger than the event line cap then a text response
    When I start the UDS agent with no [session]
    And I send prompt "run a bulk tool"
    And I send command "get_messages" with id "gm-tool-roles-large"
    And I close the UDS connection
    Then the agent_end event should stay well under the frame size limit
    And the agent_end event should not re-carry full message content
    And the agent_end event should identify the run messages by non-empty message refs
    # Role coverage of the refs is asserted in the small tool-call scenario
    # below; here the oversized tool-call message exceeds the frame budget, so
    # get_messages/get_message cannot return it whole (#1062) and the refs
    # cannot be resolved to roles over the wire.

  # ─── Stable message references (AC6) ───────────────────────────────────────

  @issue-1060 @adr-0008-part2
  @done
  Scenario: A completed text turn emits non-empty message refs on end-of-turn events
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Hello world"
    When I start the UDS agent with no [session]
    And I send prompt "greet me"
    And I close the UDS connection
    Then the turn_end event should identify the turn messages by non-empty message refs
    And the agent_end event should identify the run messages by non-empty message refs

  @issue-1060 @adr-0008-part2
  @done
  Scenario: A tool-using turn emits refs for assistant tool-call and tool-result messages
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a tool call then a text response "done"
    When I start the UDS agent with no [session]
    And I send prompt "run a tool"
    And I send command "get_messages" with id "gm-tool-roles"
    And I close the UDS connection
    Then the agent_end event should identify the run messages by non-empty message refs
    And the agent_end message refs should cover assistant tool-call and tool-result roles
    And the agent_end event should not re-carry full message content

  @issue-1060 @adr-0008-part2
  @done
  Scenario: get_messages exposes the same stable message identifiers as end-of-turn refs
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "stable ids"
    When I start the UDS agent with no [session]
    And I send prompt "remember this"
    And I send command "get_messages" with id "gm-ids"
    And I close the UDS connection
    Then the agent output should contain a response command "get_messages" with success true
    And the get_messages response messages should each carry a non-empty stable message identifier
    And the get_messages message identifiers should match the end-of-turn message refs

  @issue-1060 @adr-0008-part2 @multi-client @persist
  @done
  Scenario: Busy-connect snapshot message identifiers match end-of-turn refs
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM will delay its response by 3 seconds
    And the mock LLM returns a text response "busy snapshot body"
    When I start the multi-client UDS agent with persist
    And client 1 connects
    And client 1 sends prompt "remember this"
    And I wait for the first turn to complete
    And client 1 sends prompt "slow second task"
    And client 2 connects while the agent is busy
    And I close all UDS clients
    Then client 2 should have received a get_messages snapshot with non-empty stable message identifiers
    And those snapshot message identifiers should match the completed turn's end-of-turn message refs

  # ─── On-demand lookup (AC4) ────────────────────────────────────────────────

  @issue-1060 @adr-0008-part2 @persist
  @done
  Scenario: A consumer can fetch full message content by stable message ref
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "fetch me by ref"
    And a completed turn whose end-of-turn events identify messages by non-empty refs
    When I request each end-of-turn message by its stable ref via get_message
    Then every get_message response should succeed with the full message content for its ref
    And every get_message response should round-trip the requested message identifier

  @issue-1060 @adr-0008-part2 @multi-client @persist
  @done
  Scenario: get_message returns full content for a prior message while a later turn is in flight
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM will delay its response by 3 seconds
    And the mock LLM returns a text response "prior body"
    When I start the multi-client UDS agent with persist
    And client 1 connects
    And client 1 sends prompt "first completed turn"
    And I wait for the first turn to complete
    And I record a non-empty message ref from the completed turn
    And client 1 sends prompt "slow second task"
    And client 2 connects while the agent is busy
    And client 2 requests get_message for the recorded ref
    And I close all UDS clients
    Then client 2 should have received a successful get_message response for the requested ref
    And the get_message response should carry full content for the requested ref

  # Acceptance checklist for #1094:
  #   - Messages larger than the protocol frame cap round-trip completely via get_message.
  #   - Both idle and busy UDS get_message paths deliver the content in bounded responses.
  #   - Every response frame stays within the protocol cap and clients remain connected.
  #   - TUI and API/WebSocket readers can request all ranges and present the reassembled body.

  @done @issue-1094 @adr-0008-part2 @persist
  Scenario: An oversized prior message is recoverable after the agent is idle
    Given an idle persisted agent session containing an oversized prior assistant message
    When I request the oversized message by its stable reference
    Then every oversized-message response fragment should stay within the protocol frame cap
    And the response fragments should reassemble the full message content
    And the UDS client connection should remain open

  @done @issue-1094 @adr-0008-part2 @multi-client @persist
  Scenario: An oversized prior message is recoverable while a later turn is in flight
    Given a persisted agent session contains an oversized prior assistant message
    And the agent is processing a later turn
    When another client requests the oversized message by its stable reference
    Then every oversized-message response fragment received by that client should stay within the protocol frame cap
    And that client should reassemble the full oversized message content
    And that client should remain connected while the agent is busy

  @issue-1093 @adr-0008-part2 @persist
  @done
  Scenario: get_message recalls full content for a collapsed message after the agent is idle
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a completed turn with a collapsed message whose full content was spilled
    When I request the collapsed message by its stable ref via get_message
    Then the get_message response should carry the full spilled content for the requested ref
    And the get_message response should not carry a recall stub for the requested ref

  @issue-1093 @adr-0008-part2 @multi-client @persist
  @done
  Scenario: get_message recalls full content for a collapsed message while a later turn is in flight
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM will delay its response by 8 seconds
    And a completed turn with a collapsed message whose full content was spilled
    When client 1 starts a later turn
    And client 2 requests the collapsed message by its stable ref while the agent is busy
    Then client 2 should have received a successful get_message response for the requested ref
    And the get_message response should carry the full spilled content for the requested ref
    And the get_message response should not carry a recall stub for the requested ref

  @issue-1093 @adr-0008-part2 @persist
  @done
  Scenario: get_message still returns a collapsed stub when spilled content is unavailable
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a completed turn with a collapsed message whose spilled content is unavailable
    When I request the collapsed message by its stable ref via get_message
    Then the get_message response should succeed for the requested ref
    And the get_message response should carry a recall stub for the requested ref

  # ─── Footer metadata preserved (AC7) ───────────────────────────────────────

  @issue-1060 @adr-0008-part2
  @done
  Scenario: turn_end still carries context and usage footer metadata
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "footer meta"
    When I start the UDS agent with no [session]
    And I send prompt "hi"
    And I close the UDS connection
    Then the turn_end event should include numeric contextTokens and maxContextTokens
    And the turn_end event should include numeric usage totals when usage is present
    And the turn_end event should not re-carry the full assistant content

  # ─── Sub-agent re-stamped path (AC3) ───────────────────────────────────────

  @issue-1060 @adr-0008-part2
  @done
  Scenario: Parent stream identifies child turn messages by refs without full content
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "child turn body"
    And a multi-client UDS agent with client 1 connected
    When a sub-agent completes a turn visible on the parent event stream
    Then the parent stream's subagent_messages_appended event should identify messages by non-empty message refs
    And the parent stream's subagent_messages_appended event should not re-carry full message content
    And the parent stream's subagent_messages_appended event should stay well under the frame size limit

  # ─── Streaming common path (AC2 wire side) ─────────────────────────────────

  @issue-1060 @adr-0008-part2 @token-streaming
  @done
  Scenario: A streamed turn completes with tokens and non-empty refs without re-carrying content
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And UDS streaming is enabled
    And the mock LLM returns a streaming response with tokens "Hello" " world"
    When I start the UDS agent with no [session]
    And I send prompt "greet me"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a token event with "Hello"
    And the agent output should contain a token event with " world"
    And the turn_end event should identify the turn messages by non-empty message refs
    And the turn_end event should not re-carry the full assistant content

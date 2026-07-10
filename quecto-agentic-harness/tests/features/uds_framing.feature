Feature: Length-prefixed UDS framing with version negotiation (ADR-0008 part 1)
  As a quecto socket peer (TUI client, harness reader, sub-agent parent monitor, extension process)
  I want every frame to declare its size before its payload and peers to negotiate the framing in use
  So that oversized frames are rejected deliberately instead of truncated mid-buffer, and
  mixed-version peers fail loudly instead of misparsing each other or hanging

  # Scenarios pin protocol behaviour, not byte layout (ADR-0010): framing is owned
  # by the shared test-transport helpers, which sit on quecto-line-io's production
  # frame writer/reader.
  #
  # The other three consumers' migrations are pinned where those consumers live:
  # quecto-tui's client tests (frame-aware writer/reader), quecto-api's gateway
  # tests, and the harness sub-agent monitor unit tests — this feature exercises
  # the harness UDS agent end of the shared protocol.
  #
  # The NDJSON deprecation window itself is documented in ADR-0008 and pinned by
  # the repo-docs conformance test (`tests/repo_docs.rs`), not by a scenario: a
  # docs-content check is not observable system behaviour.

  # ─── Version announcement ───────────────────────────────────────────────────

  @done @issue-1059
  Scenario: The socket announcement carries a protocol version token a client can act on before speaking
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "hello"
    And the UDS agent is running with no [session]
    And a length-prefixed framing client that disconnects after sending
    When I read the socket announcement
    Then the socket announcement should include a protocol version token

  # ─── Framed operation ───────────────────────────────────────────────────────

  @done @issue-1059
  Scenario: A framed client completes a prompt round-trip over length-prefixed frames
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "framed hello"
    And the UDS agent is running with no [session]
    And a length-prefixed framing client that disconnects after sending
    When I send prompt "hi" as a length-prefixed frame
    Then the agent output should contain an event of type "turn_end"
    And the agent output should contain an event of type "agent_end"

  @done @issue-1059
  Scenario: An over-limit frame does not break the connection's subsequent frames
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "still alive"
    And the UDS agent is running with no [session]
    And a length-prefixed framing client that disconnects after sending
    And the client has sent a frame declaring a size above the frame size limit
    When I send prompt "are you still there?" as a length-prefixed frame
    Then the agent should log a protocol error for the over-limit frame
    And the agent output should contain an event of type "turn_end"
    And the agent output should contain an event of type "agent_end"

  # ─── Version negotiation / deprecation window ───────────────────────────────

  @done @issue-1059
  Scenario: A legacy newline-framed client interoperates during the deprecation window
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "legacy hello"
    And the UDS agent is running with no [session]
    And a legacy newline-framing client that disconnects after sending
    When I send prompt "hi" as a legacy newline-framed line
    Then the agent output should contain an event of type "turn_end"
    And the agent output should contain an event of type "agent_end"

  @done @issue-1059
  Scenario: A peer speaking neither framing fails with an explicit version-mismatch error, never a hang
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "unused"
    And the UDS agent is running with no [session]
    And a raw client that disconnects after sending
    When the client sends bytes that are neither a frame nor legacy JSON
    Then the UDS agent exits with code 0
    And the agent should log an explicit protocol version-mismatch error

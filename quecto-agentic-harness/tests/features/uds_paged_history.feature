Feature: Paged history on connect and resume (ADR-0008 part 3)
  As a UDS client attached to a long-running quecto session
  I want conversation history to arrive as bounded, navigable history slices
  So that newest messages appear promptly and older history remains explicitly reachable without silent loss

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Attaching to a long session renders the newest bounded history slice
    Given a persisted UDS session with enough history to require paging
    When a client attaches to the session
    Then the client should receive only a bounded newest slice of history
    And the client should know older history can be requested
    And no reachable history should be reported as trimmed

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: A session that exactly fits the first history slice has no older history
    Given a persisted UDS session whose history exactly fits in the first slice
    When a client attaches to the session
    Then the client should receive every session message
    And the client should know the beginning of history has been reached

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: A session just beyond the first history slice keeps the oldest message reachable
    Given a persisted UDS session whose history continues just beyond the first slice
    When a client attaches to the session
    Then the client should receive the newest bounded history slice
    And the client should know older history can be requested
    And the omitted oldest message should be reachable by paging

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Paging backward exposes the full history without gaps
    Given a persisted UDS session with enough history to require multiple older slices
    When a client pages backward to the beginning of the session
    Then every history slice should join to the next slice without an interior gap
    And the collected history should contain each session message exactly once
    And the collected history should include the first session message
    And the collected history should include the newest session message
    And the client should know the beginning of history has been reached

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Near-limit live pages remain bounded and gap-free
    Given a persisted UDS session whose newest history slice is near the wire limit
    When a client pages backward to the beginning of the session
    Then every history slice should join to the next slice without an interior gap
    And the collected history should contain each session message exactly once
    And the client should know the beginning of history has been reached

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Older clients still receive a newest history slice
    Given a persisted UDS session with enough history to require paging
    When an older client requests conversation history without a paging cursor
    Then the client should receive a usable newest history slice
    And the client should know older history can still be requested

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Stubbed history appears in place on attach
    Given a persisted UDS session containing a stubbed long message
    When a client attaches to the session
    Then the history should show the stubbed message in place
    And the stubbed message should include a stable message reference

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Stubbed history can be recalled by reference
    Given a client has received history containing a stubbed long message
    When the client requests the full message by its stable message reference
    Then the client should receive the full message content

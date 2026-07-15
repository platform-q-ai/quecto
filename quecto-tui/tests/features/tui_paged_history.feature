Feature: TUI history backfill through paged UDS history
  As an operator attaching the TUI to a long-running quecto session
  I want scroll-back to reveal complete prior conversation history
  So that the interface never appears to have forgotten messages that remain in the session

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Main chat attach backfills the newest session history
    Given a running agent session with prior conversation history
    When the TUI attaches to the session socket
    Then the main chat should show the newest prior messages
    And the chat should show whether older history is available

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Scrolling back reveals older main chat history continuously
    Given the TUI is attached to a session with enough history to require backfill
    When the operator scrolls back until the beginning of history is reached
    Then the chat should reveal the first session message
    And the revealed history should contain each session message exactly once
    And the revealed history should contain no interior gap

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: One-slice main chat history has no older backfill
    Given the TUI is attached to a session whose history exactly fits in the initial backfill
    When the operator scrolls back to the top of history
    Then the chat should not request older history
    And the chat should continue to show every session message

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Just-over-one-slice main chat history keeps the oldest message reachable
    Given the TUI is attached to a session with one older message beyond the initial backfill
    When the operator scrolls back to the top of the newest history
    Then the chat should request older history
    And the oldest session message should become visible

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Sub-agent scroll-back uses the same paged history behaviour
    Given the TUI is viewing a sub-agent with enough history to require backfill
    When the operator scrolls back until the beginning of that history is reached
    Then the sub-agent chat should reveal the first sub-agent message
    And the revealed sub-agent history should contain each sub-agent message exactly once
    And the revealed sub-agent history should contain no interior gap

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Resume keeps older messages reachable through paging
    Given a resumable session with enough history to require backfill
    When the operator resumes the session in the TUI
    Then the main chat should show the newest resumed messages
    And older resumed messages should be reachable by scrolling back

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: A failed master page enqueue can be retried
    Given the TUI master command channel disconnects with older history available
    When the operator retries scroll back after the page enqueue fails
    Then both older history attempts should reach command failure handling

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: A failed master stub enqueue can be retried
    Given the TUI master command channel disconnects with a visible history stub
    When the operator retries stub recall after the enqueue fails
    Then both stub recall attempts should reach command failure handling

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Starting a new conversation invalidates an in-flight older page
    Given the TUI has an older history page in flight
    When the operator starts a new conversation before that page arrives
    Then the late older page should not appear in the new conversation
    And scrolling the new conversation should not request the old history cursor

  @done @issue-1061 @adr-0008-part3 @persist
  Scenario: Expanding stubbed history replaces the stub with full content
    Given the TUI is attached to a session containing a stubbed long message
    When the operator requests the full content for that history message
    Then the recalled content should replace the stubbed history entry

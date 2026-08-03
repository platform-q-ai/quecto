@done @tui @issue-972
Feature: TUI remains stable during high-throughput streaming
  As a TUI user receiving a very fast assistant response
  I want the interface to remain visually stable and responsive
  So that streaming output does not corrupt my terminal view

  Scenario: Sustained fast streaming keeps a stable frame
    Given the TUI is receiving a sustained high-throughput assistant response
    When the response continues for an extended period
    Then the TUI presents a stable frame without stray cursor blocks

  Scenario: Bursty token output remains responsive
    Given the TUI is receiving a burst of assistant tokens
    When the user provides input during the burst
    Then the user input is reflected promptly while the response continues

  Scenario: Bursty token output avoids excessive repainting
    Given the TUI is receiving a burst of assistant tokens
    When the burst is presented to the user
    Then the streaming response remains visually smooth without distracting flicker

  Scenario: Streaming indicator stays within the chat frame
    Given an assistant response is streaming near the right edge of the chat frame
    When the TUI presents the streaming response near the chat frame edge
    Then the streaming indicator remains inside the chat frame

  Scenario: Streaming content has vertical breathing room
    Given an assistant response is streaming
    When the TUI presents the streaming response
    Then the streaming content is separated from the master idle area by one blank line
    And the streaming content is separated from the working area by one blank line

  Scenario: Terminal cursor stays hidden during streaming recovery
    Given the terminal cursor is hidden while an assistant response streams
    When the display recovers during the streaming response
    Then the real terminal cursor stays hidden

  Scenario: Intentional cursors remain available after streaming stabilisation
    Given an assistant response is streaming
    When the TUI presents the streaming response
    Then the editor cursor and assistant streaming indicator remain visible

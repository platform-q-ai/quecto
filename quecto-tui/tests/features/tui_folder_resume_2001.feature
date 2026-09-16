@tui @done @issue-2001
Feature: Folder-aware resume picker presentation (#2001)
  As an operator resuming a persisted conversation
  I want folder scope to be explicit in the session picker
  So that I do not silently resume a conversation in the wrong workspace

  @issue-2001-red
  Scenario: Bare resume exposes a visible Local and Global scope control
    Given a fresh TUI app harness
    When bare resume returns two persisted sessions
    Then the resume picker shows the scope control "Local | Global"

  @issue-2001-compat
  Scenario: Exact opaque-key resume remains globally compatible
    Given a fresh TUI app harness
    When I resume the exact opaque session key "cli:01J-FOLDER-OTHER"
    Then the exact opaque session key "cli:01J-FOLDER-OTHER" is sent for resume

  @issue-2001-compat
  Scenario: Ctrl+G still jumps a scrolled conversation to its latest message
    Given a long TUI conversation scrolled away from its latest message
    When I press Ctrl+G in the conversation
    Then the TUI returns to the latest conversation message

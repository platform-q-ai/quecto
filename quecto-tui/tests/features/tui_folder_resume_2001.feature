@tui @done @issue-2001
Feature: Operational folder-aware resume picker (#2001)
  As an operator resuming a persisted conversation
  I want the picker scope, query, results, and decisions to be real controls
  So that I cannot silently resume a conversation in the wrong workspace

  @issue-2001-red
  Scenario: Bare resume defaults to local discovery and exposes loose scope labels
    Given a fresh TUI app harness
    When bare resume returns two persisted sessions
    Then the bare resume request uses the approved local discovery default
    And the resume picker separately shows Local and Global scope labels

  @issue-2001-red
  Scenario: Enter and Space activate Global and present global results
    Given a fresh TUI app harness
    When I activate Global in the resume picker with Enter and Space
    Then both activations request scope "global" and present a global result

  @issue-2001-red
  Scenario: Tab and Shift Tab move modal focus visibly
    Given a fresh TUI app harness
    When I move resume picker focus forward and backward
    Then each resume picker focus move is visible and reversible

  @issue-2001-red
  Scenario: Resume query searches both title and opaque key
    Given a fresh TUI app harness
    When I enter a resume query that only matches an opaque session key
    Then only the session with the matching opaque key remains in the results

  @issue-2001-red
  Scenario: Enter and Space activate the focused result
    Given a fresh TUI app harness
    When I activate focused resume results with Enter and Space
    Then both focused results are requested for resume

  @issue-2001-red
  Scenario: The Global scope label is a mouse hit target
    Given a fresh TUI app harness
    When I click the Global resume scope label
    Then the TUI requests another session list and presents its global result

  @issue-2001-compat
  Scenario: Escape cancels without selecting a resume result
    Given a fresh TUI app harness
    When I cancel the open resume picker with Escape
    Then no session is requested for resume

  @issue-2001-red
  Scenario: Only the exact latest response updates results and stable-key selection
    Given a fresh TUI app harness
    When newer and older correlated Global results arrive out of order around my stable selection
    Then only the newest response is shown and the same stable session remains highlighted

  @issue-2001-red
  Scenario: A same-execution-directory selection resumes without a foreign-folder decision
    Given a fresh TUI app harness
    When I select a session whose path is the current execution directory
    Then the TUI requests that exact session and accepts status "resumed" without foreign-folder choices

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

  @issue-2001-red @decision-modal
  Scenario: A foreign-folder decision exposes operational keyboard controls
    Given a foreign-folder resume decision in the TUI
    When I activate each foreign decision choice with the keyboard
    Then Open original Fork here and Cancel each send their allowlisted action

  @issue-2001-red @decision-modal @mouse
  Scenario: A foreign-folder decision exposes operational mouse controls
    Given a foreign-folder resume decision in the TUI
    When I click each foreign decision choice
    Then Open original Fork here and Cancel each send their allowlisted action

  @issue-2001-red @decision-modal
  Scenario: Legacy and missing-folder decisions expose only their affirmative controls
    Given a legacy resume decision in the TUI
    Then the decision modal offers Associate Fork here and Cancel
    Given a missing-folder resume decision in the TUI
    Then the decision modal offers Locate Fork here and Cancel

  @issue-2001-red @decision-modal @outcomes
  Scenario: Decision cancellation preserves the conversation and Open enters the target
    Given a foreign-folder resume decision in the TUI
    When I cancel the decision with the keyboard
    Then the current conversation remains unchanged
    Given a foreign-folder resume decision in the TUI
    When I choose Open original and the target becomes ready
    Then the TUI enters the target conversation

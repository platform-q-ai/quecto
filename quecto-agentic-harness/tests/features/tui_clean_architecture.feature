@done @tui
Feature: TUI feature-oriented architecture and executable BDD enforcement
  The quecto-tui crate is a feature-oriented presentation adapter for the
  harness. Its current Clean Architecture layer checks are interim compatibility
  guardrails while production source migrates toward harness-facing capability
  modules, and the BDD suite executes TUI standards instead of only keeping them
  as pending documentation.

  @issue-1149
  Scenario: quecto-tui documents feature-oriented presentation boundaries
    Given the quecto-tui architecture documents are present
    Then the quecto-tui feature-oriented architecture document should list each target harness-facing capability module
    And the quecto-tui feature-oriented architecture document should document protocol and pure-policy boundaries
    And the old quecto-tui Clean Architecture target model should be superseded
    And the quecto-tui README should point to the feature-oriented architecture document

  Scenario: quecto-tui retains interim compatibility layers during migration
    Then the quecto-tui source tree should contain layer "domain"
    And the quecto-tui source tree should contain layer "protocol"
    And the quecto-tui source tree should contain layer "infrastructure"
    And the quecto-tui source tree should contain layer "interface"

  Scenario: quecto-tui domain layer remains free of runtime I/O
    Then the quecto-tui domain source should not contain runtime I/O patterns

  Scenario: quecto-tui layer dependencies point inward
    Then the quecto-tui domain source should not import outer layers
    And the quecto-tui infrastructure source should not import application or interface layers
    And the quecto-tui protocol source should not import feature or shell modules
    And the quecto-tui shell should own runtime adapters

  Scenario: quecto-tui production files live inside approved top-level modules
    Then every quecto-tui production Rust file should be under an approved top-level module

  Scenario: quecto-tui crate roots stay as thin composition entrypoints
    Then the quecto-tui library root should expose only approved top-level modules
    And the quecto-tui binary root should delegate to the shell module

  Scenario: quecto-tui architecture is enforced by the same architecture test target as quecto
    Then the architecture test target should enforce quecto-tui Clean Architecture layers
    And the architecture test target should enforce quecto-tui runtime I/O boundaries
    And the architecture test target should enforce quecto-tui root file placement

  @issue-1020
  Scenario: TUI standards are executable through BDD
    Then the BDD runners should not use wip as a default run inclusion gate
    And the TUI architecture feature should not contain pending scenarios

  @issue-741
  Scenario: TUI session payload parsing lives outside the App interface
    Then the TUI protocol layer should parse session stats payloads into typed values
    And the TUI protocol layer should validate resumed chat payloads into typed messages
    And the TUI App methods should delegate session payload parsing to the protocol layer

  @issue-739
  Scenario: TUI keeps current chat when resumed messages are malformed
    Then the TUI should validate resumed messages before replacing chat history

  @issue-740
  Scenario: TUI selector components share list navigation
    Then the TUI components layer should expose a shared ListNavigator
    And slash autocomplete, files autocomplete, model selector, and select list should use ListNavigator
    And ListNavigator should own wraparound and visible-window selection behavior

  Scenario: TUI scrollback remains stable while an assistant response streams
    Given a quecto-tui chat view is scrolled into history
    When streaming assistant content extends the conversation
    Then the quecto-tui chat viewport should keep showing the same historical lines

  Scenario: TUI scrollback stops at a full page instead of blanking while streaming
    Given a quecto-tui chat view is scrolled beyond the oldest full page
    When streaming assistant content extends the conversation
    Then the quecto-tui chat viewport should still show a full historical page

  @issue-757
  Scenario: TUI chat render is stable across repeated frames
    Given a quecto-tui chat view with conversation history
    When the chat is rendered twice without changes
    Then both quecto-tui chat renders should be identical

  Scenario: TUI slash autocomplete exposes session resume
    Then the quecto-tui slash autocomplete should include command "resume"

  Scenario: TUI rejects unknown slash commands locally
    Then quecto-tui should reject unknown slash commands before sending a prompt

  Scenario: TUI can list and resume persisted CLI sessions over UDS
    Then the UDS protocol should support listing sessions
    And the UDS protocol should support resuming a session

  Scenario: TUI resume selector is readable above chat history
    Then the quecto-tui resume selector should render with a themed box border

  Scenario: TUI does not render a separate workflow header bar
    Then quecto-tui should not render a separate workflow header bar

  Scenario: TUI shows a Quecto-style workflow widget above the editor
    Then the quecto-tui workflow widget should render as plain text matching the Quecto workflow
    And the quecto-tui workflow widget should show workflow hotkey hints with toggle state

  Scenario: TUI workflow widget uses only active toggle hotkeys
    Then quecto-tui should not expose the Ctrl+Shift+W workflow overlay

  Scenario: TUI drops the dead OverlayStack compositing machinery
    Then quecto-tui should not retain the dead OverlayStack overlay machinery
    And quecto-tui should not keep tests that pin the dead OverlayStack machinery alive
    And quecto-tui should keep the live splice_line overlay helpers

  Scenario: TUI drops the legacy workflow_bar render path
    Then quecto-tui should not retain the legacy workflow_bar render function
    And quecto-tui should not keep tests that pin the legacy workflow_bar render path alive
    And quecto-tui should keep the live workflow_bar render_widget path

  @issue-759
  Scenario: TUI compose_frame splices centered overlays through one helper
    Then the quecto-tui render compositing should expose a composite_centered helper
    And the quecto-tui resume, rewind, and model overlays should splice through composite_centered

  @issue-759
  Scenario: TUI footer stats update has a single owner
    Then the quecto-tui show_session_stats should delegate to update_footer_stats
    And the quecto-tui session-stats footer mapping should live in a single Footer owner

  @issue-759
  Scenario: TUI slash and file autocomplete share one suggestion list
    Then the quecto-tui components layer should expose a shared SuggestionList
    And SuggestionList should own suggestions_match and set_suggestions
    And slash autocomplete and files autocomplete should use SuggestionList

  @issue-759
  Scenario: TUI chat renderers share preview and header helpers
    Then the quecto-tui chat_render should expose push_preview and push_header helpers
    And the quecto-tui chat tool renderers should build previews and headers through the helpers

  @issue-759
  Scenario: TUI workflow widget has a single phase label map
    Then the quecto-tui workflow_bar should expose exactly one phase-to-label map
    And the quecto-tui workflow_bar should not keep the phase_label_for_widget forwarder

  @issue-759
  Scenario: TUI client command serialization lives in one place
    Then the quecto-tui client serialize-and-newline rule should appear once

  @issue-759
  Scenario: TUI markdown rendering extracts per-block flush handlers
    Then the quecto-tui markdown renderer should extract table and code-block flush handlers

  @issue-759
  Scenario: TUI built-in slash commands have one source of truth
    Then the quecto-tui builtin command set should be the single source of truth
    And quecto-tui show_help and command dispatch should derive from builtin_commands

  @issue-760
  Scenario: TUI footer shows a streaming indicator while a response streams
    Given a quecto-tui footer marked as streaming
    Then the quecto-tui footer should render a streaming indicator

  @issue-760
  Scenario: TUI footer hides the streaming indicator while idle
    Given a quecto-tui footer that is idle
    Then the quecto-tui footer should not render a streaming indicator

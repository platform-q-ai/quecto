@tui
Feature: TUI Foundation
  The quecto-tui binary is a standalone terminal UI client that communicates
  with a quecto agent process over a Unix domain socket. These scenarios verify
  the Phase 1 foundation: UDS client connectivity, event handling, key parsing,
  component rendering, and differential output.
  These scenarios are specifications for the quecto-tui crate; tested via
  unit tests in quecto-tui/src/. Tagged @tui to exclude from normal BDD runs.

  # ---------------------------------------------------------------------------
  # UDS Client
  # ---------------------------------------------------------------------------

  @tui @pending
  Scenario: TUI connects to a UDS agent socket
    Given a quecto agent is running in UDS mode
    When the TUI client connects to the agent socket
    Then the TUI client should be connected

  @tui @pending
  Scenario: TUI receives agent state after connecting
    Given a quecto agent is running in UDS mode
    And the TUI client connects to the agent socket
    When the TUI client sends a get_state command
    Then the TUI client should receive a response with command "get_state" and success true

  @tui @pending
  Scenario: TUI footer updates when the current git branch changes
    Given quecto-tui is running in a git repository on branch "main"
    When the repository switches to branch "feature/footer-branch"
    Then the footer should show branch "feature/footer-branch" without restarting the TUI

  @tui @pending
  Scenario: TUI sends a prompt and receives streaming tokens
    Given a quecto agent is running in UDS mode
    And the TUI client connects to the agent socket
    When the TUI client sends a prompt "Say hello"
    Then the TUI client should receive an agent_start event
    And the TUI client should receive one or more token events
    And the TUI client should receive an agent_end event

  @tui @pending
  Scenario: TUI sends abort while agent is running
    Given a quecto agent is running in UDS mode
    And the TUI client connects to the agent socket
    When the TUI client sends a prompt "Count to a million slowly"
    And the TUI client waits for agent_start
    And the TUI client sends an abort command
    Then the TUI client should receive a response with command "abort" and success true

  # ---------------------------------------------------------------------------
  # Key Parsing
  # ---------------------------------------------------------------------------

  @tui @pending
  Scenario Outline: Key parser recognises basic escape sequences
    When the key parser receives <raw_bytes>
    Then it should produce key <key_name>

    Examples:
      | raw_bytes      | key_name   |
      | "\x1b[A"       | Up         |
      | "\x1b[B"       | Down       |
      | "\x1b[C"       | Right      |
      | "\x1b[D"       | Left       |
      | "\r"           | Enter      |
      | "\x7f"         | Backspace  |
      | "\x1b"         | Escape     |
      | "\t"           | Tab        |
      | "\x1b[3~"      | Delete     |
      | "\x1b[H"       | Home       |
      | "\x1b[F"       | End        |

  @tui @pending
  Scenario: Key parser recognises Ctrl+C
    When the key parser receives "\x03"
    Then it should produce key Ctrl_C

  @tui @pending
  Scenario: Key parser recognises printable characters
    When the key parser receives "a"
    Then it should produce key Char_a

  # ---------------------------------------------------------------------------
  # Component Rendering
  # ---------------------------------------------------------------------------

  @tui @pending
  Scenario: Text component renders within width
    Given a text component with content "Hello, world!"
    When the component renders at width 80
    Then every rendered line should be at most 80 visible characters

  @tui @pending
  Scenario: Text component wraps long lines
    Given a text component with content "The quick brown fox jumps over the lazy dog and keeps running"
    When the component renders at width 30
    Then the rendered output should have more than 1 line
    And every rendered line should be at most 30 visible characters

  # ---------------------------------------------------------------------------
  # Differential Renderer
  # ---------------------------------------------------------------------------

  @tui @pending
  Scenario: Diff renderer outputs all lines on first render
    Given a diff renderer targeting a capture buffer
    When it renders lines "alpha" and "beta" at width 80
    Then the capture buffer should contain "alpha"
    And the capture buffer should contain "beta"

  @tui @pending
  Scenario: Diff renderer only rewrites changed lines
    Given a diff renderer targeting a capture buffer
    And it has previously rendered lines "line1" and "line2"
    When it renders lines "line1" and "CHANGED" at width 80
    Then the capture buffer should contain "CHANGED"
    And the capture buffer should not re-emit "line1"

  @tui @pending
  Scenario: App render path uses differential output
    Given the TUI has composed and rendered a frame
    When only one visible line changes
    Then the terminal writer should rewrite only the changed line
    And unchanged chat history lines should not be emitted again

  @tui @pending
  Scenario: Chat markdown rendering is cached per entry
    Given chat history contains an unchanged assistant markdown message
    When the chat component renders multiple frames at the same width
    Then the assistant markdown entry should be reused from cache
    And markdown parsing should only run again after the entry text or width changes

  @tui @pending
  Scenario: Terminal control sanitizing is shared across rendered components
    Given agent-sourced text contains ANSI CSI, OSC, and control characters
    When markdown, tool output, subagent rows, and model names render that text
    Then each component should remove terminal control sequences before display
    And normal printable Unicode text should remain visible

  @tui @pending
  Scenario: Command send failures are observable
    Given the TUI command channel is disconnected
    When the TUI tries to send a command to the agent
    Then the send failure should be reported instead of silently ignored

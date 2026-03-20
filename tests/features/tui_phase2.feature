@tui @pending
Feature: TUI Phase 2 — Event Loop, Editor, and Chat Display
  The quecto-tui interactive mode: raw terminal, async event loop,
  multi-line editor, streaming chat display, spinner, and footer.

  # ---------------------------------------------------------------------------
  # Editor
  # ---------------------------------------------------------------------------

  Scenario: Editor inserts printable characters
    Given an editor component
    When the user types "hello"
    Then the editor text should be "hello"

  Scenario: Editor handles backspace
    Given an editor component with text "hello"
    When the user presses Backspace
    Then the editor text should be "hell"

  Scenario: Editor handles cursor left and right
    Given an editor component with text "abcd"
    When the user presses Left twice
    And the user types "X"
    Then the editor text should be "abXcd"

  Scenario: Editor handles Home and End
    Given an editor component with text "hello"
    When the user presses Home
    And the user types "X"
    Then the editor text should be "Xhello"

  Scenario: Editor handles Ctrl+U to clear line before cursor
    Given an editor component with text "hello world"
    And the cursor is at position 5
    When the user presses Ctrl+U
    Then the editor text should be " world"

  Scenario: Editor handles Ctrl+K to clear line after cursor
    Given an editor component with text "hello world"
    And the cursor is at position 5
    When the user presses Ctrl+K
    Then the editor text should be "hello"

  Scenario: Editor supports multi-line input
    Given an editor component
    When the user types "line1"
    And the user presses Enter
    And the user types "line2"
    Then the editor text should contain "line1" and "line2"

  Scenario: Editor renders within width with border
    Given an editor component with text "hello"
    When the editor renders at width 40
    Then the rendered output should contain a top border
    And the rendered output should contain "hello"
    And every rendered line should be at most 40 visible characters

  Scenario: Editor input history navigates with Up/Down
    Given an editor component
    When the user submits "first message"
    And the user submits "second message"
    And the user presses Up
    Then the editor text should be "second message"
    When the user presses Up
    Then the editor text should be "first message"
    When the user presses Down
    Then the editor text should be "second message"

  Scenario: Pasting CRLF text preserves lines without stray carriage returns
    Given an editor component
    When the user pastes "first line\r\nsecond line\r\nthird line"
    Then the editor text should be "first line\nsecond line\nthird line"

  # ---------------------------------------------------------------------------
  # Chat Display
  # ---------------------------------------------------------------------------

  Scenario: Chat displays user message
    Given a chat component
    When a user message "Hello agent" is added
    Then the rendered output should contain "Hello agent"

  Scenario: Chat displays streaming tokens
    Given a chat component
    When streaming tokens "Hello" " world" are received
    Then the rendered output should contain "Hello world"

  Scenario: Chat displays tool execution
    Given a chat component
    When a tool execution starts for "bash" with args "ls -la"
    Then the rendered output should contain "bash"
    When the tool execution ends with result "file1.txt"
    Then the rendered output should contain "file1.txt"

  # ---------------------------------------------------------------------------
  # Spinner
  # ---------------------------------------------------------------------------

  Scenario: Spinner shows working message during agent processing
    Given a spinner component with message "Working..."
    When the spinner renders at width 40
    Then the rendered output should contain "Working..."

  Scenario: Spinner cycles through animation frames
    Given a spinner component with message "Thinking"
    When the spinner ticks 3 times
    Then the spinner frame should have advanced

  # ---------------------------------------------------------------------------
  # Footer
  # ---------------------------------------------------------------------------

  Scenario: Footer shows model name
    Given a footer component with model "claude-sonnet-4-20250514"
    When the footer renders at width 80
    Then the rendered output should contain "claude-sonnet-4-20250514"

  Scenario: Footer shows git branch
    Given a footer component with git branch "main"
    When the footer renders at width 80
    Then the rendered output should contain "main"

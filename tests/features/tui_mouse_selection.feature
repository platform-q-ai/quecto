Feature: Mouse text selection and clipboard copy (#528)
  As a TUI user
  I want to click and drag to select text and copy it to the clipboard
  So that I can easily copy output from the interface

  # TUI mouse selection is tested via unit tests in quecto-tui/src/interface/keys.rs
  # (mouse event parsing) and quecto-tui/src/interface/app.rs (base64, ANSI stripping).
  # These BDD scenarios verify the SGR mouse protocol parsing.

  @wip
  Scenario: SGR mouse press parsed as MousePress event
    Given an SGR mouse sequence for button 0 press at col 10 row 5
    When the sequence is parsed
    Then the result should be a MousePress at col 9 row 4

  @wip
  Scenario: SGR mouse drag parsed as MouseDrag event
    Given an SGR mouse sequence for button 32 press at col 15 row 7
    When the sequence is parsed
    Then the result should be a MouseDrag at col 14 row 6

  @wip
  Scenario: SGR mouse release parsed as MouseRelease event
    Given an SGR mouse release sequence at col 20 row 10
    When the sequence is parsed
    Then the result should be a MouseRelease at col 19 row 9

  @wip
  Scenario: OSC 52 clipboard write is base64 encoded
    Given the text "hello world" to copy
    When it is base64 encoded for OSC 52
    Then the encoded value should be "aGVsbG8gd29ybGQ="

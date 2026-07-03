@done
Feature: Harness responses remain bounded and backwards compatible
  The harness presents previews, usage totals, and saved configuration data
  consistently so users can rely on bounded output and preserved settings.

  Scenario: A long multibyte value is previewed without splitting a character
    When a 500-character multibyte string is previewed to 100 characters
    Then the preview shows 100 characters ending in an ellipsis
    And the preview does not contain a broken character

  Scenario: OpenAI token usage is recorded from a provider usage report
    When an OpenAI response reports 12 prompt, 7 completion and 19 total tokens
    Then the recorded usage shows 12 prompt, 7 completion and 19 context tokens

  Scenario: Codex token usage is recorded from a provider usage report
    When a Codex response reports 100 input, 40 output and 30 cached tokens
    Then the recorded usage shows 100 prompt, 40 completion and 30 cached tokens

  Scenario: A config written by an older release still loads
    When a provider config written by an older release is loaded
    Then the provider config loads successfully
    And the saved credentials and endpoint are preserved

  Scenario: Offset reads of large files return bounded pages
    Given a large text file is available to the harness
    When the text file is read from a later line with a small page size
    Then only the requested page is shown
    And the next page guidance is shown

  Scenario: Command output remains bounded for long logs
    Given a command result contains many log lines
    When the command result is prepared for display
    Then the latest log lines are shown
    And the full output guidance is shown

  Scenario: Fetched HTML hides configured non-content regions
    Given fetched HTML contains configured non-content regions
    When the fetched HTML is converted to readable text
    Then the article text remains visible
    And the configured non-content regions are hidden

  Scenario: Plain text edits preserve the requested change
    Given a plain text file without a byte-order mark is available
    When the file is edited with an exact replacement
    Then the file contains the requested replacement
    And the edit confirmation shows the changed line

@done
Feature: Harness tool hot paths stay bounded (issue #991)
  Tool responses for large inputs remain bounded and correct: offset reads
  return only the requested page, long command output keeps the latest lines,
  fetched HTML hides non-content regions, and plain-text edits are preserved
  exactly.

  Scenario: Offset reads of large files return bounded pages
    Given a large text file is available to the harness
    When the text file is read from a later line with a small page size
    Then only the requested page is shown
    And the next page guidance is shown

  Scenario: Command output remains bounded for long logs
    Given a workspace for running commands
    When a command producing more log lines than the display limit is executed
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

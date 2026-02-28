@done
Feature: Output truncation module
  As a tool implementer
  I want a shared truncation module with head and tail truncation
  So that all tools consistently limit output size

  # --- truncate_head ---

  Scenario: Head-truncate empty string returns empty result
    Given an empty string to truncate
    When I head-truncate with max 2000 lines and 50KB bytes
    Then the truncation result should be empty
    And the result should not be truncated

  Scenario: Head-truncate short content is returned unchanged
    Given a string with 5 lines totalling 50 bytes
    When I head-truncate with max 2000 lines and 50KB bytes
    Then all 5 lines should be returned
    And the result should not be truncated

  Scenario: Head-truncate by line limit
    Given a string with 3000 lines of 10 bytes each
    When I head-truncate with max 2000 lines and 50KB bytes
    Then exactly 2000 lines should be returned
    And the result should be truncated by lines
    And total_lines should be 3000
    And output_lines should be 2000

  Scenario: Head-truncate by byte limit
    Given a string with 100 lines of 1000 bytes each
    When I head-truncate with max 2000 lines and 50KB bytes
    Then the result should be truncated by bytes
    And the output should be at most 50KB
    And no partial lines should be present

  Scenario: Head-truncate where first line exceeds byte limit
    Given a single line of 60000 bytes
    When I head-truncate with max 2000 lines and 50KB bytes
    Then the result content should be empty
    And first_line_exceeds_limit should be true
    And the result should be truncated

  Scenario: Head-truncate content exactly at limits
    Given a string with exactly 2000 lines totalling exactly 50KB
    When I head-truncate with max 2000 lines and 50KB bytes
    Then all 2000 lines should be returned
    And the result should not be truncated

  Scenario: Head-truncate preserves multi-byte UTF-8
    Given a string with lines containing multi-byte UTF-8 characters
    When I head-truncate with max 10 lines and 100 bytes
    Then the result should not split any UTF-8 codepoints
    And the result should be valid UTF-8

  # --- truncate_tail ---

  Scenario: Tail-truncate empty string returns empty result
    Given an empty string to truncate
    When I tail-truncate with max 2000 lines and 50KB bytes
    Then the truncation result should be empty
    And the result should not be truncated

  Scenario: Tail-truncate short content is returned unchanged
    Given a string with 5 lines totalling 50 bytes
    When I tail-truncate with max 2000 lines and 50KB bytes
    Then all 5 lines should be returned
    And the result should not be truncated

  Scenario: Tail-truncate by line limit keeps last lines
    Given a string with 3000 lines of 10 bytes each
    When I tail-truncate with max 2000 lines and 50KB bytes
    Then exactly 2000 lines should be returned
    And the result should be truncated by lines
    And the result should contain the last line of the input
    And the result should not contain the first line of the input

  Scenario: Tail-truncate by byte limit keeps last bytes
    Given a string with 100 lines of 1000 bytes each
    When I tail-truncate with max 2000 lines and 50KB bytes
    Then the result should be truncated by bytes
    And the output should be at most 50KB
    And the result should contain the last line of the input

  Scenario: Tail-truncate single huge last line takes partial tail
    Given a single line of 60000 bytes
    When I tail-truncate with max 2000 lines and 50KB bytes
    Then the result should be truncated
    And last_line_partial should be true
    And the output should be at most 50KB

  # --- truncate_line ---

  Scenario: Truncate line shorter than limit is unchanged
    Given a line of 100 characters
    When I truncate the line to 500 characters
    Then the line should be returned unchanged
    And the line should not be marked as truncated

  Scenario: Truncate line longer than limit is cut with suffix
    Given a line of 600 characters
    When I truncate the line to 500 characters
    Then the line should be at most 500 characters plus suffix
    And the line should end with "... [truncated]"
    And the line should be marked as truncated

  # --- format_size ---

  Scenario: Format bytes as human-readable size
    Then format_size should produce the expected output for each
      | bytes   | expected |
      | 0       | 0B       |
      | 512     | 512B     |
      | 1024    | 1.0KB    |
      | 1536    | 1.5KB    |
      | 51200   | 50.0KB   |
      | 1048576 | 1.0MB    |
      | 1572864 | 1.5MB    |

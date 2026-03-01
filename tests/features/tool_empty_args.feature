@done
Feature: Tool Empty Arguments Handling
  As an AI agent
  I want graceful handling when tool calls arrive with empty or missing arguments
  So that I get actionable error messages instead of cryptic JSON parse failures

  Background:
    Given a tool workspace

  # --- Fix 1: Empty argument normalisation ---

  Scenario: Empty string arguments are normalised to empty JSON object
    When the agent executes tool "ls" with raw arguments ""
    Then the tool result should not be an error
    And the tool result should not contain "EOF while parsing"

  Scenario: Whitespace-only arguments are normalised to empty JSON object
    When the agent executes tool "ls" with raw arguments "   "
    Then the tool result should not be an error
    And the tool result should not contain "EOF while parsing"

  # --- Fix 2: Actionable missing-parameter errors ---

  Scenario: find with empty args returns actionable error mentioning required params
    When the agent executes tool "find" with raw arguments "{}"
    Then the tool result should be an error
    And the tool result should contain "pattern"
    And the tool result should contain "Example"
    And the tool result should not contain "EOF while parsing"

  Scenario: grep with empty args returns actionable error mentioning required params
    When the agent executes tool "grep" with raw arguments "{}"
    Then the tool result should be an error
    And the tool result should contain "pattern"
    And the tool result should contain "Example"
    And the tool result should not contain "EOF while parsing"

  Scenario: read with empty args returns actionable error mentioning required params
    When the agent executes tool "read" with raw arguments "{}"
    Then the tool result should be an error
    And the tool result should contain "path"
    And the tool result should contain "Example"
    And the tool result should not contain "EOF while parsing"

  Scenario: write with empty args returns actionable error mentioning required params
    When the agent executes tool "write" with raw arguments "{}"
    Then the tool result should be an error
    And the tool result should contain "path"
    And the tool result should contain "Example"
    And the tool result should not contain "EOF while parsing"

  Scenario: edit with empty args returns actionable error mentioning required params
    When the agent executes tool "edit" with raw arguments "{}"
    Then the tool result should be an error
    And the tool result should contain "path"
    And the tool result should contain "Example"
    And the tool result should not contain "EOF while parsing"

  Scenario: bash with empty args returns actionable error mentioning required params
    When the agent executes tool "bash" with raw arguments "{}"
    Then the tool result should be an error
    And the tool result should contain "command"
    And the tool result should contain "Example"
    And the tool result should not contain "EOF while parsing"

  # --- Fix 3: Tool descriptions include usage examples ---

  Scenario: find tool description includes usage example
    Given a tool workspace
    Then the tool definition for "find" should contain "Example"

  Scenario: grep tool description includes usage example
    Given a tool workspace
    Then the tool definition for "grep" should contain "Example"

  Scenario: read tool description includes usage example
    Given a tool workspace
    Then the tool definition for "read" should contain "Example"

  Scenario: write tool description includes usage example
    Given a tool workspace
    Then the tool definition for "write" should contain "Example"

  Scenario: edit tool description includes usage example
    Given a tool workspace
    Then the tool definition for "edit" should contain "Example"

  Scenario: bash tool description includes usage example
    Given a tool workspace
    Then the tool definition for "bash" should contain "Example"

  Scenario: ls tool description includes usage example
    Given a tool workspace
    Then the tool definition for "ls" should contain "Example"

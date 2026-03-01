@done
Feature: ensure_tool utility — auto-download rg and fd binaries
  As a tool operator
  I want rg and fd to be available automatically
  So that users don't need to pre-install external dependencies

  Scenario: PATH lookup succeeds when binary is available
    When I ensure tool "rg" is available with mock binary on PATH
    Then the ensure_tool result should be a valid path
    And the ensure_tool result should not be an error

  Scenario: Cached binary is used when available
    Given a cached binary "rg" in the tools cache directory
    When I ensure tool "rg" is available without PATH
    Then the ensure_tool result should be a valid path
    And the ensure_tool result path should be in the cache directory
    And the ensure_tool result should not be an error

  @done
  Scenario: PATH takes priority over cache
    Given a cached binary "rg" in the tools cache directory
    When I ensure tool "rg" is available with mock binary on PATH
    Then the ensure_tool result should be a valid path
    And the ensure_tool result should not be an error

  @done
  Scenario: Offline mode returns error without downloading
    When I ensure tool "rg" with QUECTO_OFFLINE=1 and no PATH/cache
    Then the ensure_tool result should be an error
    And the ensure_tool result should contain "offline"

  @done
  Scenario: Unsupported tool name returns error
    When I ensure tool "unknown_tool_xyz" is available
    Then the ensure_tool result should be an error

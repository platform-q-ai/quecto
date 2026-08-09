Feature: Quecto MCP UDS extension
  Quecto needs a standalone MCP client extension that registers remote MCP tools
  into a running Quecto UDS agent and proxies tool executions back to the MCP server.

  Scenario: Maps Perme8 MCP tool names to Quecto-safe names
    Given an MCP tool named "community.channels.send_message"
    When I map the MCP tool name for Quecto
    Then the Quecto tool name should be "community_channels_send_message"

  Scenario: Filters discovered MCP tools by prefix
    Given discovered MCP tools:
      | name                            |
      | community.feed.list             |
      | community.channels.send_message |
      | ticket.read                     |
    When I filter tools with prefix "community."
    Then the filtered MCP tool names should be:
      | name                            |
      | community.feed.list             |
      | community.channels.send_message |

  Scenario: Filters discovered MCP tools with allowlist, prefix, and denylist precedence
    Given discovered MCP tools:
      | name                            |
      | community.feed.list             |
      | community.channels.send_message |
      | ticket.read                     |
    When I filter tools with prefix "community." allowlist "community.feed.list,ticket.read" and denylist "community.feed.list"
    Then the filtered MCP tool names should be:
      | name |

  Scenario: Allows explicit allowlist to disable the default Community prefix
    Given discovered MCP tools:
      | name        |
      | ticket.read |
    When I filter tools with allowlist "ticket.read"
    Then the filtered MCP tool names should be:
      | name        |
      | ticket.read |

  Scenario: Builds a Quecto tool registration from an MCP tool
    Given an MCP tool named "community.feed.list"
    And the MCP tool description is "List feed posts"
    And the MCP tool input schema is "{\"type\":\"object\"}"
    When I build a Quecto registration
    Then the Quecto tool name should be "community_feed_list"
    And the Quecto tool description should be "List feed posts"
    And the Quecto tool schema should be "{\"type\":\"object\"}"

  Scenario: Defaults to Perme8 Community tools when no filter is configured
    Given required quecto-mcp connection arguments
    When I parse the quecto-mcp configuration
    Then the configured tool prefixes should be:
      | prefix     |
      | community. |

  Scenario: Applies configured Quecto tool name prefix
    Given an MCP tool named "community.feed.list"
    When I build a Quecto registration with name prefix "mcp_"
    Then the Quecto tool name should be "mcp_community_feed_list"

  Scenario: Rejects invalid configured Quecto tool name prefix
    Given an MCP tool named "community.feed.list"
    When I try to build a Quecto registration with name prefix "1_"
    Then quecto-mcp should reject the MCP tool configuration

  Scenario: Rejects MCP tool name collisions
    Given discovered MCP tools:
      | name |
      | a.b  |
      | a_b  |
    When I build Quecto tool registrations for the discovered tools
    Then quecto-mcp should reject the MCP tool configuration

  Scenario: Rejects duplicate MCP tool names
    Given discovered MCP tools:
      | name                |
      | community.feed.list |
      | community.feed.list |
    When I build Quecto tool registrations for the discovered tools
    Then quecto-mcp should reject the MCP tool configuration

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

  Scenario: Builds a Quecto tool registration from an MCP tool
    Given an MCP tool named "community.feed.list"
    And the MCP tool description is "List feed posts"
    And the MCP tool input schema is "{\"type\":\"object\"}"
    When I build a Quecto registration
    Then the Quecto tool name should be "community_feed_list"
    And the Quecto tool description should be "List feed posts"
    And the Quecto tool schema should be "{\"type\":\"object\"}"

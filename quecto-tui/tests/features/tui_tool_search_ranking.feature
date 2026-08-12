Feature: TUI tool search ranking
  Users can quickly find the intended tool when filtering the tool management list.

  @done
  Scenario: Tool id and name matches outrank description-only matches
    Given the TUI has tools with ids, labels, aliases, and descriptions
    When the user filters tools by "read"
    Then tools whose id or label contains "read" are ranked before tools that only mention "read" in the description

  @done
  Scenario: Prefix and word-boundary matches outrank scattered character matches
    Given the TUI has tools with ids, labels, aliases, and descriptions
    When the user filters tools by "web"
    Then tools with prefix or word-boundary matches for "web" are ranked before tools with only scattered character matches

  @done
  Scenario: The matching tool name outranks separate description mentions
    Given the TUI has tools with ids, labels, aliases, and descriptions
    When the user filters tools by "web fetch"
    Then the Web Fetch tool is ranked before tools that mention "web" and "fetch" only in unrelated description text

  @done
  Scenario: Tool aliases find the intended tool
    Given the TUI has tools with ids, labels, aliases, and descriptions
    When the user filters tools by "shell"
    Then the bash tool is ranked before tools without the "shell" alias

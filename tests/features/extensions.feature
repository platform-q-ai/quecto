@done
Feature: Extension system
  As an agent operator
  I want to extend the agent with native extensions and UDS-registered tools
  So that I can add capabilities without recompiling or with minimal configuration

  # ─── Extension trait ────────────────────────────────────────────────────────

  Scenario: Extension trait provides tools and optional prompt snippet
    Given an extension named "test-ext" with 2 tools
    Then the extension name should be "test-ext"
    And the extension should provide 2 tools
    And the extension system prompt snippet should be None

  Scenario: Extension with system prompt snippet
    Given an extension named "test-ext" with prompt snippet "Always be polite."
    Then the extension system prompt snippet should be "Always be polite."

  # ─── Extension registry ────────────────────────────────────────────────────

  Scenario: Empty extension registry returns no tools
    Given an empty extension registry
    Then the registry should have 0 extension tools
    And the registry system prompt snippets should be empty

  Scenario: Registry returns tools from registered extension
    Given an empty extension registry
    And an extension named "my-ext" with 2 tools is registered
    Then the registry should have 2 extension tools

  Scenario: Registry concatenates prompt snippets from multiple extensions
    Given an empty extension registry
    And an extension with prompt snippet "Snippet A" is registered
    And an extension with prompt snippet "Snippet B" is registered
    Then the registry system prompt snippets should contain "Snippet A"
    And the registry system prompt snippets should contain "Snippet B"

  Scenario: Registry deduplicates tools by name — last wins
    Given an empty extension registry
    And an extension with tool named "mytool" and description "first" is registered
    And an extension with tool named "mytool" and description "second" is registered
    Then the registry should have 1 extension tools
    And the extension tool "mytool" should have description "second"

  # ─── Native extensions (#351) ───────────────────────────────────────────────

  Scenario: NativeExtension wraps a tool as an extension
    Given a native extension named "web_search" wrapping a tool with description "Search the web"
    Then the native extension name should be "web_search"
    And the native extension should provide 1 tool
    And the native extension tool should have name "web_search"

  Scenario: NativeExtension with system prompt snippet
    Given a native extension named "web_search" with system prompt "Use web_search to find information online."
    Then the native extension system prompt snippet should be "Use web_search to find information online."

  Scenario: NativeExtension without system prompt snippet
    Given a native extension named "web_search" wrapping a tool with description "Search the web"
    Then the native extension system prompt snippet should be None

  Scenario: NativeExtension registered in ExtensionRegistry
    Given an empty extension registry
    And a native extension named "web_search" is registered in the extension registry
    Then the registry should have 1 extension tools
    And the registry should contain tool "web_search"

  Scenario: NativeExtension registered via ToolRegistryImpl as extension tool
    Given a tool registry with core tools
    And a native extension "web_search" registered as an extension tool
    Then the tool registry should contain "web_search"
    And the tool registry extension names should include "web_search"
    And the tool registry extension names should not include "bash"

  Scenario: NativeExtension cannot shadow core tools
    Given a tool registry with core tools
    And a native extension "bash" registered as an extension tool
    Then the tool registry extension names should not include "bash"

  Scenario: WebSearchTool registered when config enables Brave
    Given a config with tools.web.brave.enabled = true and api_key = "test-brave-key"
    When I build native extensions from config
    Then the native extensions list should contain "web"
    And the built native extension "web" should provide tool "web_search"

  Scenario: WebSearchTool registered when config enables DuckDuckGo
    Given a config with tools.web.duckduckgo.enabled = true
    When I build native extensions from config
    Then the native extensions list should contain "web"
    And the built native extension "web" should provide tool "web_search"

  Scenario: No web extension when all web tools are disabled
    Given a config with tools.web.brave.enabled = false and tools.web.duckduckgo.enabled = false
    And config with tools.web.fetch.enabled = false
    When I build native extensions from config
    Then the native extensions list should not contain "web"

  Scenario: WebSearchTool uses Brave when API key is configured
    Given a config with tools.web.brave.enabled = true and api_key = "test-brave-key"
    When I build native extensions from config
    Then the web_search native extension should use Brave backend

  Scenario: WebSearchTool uses DuckDuckGo when no Brave API key
    Given a config with tools.web.duckduckgo.enabled = true
    When I build native extensions from config
    Then the web_search native extension should use DuckDuckGo backend

  # ─── Multi-tool native extension (#364) ─────────────────────────────────────

  Scenario: NativeExtension with multiple tools
    Given a native extension named "web" with tools "web_search" and "web_fetch"
    Then the native extension name should be "web"
    And the native extension should provide 2 tools
    And the native extension tool should have name "web_search"
    And the native extension tool should have name "web_fetch"

  # ─── WebFetchTool config gating (#364) ──────────────────────────────────────

  Scenario: WebFetchTool registered when config enables fetch
    Given a config with tools.web.fetch.enabled = true
    When I build native extensions from config
    Then the native extensions list should contain "web"
    And the built native extension "web" should provide tool "web_fetch"

  Scenario: WebFetchTool not registered when fetch is disabled
    Given a config with tools.web.fetch.enabled = false
    When I build native extensions from config
    Then there should be no built native extension providing tool "web_fetch"

  Scenario: Both web_search and web_fetch in single extension when both enabled
    Given a config with tools.web.brave.enabled = true and api_key = "test-key" and fetch.enabled = true
    When I build native extensions from config
    Then the native extensions list should contain "web"
    And the built native extension "web" should provide tool "web_search"
    And the built native extension "web" should provide tool "web_fetch"

  Scenario: Only web_search when fetch disabled
    Given a config with tools.web.brave.enabled = true and api_key = "test-key" and fetch.enabled = false
    When I build native extensions from config
    Then the native extensions list should contain "web"
    And the built native extension "web" should provide tool "web_search"
    And there should be no built native extension providing tool "web_fetch"

  Scenario: Only web_fetch when search disabled
    Given a config with tools.web.fetch.enabled = true and search disabled
    When I build native extensions from config
    Then the native extensions list should contain "web"
    And the built native extension "web" should provide tool "web_fetch"
    And there should be no built native extension providing tool "web_search"

  Scenario: No web extension when all web tools disabled
    Given a config with tools.web.brave.enabled = false and tools.web.duckduckgo.enabled = false
    And config with tools.web.fetch.enabled = false
    When I build native extensions from config
    Then the native extensions list should not contain "web"

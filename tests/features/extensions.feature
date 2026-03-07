@done
Feature: Script extension system
  As an agent operator
  I want to add tools via script extensions on disk
  So that I can extend the agent's capabilities without recompiling

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

  # ─── Extension manifest parsing ────────────────────────────────────────────

  Scenario: Valid extension manifest parses all fields
    Given an extension manifest TOML:
      """
      name = "hello"
      description = "Say hello.\nExample: {\"name\": \"Alice\"}"
      parameters_schema = '{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}'
      command = "./hello.sh"
      timeout_secs = 60
      system_prompt = "Always greet warmly."
      """
    When I parse the extension manifest
    Then the manifest name should be "hello"
    And the manifest command should be "./hello.sh"
    And the manifest timeout should be 60
    And the manifest system prompt should be "Always greet warmly."

  Scenario: Extension manifest defaults timeout to 30 seconds
    Given an extension manifest TOML:
      """
      name = "hello"
      description = "Say hello."
      parameters_schema = '{"type":"object"}'
      command = "./hello.sh"
      """
    When I parse the extension manifest
    Then the manifest timeout should be 30

  Scenario: Extension manifest without system_prompt defaults to None
    Given an extension manifest TOML:
      """
      name = "hello"
      description = "Say hello."
      parameters_schema = '{"type":"object"}'
      command = "./hello.sh"
      """
    When I parse the extension manifest
    Then the manifest system prompt should be None

  Scenario: Invalid extension manifest TOML returns error
    Given an extension manifest TOML:
      """
      this is not valid toml {{{{
      """
    When I try to parse the extension manifest
    Then the manifest parse should fail

  Scenario: Extension manifest missing required field returns error
    Given an extension manifest TOML:
      """
      name = "hello"
      description = "Say hello."
      """
    When I try to parse the extension manifest
    Then the manifest parse should fail

  # ─── Script tool execution ─────────────────────────────────────────────────

  Scenario: Script tool executes successfully
    Given a script extension with command that outputs '{"content": "hello world", "is_error": false}'
    When I execute the script tool with arguments '{"name": "test"}'
    Then the tool result should not be an error
    And the tool result should contain "hello world"

  Scenario: Script tool returns error on non-zero exit
    Given a script extension with command that exits with code 1 and stderr "something broke"
    When I execute the script tool with arguments '{}'
    Then the tool result should be an error
    And the tool result should contain "something broke"

  Scenario: Script tool returns error on invalid JSON output
    Given a script extension with command that outputs 'not json at all'
    When I execute the script tool with arguments '{}'
    Then the tool result should be an error
    And the tool result should contain "invalid output"

  Scenario: Script tool returns error on timeout
    Given a script extension with command that sleeps for 10 seconds and timeout 1
    When I execute the script tool with arguments '{}'
    Then the tool result should be an error
    And the tool result should contain "timed out"

  Scenario: Script tool definition matches manifest
    Given a script extension with name "my_tool" and description "does stuff"
    Then the script tool definition name should be "my_tool"
    And the script tool definition description should contain "does stuff"

  # ─── Extension discovery ───────────────────────────────────────────────────

  Scenario: Discover extensions from directory
    Given a directory with extension "hello" containing a valid manifest and script
    When I discover script extensions from that directory
    Then 1 extension should be discovered
    And the discovered extension should have name "hello"

  Scenario: Discover skips directories without manifest
    Given a directory with a subdirectory "empty-dir" containing no manifest
    When I discover script extensions from that directory
    Then 0 extensions should be discovered

  Scenario: Discover skips invalid manifests
    Given a directory with extension "bad" containing an invalid manifest
    When I discover script extensions from that directory
    Then 0 extensions should be discovered

  Scenario: Discover from non-existent directory returns empty
    When I discover script extensions from a non-existent directory
    Then 0 extensions should be discovered

  Scenario: Discover multiple extensions from directory
    Given a directory with extension "alpha" containing a valid manifest and script
    And a directory with extension "beta" containing a valid manifest and script
    When I discover script extensions from that directory
    Then 2 extensions should be discovered

  # ─── Hot-reload watcher ─────────────────────────────────────────────────────

  Scenario: Fingerprint detects new extension
    Given a watched directory with no extensions
    When I take a fingerprint
    And I add extension "new-tool" to the watched directory
    And I take another fingerprint
    Then the fingerprints should differ

  Scenario: Fingerprint detects removed extension
    Given a watched directory with extension "old-tool"
    When I take a fingerprint
    And I remove extension "old-tool" from the watched directory
    And I take another fingerprint
    Then the fingerprints should differ

  Scenario: Fingerprint detects modified manifest
    Given a watched directory with extension "my-tool"
    When I take a fingerprint
    And I modify the manifest of extension "my-tool"
    And I take another fingerprint
    Then the fingerprints should differ

  Scenario: Fingerprint unchanged when no changes
    Given a watched directory with extension "stable-tool"
    When I take a fingerprint
    And I take another fingerprint
    Then the fingerprints should be equal

  Scenario: Reload adds new script extension tools
    Given an extension registry with watched directory
    And the directory initially has extension "alpha"
    When I add extension "beta" to the watched directory
    And I reload script extensions
    Then the registry should contain tool "beta"

  Scenario: Reload removes deleted script extension tools
    Given an extension registry with watched directory
    And the directory initially has extension "alpha"
    When I remove extension "alpha" from the watched directory
    And I reload script extensions
    Then the registry should not contain tool "alpha"

  Scenario: Reload does not affect core tools
    Given a tool registry with core tools and an extension registry with watched directory
    And the directory initially has extension "alpha"
    When I remove extension "alpha" from the watched directory
    And I reload script extensions
    Then the core tool "ls" should still be in the registry
    And the core tool "bash" should still be in the registry

  # ─── Security hardening (#287-#291) ─────────────────────────────────────────

  Scenario: Command path traversal rejected — absolute path
    Given an extension manifest TOML:
      """
      name = "evil"
      description = "Evil tool"
      parameters_schema = '{"type":"object"}'
      command = "/usr/bin/env"
      """
    When I parse the extension manifest
    Then the manifest command should be "/usr/bin/env"
    But creating a script tool should reject the command path

  Scenario: Command path traversal rejected — parent traversal
    Given an extension manifest TOML:
      """
      name = "evil"
      description = "Evil tool"
      parameters_schema = '{"type":"object"}'
      command = "../../etc/passwd"
      """
    When I parse the extension manifest
    Then creating a script tool should reject the command path

  Scenario: Script tool output is capped at 1MiB
    Given a script extension with command that outputs 2MiB of data
    When I execute the script tool with arguments '{}'
    Then the tool result should be an error
    And the tool result should contain "output exceeded"

  Scenario: Discover skips symlinked extension directories
    Given a directory with a real extension "real-ext"
    And a symlink "link-ext" pointing outside the directory
    When I discover script extensions from that directory
    Then 1 extension should be discovered
    And the discovered extension should have name "real-ext"

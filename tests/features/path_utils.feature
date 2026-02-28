@done
Feature: Path utilities module
  As a tool implementer
  I want a shared path resolution module with ~ expansion and absolute path support
  So that all tools consistently resolve user-supplied paths

  # --- resolve_to_cwd ---

  Scenario: Relative path resolved against workspace
    Given a workspace directory at a temp path
    When I resolve path "subdir/file.txt" relative to the workspace
    Then the resolved path should equal workspace joined with "subdir/file.txt"

  Scenario: Absolute path returned as-is
    Given a workspace directory at a temp path
    When I resolve path "/etc/hosts" relative to the workspace
    Then the resolved path should be "/etc/hosts"

  Scenario: Tilde alone expands to home directory
    Given a workspace directory at a temp path
    When I resolve path "~" relative to the workspace
    Then the resolved path should equal the home directory

  Scenario: Tilde slash expands to home-relative path
    Given a workspace directory at a temp path
    When I resolve path "~/projects/foo.txt" relative to the workspace
    Then the resolved path should equal the home directory joined with "projects/foo.txt"

  Scenario: At-sign prefix is stripped before resolution
    Given a workspace directory at a temp path
    When I resolve path "@src/main.rs" relative to the workspace
    Then the resolved path should equal workspace joined with "src/main.rs"

  Scenario: Unicode space normalised to regular space
    Given a workspace directory at a temp path
    When I resolve path containing a non-breaking space in name "my\u00A0file.txt"
    Then the resolved path should equal workspace joined with "my file.txt"

  Scenario: Dot resolves to workspace root
    Given a workspace directory at a temp path
    When I resolve path "." relative to the workspace
    Then the resolved path should equal the workspace root

  # --- resolve_read_path ---

  Scenario: Existing file returned directly by resolve_read_path
    Given a workspace directory at a temp path
    And a file "readme.md" exists in the workspace
    When I resolve read path "readme.md" relative to the workspace
    Then the resolved read path should exist on disk

  Scenario: Non-existent file returns resolved path anyway
    Given a workspace directory at a temp path
    When I resolve read path "missing.txt" relative to the workspace
    Then the resolved read path should equal workspace joined with "missing.txt"

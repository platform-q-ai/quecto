@done
Feature: SpawnTool — child agent process spawning
  As an AI agent
  I want the SpawnTool to correctly parse, validate, and execute spawn requests
  So that subagent tasks run with the right configuration and error handling

  # --- Argument parsing ---

  Scenario: Parse a valid task-only request
    Given a SpawnTool with allowlist "news-bot,weather-bot" and restrict_to_workspace true
    When I parse spawn arguments '{"task":"Summarize news"}'
    Then the parsed config should have task "Summarize news"
    And the parsed config should have no agent_id
    And the parsed config should have restrict_to_workspace true

  Scenario: Parse a request with agent_id
    Given a SpawnTool with allowlist "news-bot,weather-bot" and restrict_to_workspace true
    When I parse spawn arguments '{"task":"Get weather","agent_id":"weather-bot"}'
    Then the parsed config should have task "Get weather"
    And the parsed config should have agent_id "weather-bot"

  Scenario: Parse a request with system prompt
    Given a SpawnTool with allowlist "news-bot,weather-bot" and restrict_to_workspace true
    When I parse spawn arguments '{"task":"Translate","system":"You are a translator"}'
    Then the parsed config should have system prompt "You are a translator"

  Scenario: Parse a request without task field succeeds
    Given a SpawnTool with allowlist "news-bot" and restrict_to_workspace true
    When I parse spawn arguments '{"agent_id":"news-bot"}'
    Then the spawn result should not be an error

  Scenario: Parse fails on invalid JSON
    Given a SpawnTool with allowlist "news-bot" and restrict_to_workspace true
    When I parse spawn arguments '{garbage}}}'
    Then the parse should fail with "invalid JSON"

  Scenario: Parse fails on empty object with no task and no agent_id
    Given a SpawnTool with allowlist "news-bot" and restrict_to_workspace true
    When I parse spawn arguments '{}'
    Then the spawn result should not be an error

  Scenario: Parse fails when task is null
    Given a SpawnTool with allowlist "news-bot" and restrict_to_workspace true
    When I parse spawn arguments '{"task":null}'
    Then the spawn result should not be an error

  Scenario: Non-string agent_id is ignored
    Given a SpawnTool with allowlist "news-bot" and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","agent_id":999}'
    Then the parsed config should have no agent_id

  Scenario: Non-string system prompt is ignored
    Given a SpawnTool with allowlist "news-bot" and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","system":123}'
    Then the parsed config should have no system prompt

  # --- Agent ID validation ---

  Scenario: Disallowed agent_id is rejected
    Given a SpawnTool with allowlist "news-bot" and restrict_to_workspace true
    When I parse spawn arguments '{"task":"evil","agent_id":"evil-bot"}'
    Then the parse should fail with "not allowed"

  Scenario: Empty allowlist permits any agent_id
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","agent_id":"any-bot"}'
    Then the parsed config should have agent_id "any-bot"

  Scenario: Agent ID with path traversal is rejected
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","agent_id":"../escape"}'
    Then the parse should fail with "[a-zA-Z0-9_-]"

  Scenario: Agent ID with spaces is rejected
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","agent_id":"hello world"}'
    Then the parse should fail with "[a-zA-Z0-9_-]"

  Scenario: Empty agent ID is rejected
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","agent_id":""}'
    Then the parse should fail with "1-64 characters"

  Scenario: Agent ID at max length 64 is accepted
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments with a 64-character agent_id
    Then the parsed config should have an agent_id of length 64

  Scenario: Agent ID exceeding 64 characters is rejected
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments with a 65-character agent_id
    Then the parse should fail with "1-64 characters"

  # --- Workspace restriction inheritance ---

  Scenario: restrict_to_workspace true is inherited
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    When I parse spawn arguments '{"task":"a"}'
    Then the parsed config should have restrict_to_workspace true

  Scenario: restrict_to_workspace false is inherited
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace false
    When I parse spawn arguments '{"task":"a"}'
    Then the parsed config should have restrict_to_workspace false

  # --- Network passthrough ---

  Scenario: Network passthrough is disabled by default
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    Then the SpawnTool should have network_passthrough false

  Scenario: Network passthrough can be enabled
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    When I enable network passthrough on the SpawnTool
    Then the SpawnTool should have network_passthrough true

  # --- Constructors ---

  Scenario: with_base_dir sets base directory
    Given a SpawnTool created with base_dir "/tmp/quecto-test"
    Then the SpawnTool should have base_dir "/tmp/quecto-test"

  Scenario: new constructor sets empty base_dir
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    Then the SpawnTool should have an empty base_dir

  # --- Tool definition ---

  Scenario: Tool definition has correct name and schema
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    Then the tool definition name should be "spawn"
    And the tool definition description should not be empty
    And the tool definition description should mention "agent_cmd"

  Scenario: Tool definition does not require task
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    Then the spawn tool schema should not require "task"

  # --- Stub-mode execution ---

  Scenario: Execute in stub mode returns success with agent_cmd reference
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I execute the SpawnTool with '{"task":"Do something useful"}'
    Then the spawn result should not be an error
    And the spawn result should contain "agent_cmd"

  Scenario: Execute in stub mode without task returns idle agent
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I execute the SpawnTool with '{"agent_id":"idle-worker"}'
    Then the spawn result should not be an error
    And the spawn result should contain "agent_cmd"

  Scenario: Execute with invalid JSON returns error
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I execute the SpawnTool with 'not valid json'
    Then the spawn result should be an error
    And the spawn result should contain "invalid JSON"

  Scenario: Execute with disallowed agent returns error
    Given a SpawnTool with allowlist "allowed-bot" and restrict_to_workspace true
    When I execute the SpawnTool with '{"task":"evil","agent_id":"not-allowed"}'
    Then the spawn result should be an error
    And the spawn result should contain "not allowed"

  Scenario: Execute with invalid agent_id format returns error
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I execute the SpawnTool with '{"task":"test","agent_id":"bad id!"}'
    Then the spawn result should be an error
    And the spawn result should contain "[a-zA-Z0-9_-]"

  # --- Subagent registry ---

  Scenario: Spawn registers child in shared registry
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I execute the SpawnTool with '{"task":"work","agent_id":"worker-1"}'
    Then the spawn result should not be an error
    And the subagent registry should contain "worker-1"

  Scenario: Spawn without agent_id uses default name
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I execute the SpawnTool with '{"task":"work"}'
    Then the spawn result should not be an error
    And the subagent registry should contain "subagent"

  # --- Debug trait ---

  Scenario: Debug output includes struct fields
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    Then the debug output should include "SpawnTool"
    And the debug output should include "bot"
    And the debug output should include "restrict_to_workspace: true"

  # --- config forwarding ---

  
  Scenario: Parse a request with config path
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","config":"/custom/config.json"}'
    Then the spawn result should not be an error
    And the parsed spawn config should have config path "/custom/config.json"

  
  Scenario: Parse a request without config path has no config
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work"}'
    Then the spawn result should not be an error
    And the parsed spawn config should have no config path

  
  Scenario: Non-string config path is ignored
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","config":123}'
    Then the spawn result should not be an error
    And the parsed spawn config should have no config path

  Scenario: Config path with path traversal is rejected
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","config":"../../etc/shadow"}'
    Then the spawn result should be an error
    And the spawn result should contain "'..' which is not allowed"

  # --- workflow forwarding ---

  
  Scenario: Parse a request with workflow enabled
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","workflow":true}'
    Then the spawn result should not be an error
    And the parsed spawn config should have workflow true

  
  Scenario: Parse a request without workflow defaults to false
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work"}'
    Then the spawn result should not be an error
    And the parsed spawn config should have workflow false

  
  Scenario: Non-bool workflow is ignored (defaults to false)
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","workflow":"yes"}'
    Then the spawn result should not be an error
    And the parsed spawn config should have workflow false

  # --- workflow_guards forwarding ---

  
  Scenario: Parse a request with workflow_guards enabled
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","workflow":true,"workflow_guards":true}'
    Then the spawn result should not be an error
    And the parsed spawn config should have workflow_guards true

  
  Scenario: Parse a request without workflow_guards defaults to false
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work"}'
    Then the spawn result should not be an error
    And the parsed spawn config should have workflow_guards false

  
  Scenario: Non-bool workflow_guards is ignored (defaults to false)
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","workflow_guards":1}'
    Then the spawn result should not be an error
    And the parsed spawn config should have workflow_guards false

  Scenario: workflow_guards true without workflow true is rejected
    Given a SpawnTool with empty allowlist and restrict_to_workspace true
    When I parse spawn arguments '{"task":"work","workflow_guards":true}'
    Then the spawn result should be an error
    And the spawn result should contain "workflow_guards requires workflow"

  # --- tool definition schema includes new fields ---

  
  Scenario: Tool definition schema includes config field
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    Then the spawn tool schema should include property "config"

  
  Scenario: Tool definition schema includes workflow field
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    Then the spawn tool schema should include property "workflow"

  
  Scenario: Tool definition schema includes workflow_guards field
    Given a SpawnTool with allowlist "bot" and restrict_to_workspace true
    Then the spawn tool schema should include property "workflow_guards"

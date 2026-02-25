@wip @coding-live
Feature: Worker Agent Loop
  As the coding runtime
  I need the worker loop to run an LLM agent loop with worker tools and event emission
  So that the worker can autonomously execute coding tasks inside nsjail

  The worker process receives a goal via CLI args, builds a worker tool
  registry (edit, grep, find, read), runs an LLM agent loop, and emits
  structured events to stdout via WorkerEventEmitter. Lifecycle events
  (ready, done, error) bracket the run, and tool events (tool.start,
  tool.result) are emitted around each tool execution.

  # --- Worker loop construction ---

  Scenario: Worker loop builds a tool registry with the 4 worker tools
    Given a worker loop context with a valid job directory
    When the worker loop builds the tool registry
    Then the registry should contain exactly "worker_edit, worker_grep, worker_find, worker_read"

  Scenario: Worker loop builds an event emitter with correct IDs
    Given a worker loop context with run_id "run-42" and job_id "job-7"
    When the worker loop builds the event emitter
    Then the emitter should be configured with run_id "run-42" and job_id "job-7"

  Scenario: Worker system prompt contains the goal text
    Given a worker loop context with goal "Fix the flaky test in auth module"
    When the worker loop builds the system prompt
    Then the system prompt should contain "Fix the flaky test in auth module"

  Scenario: Worker system prompt lists available tools
    Given a worker loop context with goal "any task"
    When the worker loop builds the system prompt
    Then the system prompt should contain "worker_read"
    And the system prompt should contain "worker_edit"
    And the system prompt should contain "worker_grep"
    And the system prompt should contain "worker_find"

  # --- Ready event ---

  Scenario: Worker loop emits a ready log event on startup
    Given a worker loop context with a valid job directory
    And a mock LLM provider that returns text "ok"
    When the worker loop runs to completion
    Then the first emitted event should be "log.message" with level "info"
    And the first emitted event message should contain "ready"

  # --- Successful completion ---

  Scenario: Worker loop sends the goal as a user message to the LLM
    Given a worker loop context with goal "Refactor the parser"
    And a mock LLM provider that captures messages and returns text "done"
    When the worker loop runs to completion
    Then the LLM should have received a user message containing "Refactor the parser"

  Scenario: Worker loop emits a done log event on successful completion
    Given a worker loop context with a valid job directory
    And a mock LLM provider that returns text "all fixed"
    When the worker loop runs to completion
    Then the last emitted event should be "log.message" with level "info"
    And the last emitted event message should contain "done"

  Scenario: Worker loop returns exit code 0 on success
    Given a worker loop context with a valid job directory
    And a mock LLM provider that returns text "finished"
    When the worker loop runs to completion
    Then the worker loop result should have exit code 0

  Scenario: Worker loop captures the LLM response text
    Given a worker loop context with a valid job directory
    And a mock LLM provider that returns text "I refactored 3 files"
    When the worker loop runs to completion
    Then the worker loop result should contain response "I refactored 3 files"

  # --- Error handling ---

  Scenario: Worker loop emits an error event when the provider fails
    Given a worker loop context with a valid job directory
    And a mock LLM provider that returns an error "connection refused"
    When the worker loop runs to completion
    Then the last emitted event should be "log.message" with level "error"
    And the last emitted event message should contain "provider"

  Scenario: Worker loop returns exit code 1 on provider error
    Given a worker loop context with a valid job directory
    And a mock LLM provider that returns an error "timeout"
    When the worker loop runs to completion
    Then the worker loop result should have exit code 1

  Scenario: Worker loop returns no response on provider error
    Given a worker loop context with a valid job directory
    And a mock LLM provider that returns an error "rate limited"
    When the worker loop runs to completion
    Then the worker loop result should have no response

  # --- Tool event emission ---

  Scenario: Worker loop emits tool.start before executing a tool
    Given a worker loop context with a file "main.rs" containing "fn main() {}"
    And a mock LLM provider that calls "worker_read" for "main.rs" then returns text "read it"
    When the worker loop runs to completion
    Then the emitted events should include a "tool.start" with tool "worker_read"

  Scenario: Worker loop emits tool.result after executing a tool
    Given a worker loop context with a file "main.rs" containing "fn main() {}"
    And a mock LLM provider that calls "worker_read" for "main.rs" then returns text "got it"
    When the worker loop runs to completion
    Then the emitted events should include a "tool.result" with tool "worker_read"
    And the "tool.result" event should have ok true

  Scenario: Tool result events include duration_ms
    Given a worker loop context with a file "lib.rs" containing "pub fn add() {}"
    And a mock LLM provider that calls "worker_read" for "lib.rs" then returns text "ok"
    When the worker loop runs to completion
    Then the "tool.result" event should have a numeric duration_ms

  # --- Iteration limit ---

  Scenario: Worker loop stops at max-iterations and reports limit reached
    Given a worker loop context with max_iterations 2
    And a worker loop context with a file "f.rs" containing "code"
    And a mock LLM provider that always calls "worker_read" for "f.rs"
    When the worker loop runs to completion
    Then the worker loop result should indicate iteration limit reached
    And the worker loop result should have exit code 0

  Scenario: Worker loop emits exactly N tool.start events when limited to N iterations
    Given a worker loop context with max_iterations 3
    And a worker loop context with a file "f.rs" containing "code"
    And a mock LLM provider that always calls "worker_read" for "f.rs"
    When the worker loop runs to completion
    Then the emitted events should include exactly 3 "tool.start" events

  # --- Multiple tool calls ---

  Scenario: Worker loop emits paired events for sequential tool calls
    Given a worker loop context with a file "a.rs" containing "fn a() {}"
    And a mock LLM provider that calls tools "worker_find, worker_read" for "a.rs" then returns text "done"
    When the worker loop runs to completion
    Then the emitted events should include exactly 2 "tool.start" events
    And the emitted events should include exactly 2 "tool.result" events

  Scenario: Event sequence follows ready, tool pairs, done ordering
    Given a worker loop context with a file "x.rs" containing "let x = 1;"
    And a mock LLM provider that calls "worker_read" for "x.rs" then returns text "read"
    When the worker loop runs to completion
    Then the event type sequence should be "log.message, tool.start, tool.result, log.message"

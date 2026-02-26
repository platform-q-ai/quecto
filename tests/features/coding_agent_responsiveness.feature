@done
Feature: Main Agent Responsiveness During Coding Jobs
  As a user interacting with the main agent
  I want the agent to remain conversational while coding jobs run in background
  So that I am not blocked waiting for long-running coding tasks to finish

  The coding coordinator runs as an async background task. The main agent's
  tool loop is not blocked by coding job execution. The agent can process
  user messages, answer questions, and use non-coding tools while jobs run.

  Background:
    Given a configured main agent with a coding coordinator
    And a mock LLM provider

  # --- Non-blocking job launch ---

  Scenario: Main agent returns immediately after starting a coding job
    When the user asks the agent to start a coding job "Fix the login bug"
    And the agent calls the coding_job tool with action "run"
    Then the agent should receive an acknowledgement with run_id and job_id
    And the agent should respond to the user without waiting for job completion

  Scenario: Main agent can start multiple coding jobs concurrently
    When the user asks the agent to start coding job "Fix login bug"
    And then immediately asks to start coding job "Add search feature"
    Then both jobs should be accepted by the coordinator
    And the agent should respond to both requests promptly

  # --- Conversational while jobs run ---

  Scenario: Main agent responds to user questions while a coding job runs
    Given a coding job is running in the background
    When the user asks "What is the capital of France?"
    Then the agent should respond with an answer
    And the coding job should continue running undisturbed

  Scenario: Main agent can use non-coding tools while a coding job runs
    Given a coding job is running in the background
    When the user asks the agent to read a file using the read_file tool
    Then the agent should execute the read_file tool and return the result
    And the coding job should still be running

  Scenario: Main agent can handle multiple conversations while jobs run
    Given 3 coding jobs are running in the background
    When the user sends 5 unrelated messages
    Then all 5 messages should receive responses
    And all 3 coding jobs should continue running

  # --- Job status checking ---

  Scenario: Main agent checks job status without blocking
    Given a coding job is running with progress 40
    When the user asks "How is the coding job going?"
    And the agent calls the coding_job tool with action "status"
    Then the agent should receive the current status with progress 40
    And should relay a summary to the user

  Scenario: Main agent receives job completion notification
    Given a coding job was running in the background
    When the coding job completes with state "succeeded"
    Then the coordinator should make the result available
    And the next time the agent processes a message it should be aware of the completion

  # --- Goal-based decisions after completion ---

  Scenario: Main agent decides next action based on job result
    Given a coding job completed with state "succeeded" and summary "all tests pass"
    When the user asks the agent to review the result
    And the agent calls the coding_job tool with action "status"
    Then the agent should receive the success summary and artifacts
    And the agent should be able to decide whether to publish or iterate

  Scenario: Main agent replans after job failure
    Given a coding job completed with state "failed" and error_code "tool_error"
    When the user asks the agent what happened
    And the agent calls the coding_job tool with action "status"
    Then the agent should receive the failure details
    And the agent should be able to start a new job with a revised approach

  # --- Blocked job interaction ---

  Scenario: Main agent receives blocked job notification and provides decision
    Given a coding job transitions to "blocked" with reason "ambiguous test requirements"
    When the user asks about the job status
    Then the agent should report the blocked state and reason
    And the user can provide guidance through the agent
    And the agent can relay the decision to unblock the job

  # --- Cancel from conversation ---

  Scenario: User cancels a running job through conversation
    Given a coding job is running in the background
    When the user says "cancel that coding job"
    And the agent calls the coding_job tool with action "cancel"
    Then the cancel response should include the job_id and state "canceled"
    And the agent should confirm the cancellation to the user

  Scenario: User cancels a queued job before it starts
    Given a coding job is queued
    When the user says "never mind, cancel the job"
    And the agent calls the coding_job tool with action "cancel"
    Then the cancel response should include state "canceled"
    And no worker should have been launched

  # --- Wall timeout ---

  Scenario: Coding job is auto-canceled when wall timeout expires
    Given the user starts a coding job with max_wall_seconds 10
    When the job exceeds the 10-second wall timeout
    Then the coordinator should cancel the job with reason "wall_timeout"
    And the next time the agent checks status it should see state "canceled"
    And the cancel_reason should be "wall_timeout"

  Scenario: Main agent starts job with explicit wall timeout
    When the user asks the agent to start a coding job with a 5-minute limit
    And the agent calls the coding_job tool with action "run" and max_wall_seconds 300
    Then the coordinator should accept the job with max_wall_seconds 300

  # --- Run response shape ---

  Scenario: Run response includes run_id, job_id, and queued state
    When the user asks the agent to start a coding job on repo "org/myrepo" at ref "develop"
    And the agent calls the coding_job tool with action "run"
    Then the run response should include run_id and job_id
    And the initial state should be "queued"

  # --- Job resumed after unblock ---

  Scenario: Blocked job emits job.resumed after main agent provides decision
    Given a coding job transitions to "blocked" with reason "needs clarification"
    When the main agent provides a decision to unblock the job
    Then a "job.resumed" event should be emitted with reason describing the resolution
    And the job state should transition back to "running"

  # --- Cleanup from conversation ---

  Scenario: User requests cleanup of a completed job
    Given a coding job completed with state "succeeded"
    When the user says "clean up that coding job"
    And the agent calls the coding_job tool with action "cleanup"
    Then the cleanup response should indicate cleaned is true
    And the agent should confirm the cleanup to the user

  Scenario: User requests cleanup with artifact preservation
    Given a coding job completed with state "succeeded"
    When the user says "clean up the job but keep the artifacts"
    And the agent calls the coding_job tool with action "cleanup" and keep_artifacts true
    Then the repo directory should be removed
    But the artifact directory should be preserved

  # --- List active jobs ---

  Scenario: Main agent lists all active coding jobs
    Given 2 coding jobs are running and 1 is queued
    When the user asks "what coding jobs are running?"
    And the agent calls the coding_job tool with action "list" and state_filter ["queued", "preparing", "running", "blocked"]
    Then the response should include 3 jobs with their states
    And the agent should report their states and progress to the user

  # --- Completion notification mid-conversation ---

  Scenario: Agent incorporates job completion into next response
    Given a coding job completes while the user is asking an unrelated question
    When the agent processes the user's message
    Then the agent should answer the user's question
    And mention that the coding job has completed

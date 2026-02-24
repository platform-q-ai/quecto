@pending
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

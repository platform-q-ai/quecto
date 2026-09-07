@done @swarm
Feature: Swarm Tool
  As an AI agent
  I want to execute Python in the persistent task workspace
  So that I can compute exactly, test hypotheses, and keep intermediate artifacts

  Background:
    Given a swarm workspace

  Scenario: Inline code executes and returns stdout
    When I run swarm inline code "print(sum(range(101)))"
    Then the swarm result should contain "5050"
    And the swarm status should be "completed"
    And the swarm result should not be an error

  Scenario: Saved workspace file executes with arguments and stdin
    Given a swarm workspace file "echo_args.py" with content:
      """
      import sys
      print("args=" + ",".join(sys.argv[1:]))
      print("stdin=" + sys.stdin.read().strip())
      """
    When I run swarm file "echo_args.py" with args "alpha,beta" and stdin "hello"
    Then the swarm result should contain "args=alpha,beta"
    And the swarm result should contain "stdin=hello"
    And the swarm status should be "completed"

  Scenario: Exactly one of code or path is accepted
    When I run swarm with both code and path
    Then the swarm result should be an error
    And the swarm result should contain "exactly one of 'code' or 'path'"

  Scenario: Neither code nor path is rejected
    When I run swarm with neither code nor path
    Then the swarm result should be an error
    And the swarm result should contain "exactly one of 'code' or 'path'"

  Scenario: Files written by Python persist for later turns
    When I run swarm inline code "open('artifact.txt','w').write('persisted')"
    And I run swarm inline code "print(open('artifact.txt').read())"
    Then the swarm result should contain "persisted"
    And the swarm status should be "completed"

  Scenario: Created files are reported as modified
    When I run swarm inline code "open('created.txt','w').write('x')"
    Then the swarm result should list "created.txt" as modified

  Scenario: Missing outside workspace script reports interpreter error
    When I run swarm file "../outside.py"
    Then the swarm result should be an error
    And the swarm result should contain "can't open file"

  Scenario: The artifact directory is reserved against script execution
    When I run swarm file ".quecto/swarm/planted.py"
    Then the swarm result should be a sandbox rejection

  Scenario: Runtime errors surface a non-zero exit and stderr
    When I run swarm inline code "raise ValueError('boom')"
    Then the swarm result should contain "ValueError"
    And the swarm result should contain "boom"
    And the swarm exit code should not be zero
    And the swarm result should be an error

  Scenario: Syntax errors surface a non-zero exit
    When I run swarm inline code "def ("
    Then the swarm result should contain "SyntaxError"
    And the swarm exit code should not be zero

  Scenario: Foreground execution enforces its timeout
    When I run swarm inline code "import time; time.sleep(30)" with timeout 1 seconds
    Then the swarm status should be "timed_out"
    And the swarm result should report cancel reason "timeout"

  Scenario: Oversized output is truncated and recoverable from an artifact
    When I run swarm inline code "print('x' * 5000)" with max output 256 bytes
    Then the swarm result should report truncated output
    And the swarm artifact should contain the full output

  Scenario: Arguments are passed without shell interpolation
    Given a swarm workspace file "show.py" with content:
      """
      import sys
      print("got:" + sys.argv[1])
      """
    When I run swarm file "show.py" with args "$(touch pwned.txt)" and stdin ""
    Then the swarm result should contain "got:$(touch pwned.txt)"
    And the swarm workspace should not contain "pwned.txt"

  Scenario: Clean successful run returns a slim envelope
    When I run swarm inline code "print('meta')"
    Then the swarm result should be a slim successful envelope

  Scenario: Status returns full metadata for a background execution
    When I run swarm inline code "print('meta')" in the background
    Then the swarm result should report a job id
    And the background swarm job should reach status "completed"
    And the swarm result should include audit metadata

  Scenario: Only an explicit minimal environment reaches the interpreter
    When I run swarm inline code "import os; print('PATH=' + os.environ.get('PATH','') + ' HOME=' + os.environ.get('HOME','absent'))"
    Then the swarm result should contain "PATH=/usr/local/bin:/usr/bin:/bin HOME=absent"

  Scenario: Background job starts, reports status, and streams output
    When I run swarm inline code "print('bg done')" in the background
    Then the swarm result should report a job id
    And the background swarm job should reach status "completed"
    And the background swarm output should contain "bg done"

  Scenario: Background job can be cancelled
    When I run swarm inline code "import os,pathlib,time; pathlib.Path('pid.txt').write_text(str(os.getpid())); time.sleep(30)" in the background
    Then the swarm result should report a job id
    And the background swarm process should be running
    When I cancel the background swarm job
    Then the swarm result should contain "cancelling"
    And the background swarm job should reach status "cancelled"
    And the cancelled swarm process should no longer be running

  Scenario: Concurrent background jobs are capped
    When I run swarm inline code "import time; time.sleep(30)" in the background
    And I run swarm inline code "import time; time.sleep(30)" in the background
    And I run swarm inline code "import time; time.sleep(30)" in the background
    Then the swarm result should contain "concurrent job limit reached"

  Scenario: Status for an unknown job id is reported as not found
    When I ask for swarm status of job "job_missing"
    Then the swarm result should contain "not_found"
    And the swarm result should be an error

  Scenario: Unknown operations are rejected
    When I run swarm op "explode"
    Then the swarm result should contain "unknown op explode"
    And the swarm result should be an error

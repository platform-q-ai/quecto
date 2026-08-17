@done
Feature: Python Lab Tool
  As an AI agent
  I want to execute Python in the persistent task workspace
  So that I can compute exactly, test hypotheses, and keep intermediate artifacts

  Background:
    Given a python lab workspace

  Scenario: Inline code executes and returns stdout
    When I run python lab inline code "print(sum(range(101)))"
    Then the python lab result should contain "5050"
    And the python lab status should be "completed"
    And the python lab result should not be an error

  Scenario: Saved workspace file executes with arguments and stdin
    Given a python lab workspace file "echo_args.py" with content:
      """
      import sys
      print("args=" + ",".join(sys.argv[1:]))
      print("stdin=" + sys.stdin.read().strip())
      """
    When I run python lab file "echo_args.py" with args "alpha,beta" and stdin "hello"
    Then the python lab result should contain "args=alpha,beta"
    And the python lab result should contain "stdin=hello"
    And the python lab status should be "completed"

  Scenario: Exactly one of code or path is accepted
    When I run python lab with both code and path
    Then the python lab result should be an error
    And the python lab result should contain "exactly one of 'code' or 'path'"

  Scenario: Neither code nor path is rejected
    When I run python lab with neither code nor path
    Then the python lab result should be an error
    And the python lab result should contain "exactly one of 'code' or 'path'"

  Scenario: Files written by Python persist for later turns
    When I run python lab inline code "open('artifact.txt','w').write('persisted')"
    And I run python lab inline code "print(open('artifact.txt').read())"
    Then the python lab result should contain "persisted"
    And the python lab status should be "completed"

  Scenario: Created files are reported as modified
    When I run python lab inline code "open('created.txt','w').write('x')"
    Then the python lab result should list "created.txt" as modified

  Scenario: Missing outside workspace script reports interpreter error
    When I run python lab file "../outside.py"
    Then the python lab result should be an error
    And the python lab result should contain "can't open file"

  Scenario: The artifact directory is reserved against script execution
    When I run python lab file ".quecto/python_lab/planted.py"
    Then the python lab result should be a sandbox rejection

  Scenario: Runtime errors surface a non-zero exit and stderr
    When I run python lab inline code "raise ValueError('boom')"
    Then the python lab result should contain "ValueError"
    And the python lab result should contain "boom"
    And the python lab exit code should not be zero
    And the python lab result should be an error

  Scenario: Syntax errors surface a non-zero exit
    When I run python lab inline code "def ("
    Then the python lab result should contain "SyntaxError"
    And the python lab exit code should not be zero

  Scenario: Foreground execution enforces its timeout
    When I run python lab inline code "import time; time.sleep(30)" with timeout 1 seconds
    Then the python lab status should be "timed_out"
    And the python lab result should report cancel reason "timeout"

  Scenario: Oversized output is truncated and recoverable from an artifact
    When I run python lab inline code "print('x' * 5000)" with max output 256 bytes
    Then the python lab result should report truncated output
    And the python lab artifact should contain the full output

  Scenario: Arguments are passed without shell interpolation
    Given a python lab workspace file "show.py" with content:
      """
      import sys
      print("got:" + sys.argv[1])
      """
    When I run python lab file "show.py" with args "$(touch pwned.txt)" and stdin ""
    Then the python lab result should contain "got:$(touch pwned.txt)"
    And the python lab workspace should not contain "pwned.txt"

  Scenario: Result carries audit metadata
    When I run python lab inline code "print('meta')"
    Then the python lab result should include audit metadata

  Scenario: Only an explicit minimal environment reaches the interpreter
    When I run python lab inline code "import os; print('PATH=' + os.environ.get('PATH','') + ' HOME=' + os.environ.get('HOME','absent'))"
    Then the python lab result should contain "PATH=/usr/local/bin:/usr/bin:/bin HOME=absent"

  Scenario: Background job starts, reports status, and streams output
    When I run python lab inline code "print('bg done')" in the background
    Then the python lab result should report a job id
    And the background python lab job should reach status "completed"
    And the background python lab output should contain "bg done"

  Scenario: Background job can be cancelled
    When I run python lab inline code "import os,pathlib,time; pathlib.Path('pid.txt').write_text(str(os.getpid())); time.sleep(30)" in the background
    Then the python lab result should report a job id
    And the background python lab process should be running
    When I cancel the background python lab job
    Then the python lab result should contain "cancelling"
    And the background python lab job should reach status "cancelled"
    And the cancelled python lab process should no longer be running

  Scenario: Concurrent background jobs are capped
    When I run python lab inline code "import time; time.sleep(30)" in the background
    And I run python lab inline code "import time; time.sleep(30)" in the background
    And I run python lab inline code "import time; time.sleep(30)" in the background
    Then the python lab result should contain "concurrent job limit reached"

  Scenario: Status for an unknown job id is reported as not found
    When I ask for python lab status of job "job_missing"
    Then the python lab result should contain "not_found"
    And the python lab result should be an error

  Scenario: Unknown operations are rejected
    When I run python lab op "explode"
    Then the python lab result should contain "unknown op explode"
    And the python lab result should be an error

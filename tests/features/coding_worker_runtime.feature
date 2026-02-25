@done
Feature: nsjail Coding Worker Runtime
  As the coding runtime coordinator
  I want to launch coding workers inside nsjail containers
  So that coding jobs execute with kernel-enforced isolation per job

  The coordinator launches a dedicated worker process per coding job inside
  nsjail. The worker communicates via JSON Lines over stdin/stdout. The
  coordinator manages the worker lifecycle: launch, monitor, timeout, kill.

  # --- Worker launch ---

  Scenario: Coordinator launches a worker process inside nsjail for a queued job
    Given a coding coordinator with nsjail available
    And a coding job in state "queued" for repo "test-repo" at base ref "main"
    When the coordinator begins preparation and the repo clone succeeds
    Then a worker process should be started inside nsjail
    And the worker should receive the job goal and config via stdin
    And the worker job state should become "running"

  Scenario: Worker process runs with job directory as sole writable mount
    Given a coding coordinator with nsjail available
    And a worker coding job in state "preparing" with job directory "jobs/job_000001/repo"
    When the worker is launched inside nsjail
    Then the nsjail mount table should include the job directory as read-write
    And the host root filesystem should be mounted read-only
    And no other directories should be writable

  Scenario: Worker process inherits nsjail resource limits from config
    Given a coding coordinator with config:
      | coding.isolation.resources.max_memory_mb   | 512 |
      | coding.isolation.resources.max_cpu_seconds  | 120 |
      | coding.isolation.resources.max_wall_seconds  | 300 |
      | coding.isolation.resources.max_pids          | 128 |
    When a worker is launched for a coding job
    Then the nsjail process should have memory limit 512 MB
    And the nsjail process should have CPU time limit 120 seconds
    And the nsjail process should have wall time limit 300 seconds
    And the nsjail process should have PID limit 128

  Scenario: Worker process runs with no_new_privs and seccomp-bpf
    Given a coding coordinator with nsjail available
    When a worker is launched for a coding job
    Then the nsjail process should have no_new_privs enabled
    And a seccomp-bpf profile should be applied

  # --- JSON Lines IPC ---

  Scenario: Worker sends structured events to coordinator via stdout
    Given a running coding worker inside nsjail
    When the worker emits a JSON Lines message on stdout
    Then the coordinator should parse it as an event envelope
    And the event should be validated against the coding contract

  Scenario: Coordinator sends commands to worker via stdin
    Given a running coding worker inside nsjail
    When the coordinator writes a JSON Lines command to the worker's stdin
    Then the worker should receive and process the command

  Scenario: Malformed JSON from worker is logged and skipped
    Given a running coding worker inside nsjail
    When the worker writes a non-JSON line to stdout
    Then the coordinator should log a warning about the malformed line
    And the coordinator should continue processing subsequent lines

  Scenario: Worker stderr is captured for diagnostics
    Given a running coding worker inside nsjail
    When the worker writes to stderr
    Then the coordinator should capture stderr output for the job's diagnostic log
    And stderr output should not be interpreted as event messages

  # --- Worker lifecycle ---

  Scenario: Coordinator detects worker exit with zero status
    Given a running coding worker inside nsjail
    When the worker process exits with status 0
    Then the coordinator should process any remaining stdout events
    And the job should transition based on the final event state

  Scenario: Coordinator detects worker exit with non-zero status
    Given a running coding worker inside nsjail
    When the worker process exits with status 1
    Then the coordinator should transition the job to "failed"
    And the worker error_code should be "worker_crash"
    And the diagnostic log should include the exit status

  Scenario: Coordinator kills worker on wall timeout
    Given a running coding worker inside nsjail
    And the job has max_wall_seconds 10
    When the worker has been running for more than 10 seconds
    Then the coordinator should kill the nsjail process
    And the job should transition to "canceled" with reason "wall_timeout"
    And a worker "job.cancel" event should be recorded

  Scenario: Coordinator kills worker when parent job is canceled
    Given a running coding worker inside nsjail
    When the parent job is canceled by the user
    Then the coordinator should send SIGTERM to the nsjail process
    And if the process does not exit within 5 seconds send SIGKILL
    And the job should reach terminal state "canceled"

  # --- Network isolation ---

  Scenario: Worker has no network access by default
    Given a coding coordinator with default network policy "deny"
    When a worker is launched for a coding job
    Then the nsjail process should have network access disabled
    And the worker should not be able to reach external hosts

  Scenario: Worker has allowlisted network access when configured
    Given a coding coordinator with network allowlist ["registry.npmjs.org", "github.com"]
    When a worker is launched for a coding job
    Then the nsjail process should allow egress to the listed hosts only

  # --- Environment isolation ---

  Scenario: Worker does not receive QUECTO-prefixed environment variables
    Given a coding coordinator with environment variable QUECTO_SECRET_KEY set
    When a worker is launched for a coding job
    Then the worker's environment should not contain QUECTO_SECRET_KEY
    And the worker's environment should not contain any QUECTO_ prefixed variables

  Scenario: Worker does not receive GitHub credentials
    Given a coding coordinator with GitHub API token configured
    When a worker is launched for a coding job
    Then the worker's environment should not contain GITHUB_TOKEN
    And the worker's environment should not contain GH_TOKEN

  Scenario: Worker receives minimal PATH and locale variables only
    Given a coding coordinator with nsjail available
    When a worker is launched for a coding job
    Then the worker's environment should include PATH
    And the worker's environment should include LANG or LC_ALL
    And the worker's total environment variable count should be small

  # --- Cleanup ---

  Scenario: nsjail sandbox is cleaned up after worker exits normally
    Given a running coding worker inside nsjail
    When the worker process exits normally
    Then no nsjail processes should remain for this job
    And no stale worker mount namespaces should remain

  Scenario: nsjail sandbox is cleaned up after worker is killed
    Given a running coding worker inside nsjail
    When the coordinator kills the worker due to timeout
    Then no nsjail processes should remain for this job
    And no stale cgroup entries should remain

  Scenario: Worker with die-with-parent is terminated when coordinator crashes
    Given a coding worker launched with die-with-parent enabled
    When the coordinator process is killed
    Then the nsjail worker process should also be terminated

  # --- Host toolchain access ---

  Scenario: Worker can access host toolchain read-only
    Given a running coding worker inside nsjail
    When the worker runs "which git" via exec
    Then the result should show a path under /usr/bin or similar
    And the worker should not be able to modify files in /usr/bin

  Scenario: Worker can run build tools from host toolchain
    Given a running coding worker inside nsjail
    When the worker runs "python3 --version" via exec
    Then the result should show the Python version
    And the command should succeed

  # --- Multiple concurrent workers ---

  Scenario: Coordinator launches multiple workers for parallel jobs
    Given a coding coordinator with max_parallel_jobs 3
    And 3 coding jobs in state "queued"
    When the coordinator begins preparation for all 3 jobs
    Then 3 separate nsjail worker processes should be running
    And each worker should have its own isolated job directory
    And workers should not be able to access each other's directories

  Scenario: Fourth job waits when parallel limit is reached
    Given a coding coordinator with max_parallel_jobs 2
    And 2 coding jobs are already running
    When a third coding job is submitted
    Then the third job should remain in state "queued"
    And the coordinator should launch it when a running job completes

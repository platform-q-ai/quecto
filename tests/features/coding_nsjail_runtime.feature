@done
Feature: Real NsjailWorkerRuntime
  As the coding coordinator
  I need a real WorkerRuntime implementation that spawns nsjail processes
  So that coding workers execute inside sandboxed containers with JSON Lines IPC

  The NsjailWorkerRuntime implements the WorkerRuntime trait using real
  tokio::process::Command spawning. It builds the nsjail command line,
  sets up stdin/stdout/stderr pipes, and manages the process lifecycle.

  # --- Command construction ---

  Scenario: Runtime builds nsjail command with quecto worker entrypoint
    Given a nsjail runtime with default config
    When the runtime builds nsjail args for a job
    Then the nsjail args should contain "--mode" followed by "o"
    And the nsjail args should contain "--" separator
    And the nsjail args after "--" should start with the quecto binary path
    And the nsjail args after "--" should contain "worker"

  Scenario: Runtime includes run-id and job-id in worker args
    Given a nsjail runtime with default config
    And a worker launch config with run_id "run-42" and job_id "job-99"
    When the runtime builds the full command for the job
    Then the worker args should contain "--run-id" followed by "run-42"
    And the worker args should contain "--job-id" followed by "job-99"

  Scenario: Runtime includes goal in worker args
    Given a nsjail runtime with default config
    And a worker launch config with goal "fix the flaky test in auth module"
    When the runtime builds the full command for the job
    Then the worker args should contain "--goal" followed by "fix the flaky test in auth module"

  Scenario: Runtime mounts job directory read-write
    Given a nsjail runtime with default config
    And a worker launch config with job_dir "/tmp/jobs/job-001/repo"
    When the runtime builds nsjail args for a job
    Then the nsjail args should contain "--bindmount" with the job directory

  Scenario: Runtime mounts host root read-only for toolchain access
    Given a nsjail runtime with default config
    When the runtime builds nsjail args for a job
    Then the nsjail args should contain "--bindmount_ro" with "/:/host"

  Scenario: Runtime sets resource limits from config
    Given a nsjail runtime with default config
    And a worker launch config with limits:
      | max_memory_mb    | 1024  |
      | max_cpu_seconds  | 180   |
      | max_wall_seconds | 600   |
      | max_pids         | 256   |
    When the runtime builds nsjail args for a job
    Then the nsjail args should include memory limit 1024
    And the nsjail args should include cpu time limit 180
    And the nsjail args should include wall time limit 600
    And the nsjail args should include pid limit 256

  Scenario: Runtime enables no_new_privs and seccomp
    Given a nsjail runtime with default config
    When the runtime builds nsjail args for a job
    Then the nsjail args should contain "--no_new_privs"
    And the nsjail args should contain "--seccomp_string"

  Scenario: Runtime disables network by default
    Given a nsjail runtime with default config
    And a worker launch config with no network hosts
    When the runtime builds nsjail args for a job
    Then the nsjail args should contain "--disable_clone_newnet"

  Scenario: Runtime allows network when hosts are specified
    Given a nsjail runtime with default config
    And a worker launch config with network hosts "github.com,registry.npmjs.org"
    When the runtime builds nsjail args for a job
    Then the nsjail args should not contain "--disable_clone_newnet"

  Scenario: Runtime sets die-with-parent when enabled
    Given a nsjail runtime with default config
    And a worker launch config with die_with_parent enabled
    When the runtime builds nsjail args for a job
    Then the nsjail args should contain "--die_with_parent"

  Scenario: Runtime omits die-with-parent when disabled
    Given a nsjail runtime with default config
    And a worker launch config with die_with_parent disabled
    When the runtime builds nsjail args for a job
    Then the nsjail args should not contain "--die_with_parent"

  # --- Environment construction ---

  Scenario: Runtime builds minimal worker environment
    Given a nsjail runtime with default config
    When the runtime builds worker env for a job
    Then the nsjail worker env should contain PATH
    And the nsjail worker env should contain LANG set to "C.UTF-8"
    And the nsjail worker env should contain HOME set to the job directory

  Scenario: Runtime blocks secret environment variables
    Given a nsjail runtime with default config
    When the runtime builds worker env for a job
    Then the nsjail worker env should not contain any QUECTO_ prefixed vars except QUECTO_ALLOWED_HOSTS
    And the nsjail worker env should not contain GITHUB_TOKEN
    And the nsjail worker env should not contain GH_TOKEN

  Scenario: Runtime includes allowed hosts in environment when specified
    Given a nsjail runtime with default config
    And a worker launch config with network hosts "github.com,registry.npmjs.org"
    When the runtime builds worker env for a job
    Then the nsjail worker env should contain QUECTO_ALLOWED_HOSTS with "github.com,registry.npmjs.org"

  # --- Quecto binary resolution ---

  Scenario: Runtime resolves the quecto binary from current executable
    Given a nsjail runtime with default config
    When the runtime resolves the quecto binary path
    Then the resolved path should be an absolute path
    And the resolved path should end with "quecto" or contain the test binary name

  # --- Run-id and job-id propagation ---

  Scenario: Runtime stores run-id and job-id for launched workers
    Given a nsjail runtime with default config
    And a worker launch config with run_id "run-abc" and job_id "job-def"
    When the runtime builds the full command for the job
    Then the nsjail runtime should track run_id "run-abc" for the launch
    And the nsjail runtime should track job_id "job-def" for the launch

  # --- Stderr capture limits ---

  Scenario: Runtime caps stderr capture at 1 MiB
    Given a nsjail runtime with default config
    When the runtime receives stderr data exceeding 1 MiB
    Then the captured stderr should be at most 1048576 bytes

  # --- Worker status tracking ---

  Scenario: Runtime reports unknown PID as killed
    Given a nsjail runtime with default config
    When the runtime checks status for an unknown PID 99999
    Then the nsjail runtime status should be killed with reason containing "unknown"

  Scenario: Runtime reports not alive for unknown PID
    Given a nsjail runtime with default config
    When the runtime checks if PID 99999 is alive
    Then the nsjail runtime should report not alive

  # --- Quiet mode ---

  Scenario: Runtime uses nsjail quiet mode
    Given a nsjail runtime with default config
    When the runtime builds nsjail args for a job
    Then the nsjail args should contain "--quiet"

  # --- Working directory ---

  Scenario: Runtime sets cwd to job directory inside nsjail
    Given a nsjail runtime with default config
    And a worker launch config with job_dir "/tmp/jobs/job-007/repo"
    When the runtime builds nsjail args for a job
    Then the nsjail args should contain "--cwd" followed by the job directory

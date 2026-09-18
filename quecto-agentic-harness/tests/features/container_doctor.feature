@container-doctor
Feature: Container failures are diagnosable
  As an agent (or operator) whose container spawn failed
  I want the failing script's own words in the tool error, a create preflight with one distinct
  message per missing prerequisite, and `quecto container doctor` to run that preflight for the
  effective container config of the working directory
  So that a missing image, runtime, tool or unreachable repository is named, never reduced to
  "failed with status 1"

  Background:
    Given script-managed subagent spawning is available from a checkout with global default script "default"

  @done @issue-2024 @container-spawn
  Scenario: A create script that fails puts its stderr tail in the spawn error
    Given the default container config's create script fails with status 3 after printing "image quecto-box:local is not present; build it first" on stderr
    When I spawn script-managed subagent "container-stderr" with default selection and no config argument and task "CONTAINER_STDERR_MARKER"
    Then the spawn result should fail with "script-managed create failed with status exit status: 3: "
    And the spawn result should fail with "image quecto-box:local is not present; build it first"
    And the spawn result should not contain a control character other than newline

  @done @issue-2024 @container-spawn
  Scenario: Only the tail of a long stderr stream is kept and terminal escapes are neutralised
    Given the default container config's create script fails after printing 10000 bytes of filler then an escape-coloured line "LAST_LINE_MARKER" on stderr
    When I spawn script-managed subagent "container-stderr-tail" with default selection and no config argument and task "CONTAINER_STDERR_TAIL_MARKER"
    Then the spawn result should fail with "LAST_LINE_MARKER"
    And the spawn result should not contain a control character other than newline
    And the spawn result should be shorter than 6000 bytes

  @done @issue-2024 @container-spawn
  Scenario: The official create script's preflight names a missing image without pulling it
    Given a controlled PATH whose fake podman reports every image as missing
    When I run the official docker create script with --preflight-only for image "quecto-box:local" and a reachable local repository
    Then the preflight should exit with a non-zero status
    And the preflight should report check "image" as failed naming "quecto-box:local"
    And the preflight should report a remedy for check "image" mentioning "podman build"
    And the preflight should report check "runtime-cli" as passed
    And the preflight should report check "repo" as passed
    And the preflight should report check "state-dir" as passed
    And the fake podman should never have been asked to pull
    And the preflight should have created no state directory

  @done @issue-2024 @container-spawn
  Scenario: The official create script's preflight names an unreachable repository
    Given a controlled PATH whose fake podman reports every image as present
    When I run the official docker create script with --preflight-only for image "quecto-box:local" and an unreachable repository
    Then the preflight should exit with a non-zero status
    And the preflight should report check "repo" as failed naming the unreachable repository
    And the preflight should report check "image" as passed

  @done @issue-2024 @container-spawn
  Scenario: A runtime that cannot answer is reported as such, never as a missing image
    Given a controlled PATH whose fake podman answers every command with "Error: cannot connect to Podman socket: permission denied" and exit 1
    When I run the official docker create script with --preflight-only for image "quecto-box:local" and a reachable local repository
    Then the preflight should exit with a non-zero status
    And the preflight should report check "image" as failed naming "could not look up image quecto-box:local"
    And the preflight should report check "image" as failed naming "permission denied"
    And the preflight should report a remedy for check "image" mentioning "podman info"

  @done @issue-2024 @container-spawn
  Scenario: Docker's own missing-image answer is a missing image
    Given a controlled PATH whose fake docker answers every command with "Error: No such image: quecto-box:local" and exit 1
    When I run the official docker create script with --preflight-only for image "quecto-box:local" and a reachable local repository
    Then the preflight should exit with a non-zero status
    And the preflight should report check "runtime-cli" as passed
    And the preflight should report check "image" as failed naming "not present in the local docker store"
    And the preflight should report a remedy for check "image" mentioning "docker build"

  @done @issue-2024 @container-spawn
  Scenario: A --repo URL's embedded credentials never reach a preflight line
    Given a controlled PATH whose fake podman reports every image as present
    When I run the official docker create script with --preflight-only for image "quecto-box:local" and repository "https://user:ghp_secret@127.0.0.1:1/x/y"
    Then the preflight should exit with a non-zero status
    And the preflight should report check "repo" as failed naming "--repo https://***@127.0.0.1:1/x/y is unreachable"
    And the output should not contain "ghp_secret"

  @done @issue-2024 @container-spawn
  Scenario: A --repo URL's embedded credentials never reach the spawn error
    Given the default container config's create script fails with status 7 after printing "--repo https://user:ghp_secret@host/x/y is unreachable: fatal: could not read Username" on stderr
    When I spawn script-managed subagent "container-redacted" with default selection and no config argument and task "CONTAINER_REDACTED_MARKER"
    Then the spawn result should fail with "--repo https://***@host/x/y is unreachable: fatal: could not read Username"
    And the spawn result should not contain "ghp_secret"

  @done @issue-2024 @container-spawn
  Scenario: The host-local reference create script's preflight names an unreachable repository
    Given a controlled PATH whose fake podman reports every image as present
    When I run the host-local reference create script with --preflight-only and an unreachable repository
    Then the preflight should exit with a non-zero status
    And the preflight should report check "repo" as failed naming the unreachable repository
    And the preflight should report check "jq" as passed

  @done @issue-2024 @container-spawn
  Scenario: quecto container doctor in a bound checkout reports a missing runtime and exits non-zero
    Given the checkout binds itself to the official docker create script with a reachable local repository under a controlled PATH without any container runtime
    When I run quecto with arguments "container doctor"
    Then the exit code should be 1
    And the doctor output should show check "runtime-cli" as failed with a remedy mentioning "podman"
    And the doctor output should name container config "official"

  @done @issue-2024 @container-spawn
  Scenario: quecto container doctor reports a missing image and passes once the image exists
    Given the checkout binds itself to the official docker create script with a reachable local repository under a controlled PATH whose fake podman reports every image as missing
    When I run quecto with arguments "container doctor"
    Then the exit code should be 1
    And the doctor output should show check "image" as failed with a remedy mentioning "podman build"
    When the fake podman is fixed to report every image as present
    And I run quecto with arguments "container doctor"
    Then the exit code should be 0
    And the doctor output should show check "image" as passed
    And the doctor output should show no failed check

  @done @issue-2024 @container-spawn
  Scenario: quecto container doctor names a config explicitly and refuses an unknown one
    Given the checkout binds itself to the official docker create script with a reachable local repository under a controlled PATH whose fake podman reports every image as present
    When I run quecto with arguments "container doctor --name official"
    Then the exit code should be 0
    When I run quecto with arguments "container doctor --name nope"
    Then the exit code should be 1
    And stderr should contain "unknown container config 'nope'"

  @done @issue-2024 @container-spawn
  Scenario: quecto container doctor never prints the credentials embedded in a config's --repo
    Given the checkout binds itself to the official docker create script with repository "https://user:ghp_secret@127.0.0.1:1/x/y" under a controlled PATH whose fake podman reports every image as present
    When I run quecto with arguments "container doctor"
    Then the exit code should be 1
    And the output should contain "(create: bash "
    And the output should contain "--repo https://***@127.0.0.1:1/x/y --image quecto-box:local)"
    And the doctor output should show check "repo" as failed with a remedy mentioning "check the URL"
    And the output should not contain "ghp_secret"

  @done @issue-2024 @container-spawn
  Scenario: quecto container doctor fails closed when the real create script dies after its first checks
    Given the checkout binds itself to the official docker create script with a reachable local repository under a controlled PATH whose fake podman reports every image as present
    And the bound create script runs with QUECTO_REPO_CHECK_TIMEOUT set to "abc"
    When I run quecto with arguments "container doctor"
    Then the exit code should be 1
    And stderr should contain "exited exit status: 2 after 4 checks without reporting a failure"
    And stderr should contain "QUECTO_REPO_CHECK_TIMEOUT must be a positive integer (seconds)"
    And the output should not contain "checks failed"

  @done @issue-2024 @container-spawn
  Scenario: quecto container doctor fails closed when a create script exits non-zero after passing checks
    Given the checkout binds itself to a create script that prints 4 passing checks then exits 2 with usage text "usage: fake-create --state-dir <dir> [--repo <url>]"
    When I run quecto with arguments "container doctor"
    Then the exit code should be 1
    And stderr should contain "exited exit status: 2 after 4 checks without reporting a failure: usage: fake-create --state-dir <dir> [--repo <url>]"
    And the output should not contain "checks failed"

  @done @issue-2024 @container-spawn
  Scenario: quecto container doctor refuses a malformed check line instead of skipping it
    Given the checkout binds itself to a create script whose preflight prints a cut-short "fail" line for check "image"
    When I run quecto with arguments "container doctor"
    Then the exit code should be 1
    And stderr should contain "printed a malformed check line"
    And stderr should contain "expected `ok|warn|fail<TAB>check<TAB>detail[<TAB>remedy]`"
    And the output should not contain "checks failed"

  @done @issue-2024 @container-spawn
  Scenario: quecto container doctor refuses to diagnose over an untrusted overlay in its own words
    Given the checkout carries an untrusted overlay binding container config "r" with repository "https://example.test/repo-r"
    When I run quecto with arguments "container doctor"
    Then the exit code should be 1
    And stderr should contain "container doctor refused: the checkout's repo-local config overlay was not applied"
    And stderr should contain "quecto config trust"
    And the output should not contain "to launch"

  @done @issue-2024 @container-spawn
  Scenario: quecto container doctor reports a create script without preflight support
    Given the checkout binds itself to a create script that rejects --preflight-only
    When I run quecto with arguments "container doctor"
    Then the exit code should be 1
    And stderr should contain "does not support --preflight-only"

  @done @issue-2024 @container-spawn @serial
  Scenario: A real quecto agent's container spawn error names the missing image
    Given the checkout binds itself to the official docker create script with a reachable local repository under a controlled PATH whose fake podman reports every image as missing
    When a real quecto agent started in the checkout is driven by a fake provider to spawn container true
    Then the real agent should have exited successfully
    And the tool result the fake provider received should be a spawn error naming "quecto-box:local"

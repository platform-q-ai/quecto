@standard-container
Feature: The standard container is landed on master
  As an operator (or an agent) setting up containers for a repository
  I want `quecto container init` to materialise the standard Containerfile and rootless-Podman
  runtime scripts under the checkout and to bind the repository to them through the trusted
  `.quecto/config.json` overlay, and `quecto container status` to say where that stands
  So that the only manual step left is building the image with the command init prints

  Background:
    Given a config file at "~/.quecto/config.json" with content:
      """
      {}
      """

  @done @issue-2024
  Scenario: init materialises the assets, binds the checkout as the default and prints the build command
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    And the standard container assets should be materialised under ".quecto/containers/standard" byte-identical to the official adapter
    And the output should list every materialised standard container asset
    And the checkout's overlay should be trusted
    And the overlay entry "standard" should be the default and its create argv should be the materialised create script with "--state-dir" under the base directory, "--repo" the origin remote and "--image" "quecto-box:local"
    And the overlay entry "standard" should carry exec, inspect, kill and cleanup argv naming the materialised scripts
    And no argv of the overlay entry "standard" should end with "--"
    And the output should contain "podman build -t quecto-box:local -f"
    And the output should contain "quecto container doctor"

  @done @issue-2024
  Scenario: init twice is idempotent
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    And I remember the checkout's overlay bytes
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    And the output should contain "no files changed"
    And the checkout's overlay bytes should be unchanged
    And the overlay entry "standard" should be the default and its create argv should be the materialised create script with "--state-dir" under the base directory, "--repo" the origin remote and "--image" "quecto-box:local"

  @done @issue-2024
  Scenario: a re-init keeps the existing --repo and --image unless the flag is given, and says which
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    When I run quecto with arguments "container init --repo https://example.test/chosen --image mine:1"
    Then the exit code should be 0
    And the output should not contain "kept:"
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    And the overlay entry "standard" create argv should carry "--repo" "https://example.test/chosen"
    And the overlay entry "standard" create argv should carry "--image" "mine:1"
    And the output should contain "kept:    --repo https://example.test/chosen (the existing entry's; pass --repo to change it)"
    And the output should contain "kept:    --image mine:1 (the existing entry's; pass --image to change it)"
    And the output should not contain "rewrote:"
    When I run quecto with arguments "container init --image mine:2"
    Then the exit code should be 0
    And the overlay entry "standard" create argv should carry "--repo" "https://example.test/chosen"
    And the overlay entry "standard" create argv should carry "--image" "mine:2"
    And the output should contain "kept:    --repo https://example.test/chosen"
    And the output should contain "rewrote: --image mine:2 (was mine:1)"
    And the output should contain "podman build -t mine:2 -f"

  @done @issue-2024
  Scenario: init beside an existing default adds the entry without the default label and says so
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And the global configuration already labels container config "other" as the default
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    And the overlay entry "standard" should not be the default
    And the effective default container config should still be "other"
    And the output should contain "not the default: other is already the default"

  @done @issue-2024
  Scenario: init without an origin remote writes a sandbox entry and says so
    Given the current directory is a git checkout without an origin remote
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    And the overlay entry "standard" create argv should carry no "--repo"
    And the output should contain "sandbox"
    And the output should contain "--repo"

  @done @issue-2024
  Scenario: an explicit --repo wins over the origin remote
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    When I run quecto with arguments "container init --repo https://example.test/explicit"
    Then the exit code should be 0
    And the overlay entry "standard" create argv should carry "--repo" "https://example.test/explicit"

  @done @issue-2024
  Scenario: --dry-run writes nothing
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    When I run quecto with arguments "container init --dry-run"
    Then the exit code should be 0
    And the output should contain "would write"
    And the output should not contain "trusted for exactly these bytes"
    And no standard container asset should exist under ".quecto/containers/standard"
    And the checkout should carry no overlay

  @done @issue-2024
  Scenario: init refuses an untrusted overlay instead of trusting it silently
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And the checkout carries an untrusted overlay declaring container config "r"
    When I run quecto with arguments "container init"
    Then the exit code should be 1
    And the stderr should contain "is not trusted"
    And the stderr should contain "quecto config trust"
    And no standard container asset should exist under ".quecto/containers/standard"

  @done @issue-2024
  Scenario: an untrusted overlay that declares no container config is refused before any asset is written, on a dry run too
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And the checkout carries an untrusted overlay pinning only the default model "m"
    When I run quecto with arguments "container init --dry-run"
    Then the exit code should be 1
    And the stderr should contain "is not trusted"
    When I run quecto with arguments "container init"
    Then the exit code should be 1
    And the stderr should contain "is not trusted"
    And no standard container asset should exist under ".quecto/containers/standard"

  @done @issue-2024
  Scenario: an origin remote that embeds a password is refused rather than written into the overlay
    Given the current directory is a git checkout whose origin remote is "https://user:ghp_secret@example.test/x/y"
    When I run quecto with arguments "container init"
    Then the exit code should be 1
    And the stderr should contain "checkout's origin remote https://***@example.test/x/y carries a credential"
    And the output should not contain "ghp_secret"
    And the checkout should carry no overlay
    And no standard container asset should exist under ".quecto/containers/standard"
    When I run quecto with arguments "container init --repo https://user:ghp_other@example.test/x/y"
    Then the exit code should be 1
    And the stderr should contain "the --repo URL https://***@example.test/x/y carries a credential"
    And the output should not contain "ghp_other"

  @done @issue-2024
  Scenario: a token as the whole userinfo of an https URL is a credential and is refused from --repo and origin alike
    Given the current directory is a git checkout whose origin remote is "https://ghp_ORIGINSECRET@github.com/org/repo.git"
    When I run quecto with arguments "container init --repo https://ghp_SECRET@github.com/org/repo.git"
    Then the exit code should be 1
    And the stderr should contain "the --repo URL https://***@github.com/org/repo.git carries a credential"
    And the output should not contain "ghp_SECRET"
    And the checkout should carry no overlay
    And no standard container asset should exist under ".quecto/containers/standard"
    And "ghp_SECRET" should appear in no file under the checkout's ".quecto"
    When I run quecto with arguments "container init"
    Then the exit code should be 1
    And the stderr should contain "checkout's origin remote https://***@github.com/org/repo.git carries a credential"
    And the output should not contain "ghp_ORIGINSECRET"
    And the checkout should carry no overlay
    And no standard container asset should exist under ".quecto/containers/standard"
    And "ghp_ORIGINSECRET" should appear in no file under the checkout's ".quecto"

  @done @issue-2024
  Scenario: an ssh origin with a bare user is a repository, not a credential
    Given the current directory is a git checkout whose origin remote is "ssh://git@example.test/org/repo.git"
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    And the overlay entry "standard" create argv should carry "--repo" "ssh://git@example.test/org/repo.git"

  @done @issue-2024
  Scenario: a symbolic link in the bundle's place is refused by dry-run, init and status alike, before anything is written
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And the checkout's ".quecto/containers" is a symbolic link to a directory outside the checkout
    When I run quecto with arguments "container init --dry-run"
    Then the exit code should be 1
    And the stderr should contain "is a symbolic link"
    When I run quecto with arguments "container init"
    Then the exit code should be 1
    And the stderr should contain "is a symbolic link"
    And the stderr should not contain "already written"
    And the checkout should carry no overlay
    And the directory outside the checkout should still be empty
    When I run quecto with arguments "container status"
    Then the exit code should be 1
    And the output should contain "refused"
    And the output should contain "is a symbolic link"

  @done @issue-2024
  Scenario: the printed build command quotes a project path with a space
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And the checkout has a subdirectory "my projects/repo one"
    And "my projects/repo one" below the checkout is its own git checkout without an origin remote
    When I run quecto with arguments "container init --project '<checkout>/my projects/repo one'" where <checkout> is the checkout
    Then the exit code should be 0
    And the output should contain the build command with the bundle directory single-quoted

  @done @issue-2024
  Scenario: init from a subdirectory of the checkout is refused naming the root
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And the checkout has a subdirectory "crates/inner"
    When I run quecto with arguments "container init --project <checkout>/crates/inner" where <checkout> is the checkout
    Then the exit code should be 1
    And the stderr should contain "/crates/inner is not the repository root"
    And the stderr should contain "pass --project "
    And the stderr should name the checkout as the repository root
    And the checkout should carry no overlay
    And no standard container asset should exist under "crates/inner/.quecto/containers/standard"
    When I run quecto with arguments "container init --project <checkout>" where <checkout> is the checkout
    Then the exit code should be 0

  @done @issue-2024
  Scenario: init refuses to run under an explicit --config
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    When I run quecto with arguments "--config /dev/null container init"
    Then the exit code should be 1
    And the stderr should contain "run without --config"

  @done @issue-2024
  Scenario: init beside an existing default tells the agent how to select the entry and diagnose it by name
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And the global configuration already labels container config "other" as the default
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    And the output should contain "select it with container: {"
    And the output should contain "container_config"
    And the output should contain "quecto container doctor --name standard"

  @done @issue-2024
  Scenario: status before init says what is missing
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    When I run quecto with arguments "container status"
    Then the exit code should be 1
    And the output should contain "assets:  missing"
    And the output should contain "config:  standard entry missing"
    And the output should contain "quecto container init"

  @done @issue-2024
  Scenario: status after init reports the assets, the entry, the trust and the image through the preflight
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And a controlled PATH whose fake podman reports every image as missing
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    When I run the real quecto binary under the controlled PATH with arguments "container status"
    Then the exit code should be 1
    And the output should contain "assets:  present (5 of 5, version 2)"
    And the output should contain "config:  standard (default, overlay) in"
    And the output should contain "trust:   trusted"
    And the output should contain "image:   ✗ image quecto-box:local is not present"
    And the output should contain "podman build"
    When the fake podman is fixed to report every image as present
    And I run the real quecto binary under the controlled PATH with arguments "container status"
    Then the exit code should be 0
    And the output should contain "image:   image quecto-box:local is present"

  @done @issue-2024
  Scenario: the materialised create script honours the preflight contract and never pulls
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And a controlled PATH whose fake podman reports every image as missing
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    When I run the materialised standard create script with --preflight-only under the controlled PATH
    Then the preflight should exit with a non-zero status
    And the preflight should report check "runtime-cli" as passed
    And the preflight should report check "jq" as passed
    And the preflight should report check "git" as passed
    And the preflight should report check "repo" as passed
    And the preflight should report check "state-dir" as passed
    And the preflight should report check "image" as failed naming "quecto-box:local"
    And the fake podman should never have been asked to pull
    And the preflight should have created no state directory

  @done @issue-2024
  Scenario: quecto container doctor diagnoses the entry init wrote
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And a controlled PATH whose fake podman reports every image as present
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    When I run the real quecto binary under the controlled PATH with arguments "container doctor"
    Then the exit code should be 0
    And the doctor output should name container config "standard"
    And the doctor output should show check "image" as passed
    And the doctor output should show no failed check

  @done @issue-2024
  Scenario: a create through the materialised script runs the container with the swarm identity and never pulls
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And a controlled PATH whose fake podman reports every image as present and accepts every run
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    When I run the materialised standard create script for a fake child under the controlled PATH
    Then the create should have succeeded reporting the clone as its checkout
    And the fake podman run argv should carry "--pull=never"
    And the fake podman run argv should carry "--userns=keep-id"
    And the fake podman run argv should carry "-e QUECTO_SWARM_CONTAINER=isolated-pid-v1"
    And the fake podman run argv should carry "-e QUECTO_SWARM_BOOTSTRAP=1"
    And the fake podman run argv should carry "-e QUECTO_SWARM_CHECKOUT="
    And the fake podman run argv should carry "-e QUECTO_SWARM_HOST_PID_NS="
    And the fake podman run argv should carry "quecto-box:local"
    And the fake podman should never have been asked to pull an image

  @done @issue-2024 @container-spawn
  Scenario: an edited standard script refuses the launch before the host runs it, and init --refresh restores it
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And a controlled PATH whose fake podman reports every image as present
    And script-managed subagent spawning is available from the checkout with no global container config
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    When the materialised standard create script is edited to record every invocation
    And I spawn script-managed subagent "standard-tampered" with default selection and no config argument and task "STANDARD_TAMPERED_MARKER"
    Then the spawn result should fail with "container config 'standard' refused: "
    And the spawn result should name the materialised "scripts/create.sh" as differing from the standard bundle
    And the spawn result should fail with "quecto container init --refresh"
    And the materialised standard create script should never have been invoked
    When I run the real quecto binary under the controlled PATH with arguments "container status"
    Then the exit code should be 1
    And the output should contain "assets:  5 of 5 present, 1 differ from the embedded version 2"
    And the output should contain "differs "
    And the output should contain "image:   unknown — container config 'standard' refused: "
    And the materialised standard create script should never have been invoked
    When I run the real quecto binary under the controlled PATH with arguments "container doctor"
    Then the exit code should be 1
    And the stderr should contain "differs from the standard bundle this quecto embeds"
    And the materialised standard create script should never have been invoked
    When I run quecto with arguments "container init --refresh"
    Then the exit code should be 0
    And the output should contain "refreshed"
    And the standard container assets should be materialised under ".quecto/containers/standard" byte-identical to the official adapter
    When I run the real quecto binary under the controlled PATH with arguments "container doctor"
    Then the exit code should be 0
    And the doctor output should name container config "standard"
    And the doctor output should show no failed check
    When I run the real quecto binary under the controlled PATH with arguments "container status"
    Then the exit code should be 0
    And the output should contain "assets:  present (5 of 5, version 2)"

  # Review round 2: the retained argv (exec, inspect, kill, cleanup) is
  # judged like the create's before the host runs it. A kill.sh edited after
  # the create never runs; the environment stays retryable until the bundle
  # is restored.
  @done @issue-2024 @container-spawn @serial
  Scenario: an edited retained kill script refuses kill_container before the host runs it, and init --refresh restores it
    Given the current directory is a git checkout whose origin remote is a reachable local repository
    And a controlled PATH whose fake podman reports every image as present and runs each container's command on the host
    And script-managed subagent spawning through the fake podman is available from the checkout with no global container config
    When I run quecto with arguments "container init"
    Then the exit code should be 0
    When I spawn script-managed subagent "standard-live" with default selection and no config argument and task "STANDARD_LIVE_MARKER"
    Then the spawn result should not be an error
    And the fake podman should have been asked to run the child once
    When the materialised standard kill script is edited to record every invocation
    And I kill container "C1"
    Then the container command result should be an error mentioning "cleanup failed: retained kill refused: "
    And the container command result should name the materialised "scripts/kill.sh" as differing from the standard bundle
    And the container command result should be an error mentioning "quecto container init --refresh"
    And the container listing should include "C1" with status "cleanup-failed" and a last error
    And the materialised standard kill script should never have been invoked
    When I run quecto with arguments "container init --refresh"
    Then the exit code should be 0
    And the output should contain "refreshed"
    When I kill container "C1"
    Then the container command result should not be an error
    And the container listing should include "C1" with status "stopped" and 0 members
    And the fake podman should have been asked to remove the container
    And scenario teardown should leave no fixture processes running

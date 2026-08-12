Feature: Multi-session shared-state hardening (#1460, epic #1467)
  As an operator running N replicant agents against one XDG runtime dir,
  one credentials file and one session store
  I want cross-process shared state to be liveness-probed, locked and single-writer
  So that starting a new agent never severs a live one, concurrent token
  refreshes lose no credentials, and two processes can never silently
  interleave writes to one session key

  # ─── Socket reaping is decided by liveness, not mtime ─────────────────────
  # A socket file's mtime is fixed at bind time, so any agent older than the
  # age threshold looks "stale" to an mtime heuristic while it is still
  # serving. Reaping must probe the socket instead.

  @done @issue-1460
  Scenario: A live agent socket survives reaping regardless of its age
    Given a live quecto agent socket in a runtime directory
    When the stale socket reaper runs treating every socket as past the stale age
    Then the live agent socket file still exists

  @done @issue-1460
  Scenario: A dead agent socket file is reaped even when its mtime is fresh
    Given a live quecto agent socket in a runtime directory
    And a dead quecto agent socket file in the runtime directory
    When the stale socket reaper runs with every socket well within the stale age
    Then the dead agent socket file has been removed
    And the live agent socket file still exists

  # ─── credentials.json single-writer locking ───────────────────────────────
  # N agents refreshing a rotating token race the whole-file load-mutate-store
  # cycle; a cross-process lock file serializes the writers.

  @done @issue-1460
  Scenario: A credential write blocks while the credentials lock is held elsewhere
    Given a credential store whose credentials lock is held by another process
    When another process attempts a credential write for provider "alpha"
    Then the credential write has not completed

  @done @issue-1460
  Scenario: A blocked credential write completes once the credentials lock is released
    Given a credential write for provider "alpha" is blocked by the credentials lock
    When the credentials lock is released
    Then the credential write completes
    And provider "alpha" is stored
    And the previously stored credentials are still present

  # ─── Session-key single-writer ownership ──────────────────────────────────
  # Two processes on one session key silently lose turns today; ownership is
  # claimed via a pid stamp and refused while the owner is alive.

  @done @issue-1460
  Scenario: Claiming a session key owned by a live process is refused
    Given session key "shared-key" is owned by another live process
    When a second process claims ownership of session key "shared-key"
    Then the ownership claim is refused with an error naming the key and owning process

  @done @issue-1460
  Scenario: A session key stamped by a dead process is reclaimed
    Given session key "stale-key" is stamped as owned by a dead process
    When a second process claims ownership of session key "stale-key"
    Then the ownership claim succeeds
    And the ownership stamp records the new owner

  @done @issue-1460
  Scenario: Saving a session whose key is owned by a live agent is refused
    Given a session store whose key "owned-key" is stamped by another live process
    When this process saves a turn for session key "owned-key"
    Then the session save is refused with an error naming the key and owning process

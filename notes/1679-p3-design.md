# #1679 P3 design decisions

- Uncertainty is group-scoped quarantine in the domain: an abandoned active
  attempt stays `Active` (retains capacity) and is listed as uncertain; `next`
  refuses grants for the group until the same scope completes it (verified
  termination) or an operator reset starts a successor epoch.
- Orphans: after authority restart the ledger's outstanding attempts become
  orphaned occupancy without a live scope. They count as active+uncertain and are
  cleared only by reset. No TTL reclaim.
- Journal-before-grant: the authority selects a candidate, writes the ledger
  (file + directory fsync), and only then replies. A failed write withdraws the
  grant (terminal, never dispatched, pacing charge kept) and refuses further grants
  until a write succeeds.
- Reset: successor epoch carries cooldown/pacing deadlines, clears scopes,
  requests, orphans and `unavailable`; every connected client is disconnected and
  must re-register. Old-epoch operations fail with `StaleEpoch`.
- Lineage: roots register on the private socket (same-UID trust); the authority
  issues a random capability per scope. Children are registered by the parent
  (`register_child` with the parent capability) before launch; the child receives
  endpoint/epoch/scope/capability through a 0600 sidecar file, never argv/env.
- Protocol: versioned framed JSON, one connection per scope, `id`-correlated
  replies; queued acquire replies when granted/failed; server-initiated
  `cancel_required` notices; connection close ⇒ abandon.
- Containers: `QUECTO_ADMISSION_DIR` is passed to create/exec scripts; the docker
  adapter mounts the directory at the same path and reports
  `admission_capability: "shared-directory-v1"`. Enabled launches require it.

# Folder-aware sessions

## Global transcript authority

Session transcript JSON remains in the global session store. Folder scope is versioned, optional metadata beside the opaque session identity; paths never become IDs and transcripts are never moved into per-folder stores. Exact transcript records remain the recovery authority.

## Local and global discovery

`/resume` opens Local discovery by default. Local scope uses canonical execution locations: the nearest containing Git repository (nested repositories win), Git-reported common-directory relationships for related worktrees, or exact canonical directory identity outside Git. Rows show the actual execution directory.

The visible `Local | Global` control is keyboard- and mouse-selectable. Global search covers title, opaque key, repository facts, and path. `Ctrl+G` remains the global jump-to-latest conversation shortcut; it is not assigned to session scope.

## Exact-key compatibility

`/resume <key>` performs exact global lookup regardless of the current directory or picker scope. Catalogue absence, corruption, staleness, or a malformed unrelated record must not make exact-key resume unavailable.

## Safe cross-folder actions

A session from another available folder offers **Open original folder**, **Fork into current folder**, and **Cancel**. Open launches a fresh process rooted at the target so configuration, tools, permissions, and working directory reload. It never changes the current process directory. Fork creates a new opaque identity and copies safe conversation content only, excluding ownership, live children, pending work, workflow/runtime state, and tool/config snapshots.

A missing or moved folder offers **Locate**, **Fork**, and **Cancel**. Locate validates the selected canonical folder and explicitly reassociates it. No flow implicitly creates a directory, performs a checkout, or guesses repository identity.

## Legacy unscoped sessions

Readers accept records without scope metadata. Such records remain visible globally and require explicit association; the application does not infer a home folder.

## Recovery and diagnostics

The catalogue is a derived projection rebuilt from authoritative global records. Operators recover by discarding the derived projection and listing authoritative records again; rebuild validates versions, skips an unreadable record with a visible diagnostic, and atomically publishes the replacement. Interrupted resume writes use the `resume-intent` marker: startup recovery completes an existing staged rename or removes the incomplete intent. A rebuild or migration failure leaves transcript files and exact-key lookup intact.

Actionable failures name the failed operation: unavailable target folder, canonicalization/Git discovery failure, ownership claim conflict, durable save failure, catalogue record skipped, or fresh runtime launch failure. Failures roll back the destination claim/staging and leave the active runtime usable.

## Architecture boundaries

Domain scope and disposition values perform no filesystem, Git, process, terminal, or UI effects. Application session use cases own planning and atomic resume transactions through affirmative capability ports. Infrastructure implements canonicalization, Git discovery, persistence, ownership, and fresh launch. Interface and TUI code parse, map, delegate, and present explicit choices.

## Retirement audit

The following unsafe mechanisms are intentionally absent and guarded by architecture tests:

- in-process cross-folder `chdir`;
- silent resume-in-current-folder behavior;
- per-folder transcript stores or path-derived session IDs;
- interface-owned filesystem/process orchestration;
- reassignment of `Ctrl+G`.

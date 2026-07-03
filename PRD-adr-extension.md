# PRD: `adr` — Shared ADR Repository Extension for Quecto (MVP)

**Status:** Draft · **Type:** UDS tool extension · **Target:** single-machine multi-agent teams

## 1. Problem

Quecto agents collaborating on long-running work (a software platform, an ML research program) repeatedly lose or contradict prior decisions. Decisions live in dead context: chat history, spilled sessions, ad-hoc files. There is no canonical, queryable record of *what was decided, why, and what superseded it* that all agents (parents and spawned subagents) can read and write.

## 2. Goal

A single UDS extension process exposing a shared, append-oriented repository of Architecture Decision Records that any connected Quecto agent can search, read, and contribute to — so decisions made by one agent in one session bind and inform all others.

**MVP success criteria**
- An agent can record a decision in <1 tool call and any other agent can find it by keyword in <1 tool call.
- Records survive agent restarts and are human-readable on disk.
- A decision can be superseded without losing history.

## 3. Non-Goals (MVP)

- No vector/semantic search (keyword FTS only).
- No multi-host sync, auth, or access control (trusted local agents).
- No approval workflows, voting, or review states beyond the status field.
- No editing of accepted records' bodies (supersede instead).
- No web UI (files on disk are the UI).

## 4. Users

- **Agents** (primary): parent agents and subagents on the same host, connected via the agent's Unix socket.
- **Humans** (secondary): read/audit the ADR directory directly; it is plain Markdown in git.

## 5. Design Overview

- **Form:** standalone binary (`quecto-adr <socket-path> [--dir <repo>]`) that connects to a Quecto agent socket and registers tools via `register_tools`. One process per agent socket; all processes point at the same `--dir`, which is the shared store.
- **Storage:** a directory of Markdown files with YAML frontmatter (`adr/0007-use-postgres.md`), plus a derived SQLite index (frontmatter fields + FTS5 over title/body/tags) rebuilt on startup and updated on writes. Files are the source of truth; the index is disposable. Directory is expected to be inside a git repo (the extension does not commit; agents/humans do).
- **Concurrency:** file lock on the directory for ID allocation and writes; last-writer-wins is avoided by ADRs being append/supersede-only.

### ADR record

```yaml
id: 7                # monotonically allocated
title: Use Postgres for metadata store
status: proposed | accepted | rejected | superseded
tags: [storage, platform]
author: <agent_id or free string>
created: <ISO 8601>
supersedes: [3]      # optional
superseded_by: 12    # set automatically
---
## Context
## Decision
## Consequences
```

## 6. Tool Surface (registered tools)

| Tool | Params | Returns |
|---|---|---|
| `adr_create` | `title`, `context`, `decision`, `consequences?`, `tags?`, `status?` (default `proposed`), `supersedes?` | new `id` + file path |
| `adr_get` | `id` | full record (frontmatter + body) |
| `adr_search` | `query?` (FTS), `tags?`, `status?`, `limit?` (default 10) | list of `{id, title, status, tags, created, snippet}` |
| `adr_list` | `status?`, `limit?`, `since?` | compact index, newest first |
| `adr_set_status` | `id`, `status`, `reason?` | updated summary; if status→`superseded` requires `superseded_by`. Appends a status-log line to the file rather than mutating history. |
| `adr_brief` | `tags?` | token-lean orientation digest: accepted decisions as one-liners grouped by tag, open proposals, recent supersessions. Intended as first call of a session. |
| `adr_comment` | `id`, `text`, `stance` (`support`\|`concern`\|`question`) | appends a signed comment to the record; enables in-flight collaboration on proposals without competing ADRs. |

Design rules: all results token-lean (summaries by default, full text only via `adr_get`); tool descriptions encode norms, not just parameters ("search before creating; supersede rather than duplicate; if you act against an accepted ADR, record why").

## 6a. Exposure & Collaboration Mechanics

The store only produces collaboration if agents actually consult it. Four mechanisms make exposure structural rather than hopeful:

1. **Ambient footer:** every `tool_result` from any `adr_*` tool appends a one-line repo pulse — `N accepted · M proposed (awaiting input: #21 #24) · K changed since your last call` — so any agent that touches the system once stays passively aware of open proposals and changes.
2. **Session-start brief:** convention (in tool descriptions and docs) is to call `adr_brief` before substantive work.
3. **Spawn-time inheritance:** documented pattern for parents — embed relevant ADR one-liners + IDs in child `task`/`system` prompts and instruct children to `adr_get` details and record new decisions. Decision context flows down the agent tree by default.
4. **Duplicate guard in `adr_create`:** creation internally runs FTS on title/decision; on strong matches it returns the candidates *instead of creating* (override with `force:true`, or supersede explicitly). The read happens inside the write.

## 7. Behavior Details

- `adr_create` with `supersedes` set: atomically marks the referenced ADRs `superseded` and writes `superseded_by`.
- Search ranks `accepted` above other statuses; superseded records are returned but flagged.
- Errors returned as `tool_result` with `isError:true` and a one-line actionable message (unknown id, bad status transition, lock timeout >2s).
- All writes fsync the file, then update the index.

## 8. Requirements Summary

**Functional:** F1 create; F2 get; F3 keyword+tag+status search; F4 list; F5 status transitions with supersession linkage; F6 shared store across concurrently connected agents; F7 index rebuild from files on startup.

**Non-functional:** search p50 <100 ms at 5k records; single static binary, no runtime deps beyond bundled SQLite; graceful degradation (if index is corrupt, rebuild automatically); files remain valid Markdown readable without the tool.

## 9. Milestones

1. **M1 — Core store:** file format, ID allocation, locking, `adr_create`/`adr_get`. 
2. **M2 — Query:** SQLite FTS index, `adr_search`/`adr_list`. 
3. **M3 — Lifecycle:** `adr_set_status`, supersession semantics, multi-agent concurrency test (two agents + subagents against one dir). 

## 10. Risks

- **Duplicate/near-duplicate ADRs from agents:** mitigated by tool-description guidance + search-first convention; semantic dedupe deferred.
- **Lock contention with many subagents:** writes are rare and small; 2s lock timeout + retry guidance.
- **30s UDS tool timeout:** all operations are local and bounded; no network calls.

## 11. Future (explicitly out of MVP)

Semantic search/embeddings; system-prompt injection of "open proposed ADRs"; cross-host sync; review/approval workflow; MADR-style option matrices; auto-git-commit.

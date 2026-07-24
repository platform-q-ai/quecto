# Agent Capability Guide

This file is the compact entry point for agents running inside the Quecto harness. Use it as an on-demand retrieval map, not as content to memorize up front.

## Retrieval policy

- Read only the docs needed for the current task.
- Start here when the task asks how to use Quecto itself, improve agent behavior, coordinate work, recover context, add capabilities, or operate the workflow system.
- After opening a targeted doc, follow its relevant cross-references before implementing changes.
- Prefer targeted reads over loading whole manuals into context.

## What to read

These docs are served by the `docs` tool (embedded in the binary, reachable
from any directory) — read one with `docs {"name": "<name>"}`, or call `docs`
with no name to list them.

| Need | Read first (`docs` name) |
|---|---|
| Understand Quecto's agent loop, built-in tools, configuration, sessions, or provider behavior | `readme` |
| Add, edit, or explain runtime models/providers in `~/.quecto/models.json` (API key vs OAuth, hot reload, provider keys) | `models-providers` |
| Manage long-running context, persisted sessions, spill/recall, pruning, or conversation recovery | `sessions` |
| Add or reason about new agent tools and prompt snippets through extensions | `extensions` |
| Delegate work, spawn child agents, steer/follow up, inspect child state, or recover subagent output | `subagents` |
| Self-organize with templates, checklists, issue state, workflow guards, or live workflow prompt guidance | `workflow` |
| Understand why a tool is unavailable or deliberately disabled | `disable-tools` |
| Build a low-level client or external extension over the Unix-domain-socket protocol | `uds-protocol` |

## Operational guidance

- Use built-in tools first: for Rust code navigation and impact assessment, prefer `rust_ast_graph` before broad text search; use filesystem tools for targeted reads/edits, `grep`/`find` for non-Rust or raw-text discovery, `bash` for focused checks, `recall` for spilled context, and `spawn`/`agent_cmd` when parallel or delegated investigation helps.
- In UDS/TUI sessions, the `workflow` tool is normally available but dormant; ask to select a template when explicit progress tracking is useful. Use `--workflow` only when you want workflow prompt guidance from the first turn, and `--no-workflow` when the workflow tool must be hidden entirely.
- Use subagents for separable investigations, parallel test/debug work, or preserving a focused parent context.
- To make a subagent run a specific multi-step workflow, pass `spawn` a `workflow_spec` (the full template by value): the child is bound to exactly that workflow in Active mode and cannot pick another. Use this instead of `workflow: true` (which lets the child choose) when you want to dictate the child's process. See the `subagents` and `workflow` docs (via the `docs` tool).
- To watch a subagent's progress, don't poll each child with `get_state` in a loop. Children's `workflow_state` events are identity-tagged (`agent_id` + `parent_id`) and forwarded onto your own event stream, and `get_subagents` returns each child's `parent_id` plus a live `workflow` snapshot — so you can reconstruct the whole unit tree and track progress from one place. See the `subagents` doc ("Observing the unit tree", via the `docs` tool).
- Use extensions when the agent needs a durable new capability rather than repeatedly improvising shell commands.
- Keep context lean: retrieve docs and historical context only when they are needed for the next decision.

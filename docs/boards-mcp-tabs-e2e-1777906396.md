# Boards MCP tab inspection smoke test

This smoke file documents the value of inspecting every Boards card tab through the MCP interface.

The end-to-end workflow validates that an outside automation client can:

- create or track a Boards card for a Quecto pod task;
- run the configured Quecto workflow in a disposable pod;
- inspect card tabs with `boards.inspect_card(tab: ...)` after work is completed; and
- confirm that tab data remains useful for delivery review without merging the pull request.

For this run, the repository change is intentionally docs-only. The expected delivery evidence is the pull request URL, the commit hash, and the required checks: `git status` and `git diff --check`.

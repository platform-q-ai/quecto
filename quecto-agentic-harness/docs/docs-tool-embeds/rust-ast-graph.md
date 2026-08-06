# Rust AST graph tool

Use `rust_ast_graph` when Rust symbol navigation would otherwise require broad `grep` and full-file reads. It builds an on-demand, sandbox-limited, syntax-derived graph of `.rs` files in the current workspace or an optional `path` scope.

The tool is intentionally **syntax-derived, not compiler/type-proven**. Treat references and calls as lexical or syntactic candidates; verify consequential edits with focused reads and tests.

## Common actions

- Workspace overview:
  `rust_ast_graph {"action":"overview","limit":20}`
- Find declarations by symbol, qualified path, or stable id:
  `rust_ast_graph {"action":"find_symbol","symbol":"ToolRegistryImpl","limit":10}`
- Traverse a selected symbol to module/import/impl/call-site neighbors:
  `rust_ast_graph {"action":"neighbors","symbol":"path.rs:10:40:struct:Thing","depth":1,"limit":25}`
- Find lexical references, excluding comments and string literals by default:
  `rust_ast_graph {"action":"references","symbol":"helper","limit":50,"snippet_lines":2}`
- Find syntactic call candidates:
  `rust_ast_graph {"action":"calls","symbol":"execute","limit":50}`
- Run structural queries:
  `rust_ast_graph {"action":"query","query":"async_functions","limit":50}`

Supported queries: `async_functions`, `unsafe_blocks`, `trait_impls`, `public_api`, and `functions`.

## Context controls

Use `limit` to cap result count, `snippet_lines` to control per-result context, `depth` for neighbor traversal, `path` to restrict the scan to a workspace-relative file or directory, and `include_bodies` only when declaration body snippets are worth the extra context. Set `raw_text: true` on `references`/`calls` only when matches in comments or string literals are useful.

## Suggested workflow

1. Start with `overview` or `find_symbol` to identify the declaration and disambiguate duplicate names.
2. Use `neighbors` and `references`/`calls` for impact assessment before editing.
3. Read only the specific files/spans needed for implementation details.
4. After editing, run targeted Rust tests/checks; do not rely on syntax candidates as proof of semantic correctness.

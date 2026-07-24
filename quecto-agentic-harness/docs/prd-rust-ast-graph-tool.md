# PRD: Rust AST graph traversal tool for agents

**Status:** Draft
**Owner:** core team (agent tools)
**Surface:** Agent tool capability

---

## Problem / Outcome

Agents currently understand Rust repositories mostly through filenames, text search,
and full-file reads. That is adequate for small localized changes, but weak for
repo-wide refactors and cross-module reasoning because text-based discovery cannot
reliably distinguish definitions, references, imports, implementations, call sites,
or similarly named symbols in comments and unrelated scopes.

Provide agents with a Rust-aware code-navigation tool that exposes the codebase as
a traversable semantic graph. The tool should let agents start from a symbol,
module, file, or syntax item and retrieve precise neighboring nodes and
relationships while returning compact snippets rather than whole files. The first
version should materially improve Rust impact assessment, refactoring planning,
and review quality, while establishing a path toward deeper type-aware information
where raw syntax alone is insufficient.

## Acceptance criteria (behavioural, checkable)

1. Agents can ask for a Rust workspace overview and receive a compact graph of
   crates, modules, source files, and top-level declarations without reading every
   source file into context.
2. Agents can locate Rust symbols by name or qualified path and get stable source
   locations, declaration kind, visibility, signature-like summary, and a concise
   surrounding snippet.
3. Agents can traverse from a Rust symbol to related graph neighbors including at
   least containing module, child declarations, imports/uses, implementations,
   trait relationships that are syntactically evident, and syntactic call sites.
4. Search and traversal results exclude comments and string literals unless the
   caller explicitly requests raw text-like matching.
5. When multiple Rust symbols share a name, the tool reports ambiguity with enough
   disambiguating context for the agent to choose a target rather than silently
   returning an arbitrary match.
6. Agents can request references or call sites for a selected Rust symbol and get
   bounded, ranked results with file locations and short snippets suitable for
   impact assessment before editing.
7. Agents can request a targeted structural query over Rust syntax, such as
   finding async functions, unsafe blocks, trait implementations, public APIs, or
   functions matching a simple predicate, and receive bounded structured results.
8. The tool clearly distinguishes information derived from syntax from
   information that requires name or type resolution, and does not present raw AST
   guesses as compiler-proven facts.
9. The tool works on the current workspace by default, respects the existing
   workspace/sandbox boundaries, and does not access files outside the allowed
   project tree.
10. The tool has predictable context controls: callers can cap result count,
    snippet size, traversal depth, and whether bodies are included.
11. Parse failures, unsupported Rust constructs, generated files, and very large
    repositories produce partial, actionable diagnostics rather than failing the
    entire tool call.
12. The tool is documented for agent use with examples that cover navigation,
    reference lookup, impact assessment, and structural queries.
13. Automated tests cover successful Rust graph construction, ambiguous symbols,
    comments/string false positives, bounded output, sandbox path handling, parse
    diagnostics, and at least one cross-module traversal scenario.

## Out of scope / Non-goals

- No multi-language AST support in this PRD; Rust only.
- No mandatory repo-wide codemod or write/edit operation in the first version.
  The tool may identify precise targets, but applying edits remains a separate
  action through existing editing tools unless a later PRD defines safe AST edits.
- No promise of full compiler-equivalent type inference or borrow checking in the
  first version.
- No background daemon, external service dependency, or network dependency is
  required for the first version.
- No changes to model/provider routing, session storage, workflow semantics, or
  TUI rendering are required.
- No unbounded indexing of files outside the current workspace or sandbox.
- No guarantee that macro-expanded code is fully represented unless a later
  type-resolution or language-server-backed phase explicitly adds it.

## Constraints

- Expected shape: a new first-party agent tool with focused parsing/indexing
  support, tests, and documentation; avoid broad architectural rewrites.
- Prefer an incremental design: syntax-backed graph first, optional deeper
  compiler or language-server integration later.
- Keep tool output compact and structured so agents can consume it without
  exhausting context.
- Respect existing repository conventions for tool registration, sandbox checks,
  error handling, truncation, and test style.
- Do not introduce speculative generic graph infrastructure unless needed for the
  Rust tool's behaviours.
- Dependencies, if added, should be actively maintained Rust ecosystem crates and
  justified by the behaviours they enable.

## Facts & sources

- Rust Analyzer provides semantic Rust IDE features including go-to definition,
  find references, and completion through the Language Server Protocol:
  https://rust-analyzer.github.io/
- The Language Server Protocol defines standard requests such as definition,
  references, document symbols, workspace symbols, call hierarchy, and rename:
  https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/
- Tree-sitter provides incremental concrete syntax parsing and has a Rust grammar:
  https://tree-sitter.github.io/tree-sitter/ and
  https://github.com/tree-sitter/tree-sitter-rust
- The `syn` crate parses Rust source into a syntax tree for procedural macro and
  source analysis use cases: https://docs.rs/syn/

## Hints (non-binding — NOT acceptance criteria)

- A possible v1 implementation is a native Rust tool named something like
  `rust_ast_graph` that indexes `.rs` files on demand and exposes actions such as
  workspace overview, find symbol, neighbors, references, calls, and structural
  query.
- Syntax-only implementation options include `syn` for Rust item-level parsing or
  Tree-sitter Rust for tolerant concrete syntax and node ranges. Tree-sitter may
  be better for partial files and structural queries; `syn` may be simpler for
  Rust item declarations.
- A later phase could optionally integrate Rust Analyzer/LSP for type-aware
  resolution, rename preparation, call hierarchy, and macro-expanded information.
- Existing core tools and registration patterns under the agent harness can serve
  as examples for tool definitions, JSON schemas, sandbox-aware paths, and tests.
- Consider returning stable node identifiers based on normalized path plus byte
  range plus kind so follow-up calls can traverse without repeating broad
  searches.

## References

- User story: agents need semantic Rust code navigation for accurate symbol
  navigation, safer refactoring, dependency analysis, targeted edits, impact
  assessment, structural queries, review quality, reduced context usage, and more
  reliable automation. The highest value is repo-wide refactors; type-resolution
  or language-server information would further improve correctness beyond a raw
  syntax tree.

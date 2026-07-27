# Parity contract — #1257 Phase 5

**Status: APPROVED** (readiness gate passed; no `__UNRESOLVED__` markers)

**Issue:** #1257 Phase 5 — `sessions/`, `workflow/`, `inference/`, `workspace/`; delete `infrastructure/`.

**Classification:** zero behaviour change (structural relocation + raw-JSON burn-down via typed protocol mappers only). Discriminator: issue explicitly mandates zero behaviour change throughout; acceptance criteria are structural/parity-only.

## Readiness gate

| Check | Result |
|---|---|
| Issue mandates zero behaviour change | PASS — plan header: "Zero behavior change throughout" |
| Observable surfaces enumerable | PASS — listed below |
| Test/render harness exists per surface | PASS — unit + characterization + architecture/BDD lockstep + existing TUI harness tests |
| No mixed behaviour+refactor | PASS — Phase 5 is file moves + mapper conversions only |
| Performance of specialized code | WARNING accepted: `workspace_files` git-first + `MAX_WORKSPACE_FILES=5000` single-pass BTreeSet / fs-walk stay byte-identical after move — consequence if regressed: monorepo `@` lag or incomplete file lists (undetectable without capacity/source tests; existing `workspace_files_tests` + production capacity pins cover) |

## Structural goals (REVIEW-TIME only — never test assertions)

- Four top-level modules exist: `sessions`, `workflow`, `inference`, `workspace`
- `infrastructure/` deleted
- `lib.rs` exact set: `agents`, `components`, `conversation`, `inference`, `interface`, `protocol`, `sessions`, `shell`, `workflow`, `workspace` (sorted as implemented)
- App slices remain `#[path]`-mounted under `interface::app` (no controller extraction)
- Architecture/BDD/doc lockstep updated; feature/view raw-JSON seed lowered only by genuine protocol conversions
- Ratchet scan roots extended to the four new modules

## (a) Touched observable surfaces

1. **Sessions flow** — resume selector latch, context-stats-requested latch
2. **Workflow flow** — auto-continue / completion-nudge mirrors; workflow automation sync; workflow snapshot → bar
3. **Inference flow** — model registry/selector, effort selector/vocabulary, set_model / set_effort command routing and success handling
4. **Workspace flow** — files autocomplete capacity 8, git branch footer + poll root, workspace file listing
5. **Footer get_state mapping** — model / maxContextTokens / effort from `get_state` payload
6. **Protocol mapper surface** — new typed DTOs for get_state footer fields, set_effort success, workflow snapshot/automation (as converted)
7. **Crate module graph / architecture pins** — lib.rs modules, layer existence, misplaced-file allowlist, doc owner map, BDD feature/steps
8. **Import paths** — `crate::infrastructure::workspace_files` → `crate::workspace::…`

## (b) Per-surface behaviours that must stay identical

### Sessions (`SessionsFlow`)
- Default: `resume_selector = None`, `context_stats_requested = false`
- After successful `list_sessions`, resume selector opens with same `SelectList` contents (via existing `parse_resume_sessions`)
- `context_stats_requested` flips true when get_state carries `maxContextTokens`; cleared on set_model master path as today

### Workflow (`WorkflowFlow` + automation)
- Default: both automation flags false
- `set_workflow_automation` success updates flags from `automation.autoContinue` / `automation.completionNudge` (or top-level keys when automation object absent — current `sync_workflow_automation` parity)
- `mirror_automation_to_bar` copies live flags onto master workflow bar after any bar rebuild
- Compact main-pane line shows `auto:on`/`auto:off` and `nudge:on`/`nudge:off` matching live flags (#897)
- `parse_workflow_event` field fallbacks (camelCase/snake_case, activeIssue shapes) unchanged if snapshot parsing moves to protocol

### Inference
- `ModelRegistry`: empty entries, `open_pending=false` by default
- Bare `/model` / open selector: ListModels once while pending; open after list (or on empty/error fallback)
- `parse_model_entries` still delegates to `protocol::model_payloads::parse_model_list` + `is_current=false`
- Master `set_model`: send command, optimistic footer+current_model, clear context_stats_requested; child focus: route to child, no optimistic local model
- Effort: empty vocabulary → bare `/effort` warns; invalid level local reject with joined valid list; empty vocabulary allows passthrough; set_effort never updates footer until success `data.effort`; master success toasts + sets current_effort only when no child focused; late master success still updates master footer effort
- `send_state_resync` issues GetState id `resync`

### Workspace
- `FilesAutocomplete::new(8)` production capacity unchanged
- Git branch: poll interval 2s; HEAD read limit 4096; `ref: refs/heads/` strip then `ref: ` strip; bidi/format control sanitization; gitdir file resolution for worktrees
- `list_workspace_files`: git ls-files hardened args first (fsmonitor/hooks disabled), merge tracked+others, safe-path filter, cap 5000 sorted; empty/fail → fs_walk skipping `.git`/`target`/`node_modules`/`.jj`/`dist` and dot dirs, same cap/sort/safe-path

### Footer `apply_get_state`
- model: sanitize via control strip; set_model; return Some(sanitized) when present
- maxContextTokens: u64 → set_context_window as usize when present
- effort: missing key OR null → set_effort(None); string → sanitized Some
- Does not touch context_used/cost/streaming/pwd/git_branch

### Architecture observability
- After phase: no `infrastructure/` dir; four new dirs with mod.rs; production files under approved tops only
- Feature scenario + steps + architecture.rs lockstep (fn names / layer lists / lib module set / doc verbatim rows)

## (c) Performance characteristics (must not regress)

| Code | Characteristic |
|---|---|
| `list_workspace_files` / git path | Single `git ls-files` + optional `--others`; BTreeSet dedupe; hard stop at 5000 |
| `list_workspace_files` / fs_walk | Iterative stack walk; skip well-known heavy dirs; sort once at end |
| Git branch read | Bounded 4KiB read; no full-repo walk |
| Footer/get_state mappers | Single-pass field extraction; no allocation beyond owned strings |
| Workflow snapshot parse | Single pass over steps array; same as current `parse_workflow_event` |

## Characterization assets (pre-move hashes)

| Path | `git hash-object` |
|---|---|
| `interface/app_models_protocol_characterization_tests.rs` | `180c9d2e859fa8bd1e913ce80a9d4801633fd901` |
| `interface/app_models_tests.rs` | `fe1c7be7693dc424ec6868b286e14e2f91fed6de` |
| `interface/app_effort_1067_tests.rs` | `97a6409aca7b739cc854bd37e6a962e2a52ce9df` |
| `interface/app_git_tests.rs` | `1032f23275ab9b4de146bc0d2cac50b6a6d9483f` |
| `interface/app_workflow_flow_tests.rs` | `96e1cefde0c400b32be7dc998e3d792712f6bf79` |
| `interface/app_workflow_box_width_tests.rs` | `090c69f574575022f18151099b61172a2b782e01` |
| `infrastructure/workspace_files_tests.rs` | `8b3188f9337b7f26ad515ecaafd09c22957d0e9b` |
| `components/footer_tests.rs` | `3194c9b22f599dc650ccd21e56aadfc83dc27c8a` |
| `protocol/session_payloads_tests.rs` | `9e5e761b493b7026e2e9d2aeefdf2aca299214fe` |
| `protocol/model_payloads_tests.rs` | `19d090d20fe55779bede78cd535a3e8093bff086` |

Frozen suite = these tests plus architecture/BDD lockstep and existing harness coverage for sessions/workflow/inference/workspace. Structural goals verified at conformance, not as behavioural asserts.

## Mapper conversion scope (genuine burn-down only)

1. **Footer get_state** — `Footer::apply_get_state` body → `protocol` typed snapshot; footer applies typed fields (component stays dumb).
2. **set_effort success** — effort string extraction → protocol helper.
3. **Workflow snapshot / automation** — automation flag extraction (and workflow event parse if moved without behaviour change) → protocol DTOs; components/app consume typed values.
4. Session stats/listing already live in `protocol/session_payloads` — no duplicate conversion.

Seeds lowered only by sites removed from feature/view scan roots via real mapper moves (not by relocating unscanned files).

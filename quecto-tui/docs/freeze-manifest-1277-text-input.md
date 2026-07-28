# Freeze manifest — #1277 text input characterization

**Status: FROZEN** after mutation evidence (M1–M18 all killed) and characterization
finder triage (MED gaps fixed; LOWs/pre-existing Gherkin declined with rationale
in mutation log).

READ-ONLY until the parity step unless a mechanical call-site adaptation is
required and re-run through mutation evidence.

| File | `git hash-object` | Role |
|---|---|---|
| `quecto-tui/src/components/editor_tests.rs` | `3eca6a3993a76980de620d237ba4618871818318` | Primary unit characterization (59 tests) |
| `quecto-tui/src/shell/app_text_input_1277_tests.rs` | (split from event-loop suite for file-size gate) | App integration pins (Enter, autocomplete, slash) |
| `quecto-tui/src/shell/app_event_loop_tests.rs` | pre-existing pins (Ctrl+C, Escape, @files, etc.) | Supporting app key routing |
| `quecto-tui/src/shell/app_input_paste_tests.rs` | `e101dfe112534178815b372c17e485ddff47e3e0` | App paste → no auto-submit |
| `quecto-tui/docs/parity-contract-1277-text-input.md` | `aa79d3e60f7981418855621f68eb20781df00129` | Approved parity contract |
| `quecto-tui/docs/mutation-log-1277-text-input.md` | `4b95890192ae978c2335d073c8d9db3de5f48bc3` | Mutation evidence + finder triage |

Supporting (existing, not modified this slice; still GREEN pins):

- `quecto-tui/src/conversation/app_rewind_response_tests.rs` — rewind baseline
- TUI BDD paste/border/submit/autocomplete scenarios

Production code at freeze: **unmodified** relative to HEAD for editor/event-loop.

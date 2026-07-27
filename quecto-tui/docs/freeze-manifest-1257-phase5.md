# Freeze manifest — #1257 Phase 5

Frozen after characterization GREEN + mutation evidence. Characterization tests are
READ-ONLY until the parity step unless a mechanical call-site adaptation is
logged with re-mutation evidence.

Recorded at freeze time on branch tip before production moves:

| Path | `git hash-object` |
|---|---|
| `quecto-tui/src/interface/app_models_protocol_characterization_tests.rs` | `180c9d2e859fa8bd1e913ce80a9d4801633fd901` |
| `quecto-tui/src/interface/app_models_tests.rs` | `fe1c7be7693dc424ec6868b286e14e2f91fed6de` |
| `quecto-tui/src/interface/app_effort_1067_tests.rs` | `97a6409aca7b739cc854bd37e6a962e2a52ce9df` |
| `quecto-tui/src/interface/app_git_tests.rs` | `1032f23275ab9b4de146bc0d2cac50b6a6d9483f` |
| `quecto-tui/src/interface/app_workflow_flow_tests.rs` | `96e1cefde0c400b32be7dc998e3d792712f6bf79` |
| `quecto-tui/src/interface/app_workflow_box_width_tests.rs` | `090c69f574575022f18151099b61172a2b782e01` |
| `quecto-tui/src/infrastructure/workspace_files_tests.rs` | `8b3188f9337b7f26ad515ecaafd09c22957d0e9b` |
| `quecto-tui/src/components/footer_tests.rs` | `3194c9b22f599dc650ccd21e56aadfc83dc27c8a` |
| `quecto-tui/src/protocol/session_payloads_tests.rs` | `9e5e761b493b7026e2e9d2aeefdf2aca299214fe` |
| `quecto-tui/src/protocol/model_payloads_tests.rs` | `19d090d20fe55779bede78cd535a3e8093bff086` |

Mechanical path updates after file moves (hash may change only if path string
literals inside tests change — none of the frozen bodies assert on source paths
except via module mounting which is outside these files). Expected post-move
path renames:

- `interface/app_*` slices → feature modules (tests move with owners)
- `infrastructure/workspace_files_tests.rs` → `workspace/workspace_files_tests.rs`

Any hash delta after moves will be explained in the parity evidence table.

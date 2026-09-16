# Session home scope and cross-folder resume (#2001)

## UX
- `/resume` (no key) opens the scope-aware picker **Local by default**.
- Visible **Local | Global** control: keyboard (Tab focus, Enter/Space toggle) and mouse.
- **Global** searches metadata only: title, opaque key, repository label, path — never transcript body.
- Exact `/resume <key>` remains a global opaque-key lookup.
- **Ctrl+G** remains jump-to-latest conversation (unchanged).
- Cross-folder selection requires explicit **Open original / Fork here / Cancel**.
- Missing/moved home requires **Locate / Fork / Cancel** (no mkdir, no silent git checkout).
- **Open original** launches a fresh target-rooted runtime (not `chdir` of the current process).
- **Fork** creates a new opaque identity and imports transcript only.

## Architecture
- Domain: `SessionHomeScope`, `ResumeDisposition`, versioned `HomeScopeMetadata` (pure).
- Application: workspace discovery ports, scope catalogue, `DecideResumeDisposition` transactions.
- Infrastructure: FS canonicalizer, Git workspace probe, file scope catalogue.
- TUI: `ResumeScopePicker` presentation state maps to application dispositions (interface parse/map/present).
- Catalogue is derived/discardable; transcripts remain recovery authority.
- Affirmative allowlist guards throughout.

## Process note (Grok 4.5 TDD experiment)
D2 discovery adapters shipped with a **TDD process exception** (functional GREEN without retained pre-impl RED). See external evidence `d2-tdd-process-exception.md` on the experiment branch report — not a product waiver of the issue contract.

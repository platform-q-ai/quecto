# Superseded Clean Architecture target model for `quecto-tui`

> **SUPERSEDED.** Epic [#1149](https://github.com/platform-q-ai/quecto/issues/1149)
> was rewritten around harness-facing presentation capability modules with
> selective functional cores. This document is retained only for historical
> context. Use
> [`feature-oriented-presentation-architecture.md`](feature-oriented-presentation-architecture.md)
> as the current architecture direction. Do not use this document as an
> implementation reference.

Parent epic: [#1149](https://github.com/platform-q-ai/quecto/issues/1149).

The former document in this file described an abandoned four-layer Clean
Architecture target for `quecto-tui`. It prescribed `domain/`, `application/`,
`infrastructure/`, and `interface/` as the end-state architecture and contained
migration instructions for that model.

That direction is no longer current. `quecto-tui` is now treated as a
feature-oriented presentation adapter for harness-facing capabilities. The
current direction is documented in
[`feature-oriented-presentation-architecture.md`](feature-oriented-presentation-architecture.md).

Historical summary of the abandoned model:

- move policy and orchestration out of `interface::App` into functional-core
  services;
- interpret raw UDS protocol JSON at infrastructure boundaries;
- keep `domain/` deliberately thin;
- treat `interface/` as composition, runtime ownership, and rendering;
- migrate in behavior-preserving vertical slices.

Those goals were useful stepping stones, but they are not the final target model.
For new implementation work, follow the feature-oriented presentation
architecture instead.

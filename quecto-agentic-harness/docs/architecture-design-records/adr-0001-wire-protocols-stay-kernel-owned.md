# ADR-0001 — Wire Protocols Stay Kernel-Owned

**Status:** Accepted.

**Implementation status:** Partially implemented.

## Context

The Models/providers surface is the one extension surface where a *genuinely new
wire/streaming protocol* currently forces a recompile. Two pressures collided:

- **The "no walls" rule** says everything the community wants should be reachable
  through a runtime surface. A blocked wire protocol is a literal wall.
- **A real incident:** adding Fireworks serverless models surfaced that quecto
  only ships OpenAI/Anthropic-shaped protocols, and that the router mishandled
  multi-segment model IDs.
- **A concrete reference design:** pi (`earendil-works/pi`) pushes wire protocols
  *out* of its kernel via in-process TypeScript extensions. That maximises reach
  but moves a security- and correctness-critical parser into community hands and
  relies on an extension model quecto does not have.

The wire protocol parses **untrusted provider output** directly into the agent's
context and tool-call stream. Edge cases around partial tool-call JSON, thinking
blocks, usage/cost accounting, context overflow, aborts, and Unicode boundaries
are exactly the things that break silently.

## Decision

Wire/streaming protocol *implementations* remain **kernel-owned**. The team builds
and maintains exactly **three** robust, fully-tested protocols, selected per model
by an `api` field:

1. `openai-completions` — OpenAI Chat Completions and compatibles.
2. `anthropic-messages` — Anthropic Messages and compatibles.
3. `google-generative-ai` — native Google Gemini.

The community extends **models** freely on top of these three via a runtime model
registry (config/`models.json` + live reload). The kernel ships only a few example
models per protocol; the registry overrides/extends them.

## Current Implementation

- `openai-completions` is implemented.
- `anthropic-messages` is implemented.
- `google-generative-ai` is reserved in the model registry parser, but provider
  construction still errors as not implemented.

## Consequences

- *Positive:* the security-/correctness-critical surface stays audited and
  semver-stable; broad compatibility is possible without a plugin loader or
  scripting engine.
- *Negative / cost:* one wall remains: a genuinely novel protocol still needs a
  kernel release, and native Gemini remains pending until implemented.
- *Mitigation:* bridge a novel protocol with a UDS/local process that translates
  it to an **OpenAI-compatible** endpoint, then add it as a normal
  `openai-completions` provider.
- *Door left open:* if demand for novel protocols proves real, a future kernel
  version may add a UDS `register_stream` surface.

## Alternatives Considered

- **A. Community-authored wire protocols.** Rejected for v1 because quecto has no
  in-process extension surface, UDS streaming would add a large protocol, and an
  embedded scripting runtime would bloat the binary and hand parsing of untrusted
  model output to community code.
- **B. Ship only OpenAI + Anthropic.** Rejected because it leaves "Google
  compatible" as a gateway-only half-truth.
- **C. Generic/pluggable protocol config.** Rejected because real protocols differ
  in streaming framing, tool-call semantics, and thinking handling in ways a
  declarative schema cannot safely capture.
- **D. Keep the status quo.** Rejected because the Fireworks incident showed the
  model/provider surface needs explicit scope.

## Related

- [Kernel boundary](../architecture/kernel-boundary.md)
- [Runtime models/providers](../runtime-models-providers.md)
- [Models runtime extensible PRD](../prd/prd-models-runtime-extensible.md)

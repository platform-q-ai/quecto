# Tool Isolation Strategy: WASM + nsjail

## Problem

The current tool system runs all tools in-process with the host. The Sandbox provides a denylist for dangerous commands and workspace-scoped path validation, but denylist coverage is inherently incomplete. An unanticipated command the LLM generates — a `find / -delete` variant, a `pip install` with a malicious post-install script, a `cargo clean` in the wrong directory — bypasses the denylist and damages the host. The threat model is not malicious tool code (we wrote it). It is accidental blast radius from LLM-generated actions that we lacked the foresight to deny.

A secondary goal is extensibility. We want the agent to eventually build its own tools at runtime and install third-party tools — code we did not write and do not trust. Those tools need hard isolation boundaries that go beyond what an in-process denylist can provide.

## Decision

Two-tier isolation model. No Docker. No daemon. No heavy runtime.

### Tier 1: WASM containers (all tools except exec and spawn)

Tools that do not need host process access run as WebAssembly components compiled to `wasm32-wasip2`, executed by Wasmtime with the Component Model. Each tool invocation gets a fresh `Store` — no state carries between calls. Tools interact with the host exclusively through a declared WIT interface.

**Tools containerized in WASM:**

| Tool | Host capabilities via WIT imports |
|---|---|
| ReadFileTool | `workspace-read(path) -> result<string, string>` |
| WriteFileTool | `workspace-write(path, content) -> result<string, string>` |
| EditFileTool | `workspace-read` + `workspace-write` |
| AppendFileTool | `workspace-append(path, content) -> result<string, string>` |
| ListDirTool | `workspace-list-dir(path) -> result<string, string>` |
| CronTool | `cron-store-op(action, payload) -> result<string, string>` |
| RecallTool | `spill-store-op(action, payload) -> result<string, string>` |
| MessageTool | `send-message(target, text) -> result<string, string>` |
| WebSearchTool | `http-request(method, url, headers, body) -> result<string, string>` |
| Future third-party/agent-built tools | Declared capabilities only |

**Sandbox properties per WASM tool execution:**

- Fresh instance per call (no cross-invocation state leaks)
- Fuel metering (CPU budget per call)
- Memory limiter (bounded per call)
- Epoch interruption (backup timeout via background ticker)
- No filesystem access except through host-imported functions with path validation
- No network access except through host-imported HTTP with endpoint allowlisting
- No process spawning capability
- Credentials injected on the host side, never visible to WASM code

**Why these tools and not the others:**

These tools are already structured as pure logic over trait ports (`CronStore`, `ContextSpillStore`, `Channel`) or straightforward filesystem/network operations. Their dependencies map directly to WIT host imports. The `Tool` trait's interface — JSON string in, `ToolResult` (string + bool) out — maps directly to a WIT `execute(params: string) -> (output: string, error: string)` contract.

### Tier 2: nsjail (exec tool only)

The exec tool runs arbitrary shell commands against the host system. The agent needs real binaries — `git`, `cargo`, `python`, whatever is installed. A WASM shell cannot provide this (WASM shells run WASM binaries, not host ELF binaries). Sandboxing exec in WASM and re-exposing it through a host import gains nothing — the host still runs `sh -c`.

Instead, exec runs inside nsjail — a lightweight Linux process isolator (~2-3 MB) that uses kernel namespaces and cgroups directly. No Docker, no daemon.

**nsjail provides:**

| Protection | Mechanism |
|---|---|
| Filesystem isolation | Mount namespace: workspace bind-mounted RW, toolchain (`/usr`, `/lib`, `/bin`) bind-mounted RO, everything else invisible |
| Process isolation | PID namespace: agent processes cannot see or signal host processes |
| Resource limits | cgroups v2: `--cgroup_mem_max` (memory), `--cgroup_pids_max` (fork bomb defense), `--rlimit_cpu` (CPU time) |
| Timeout | `--time_limit` with automatic kill |
| Syscall filtering | Kafel seccomp-bpf DSL for restricting available syscalls |
| Network control | Network namespace with optional passthrough (`--disable_clone_newnet`) |
| Cleanup | `PR_SET_PDEATHSIG` ensures sandbox dies if parent crashes |

**Why nsjail over bubblewrap:**

bubblewrap provides namespace isolation but no resource control — no memory limits, no PID limits, no CPU limits, no built-in timeout. When an LLM-generated command accidentally fork-bombs or runs a build that eats 8 GB of RAM, nsjail kills it within configured limits. bubblewrap lets the host take the hit. nsjail also provides human-readable seccomp filtering (Kafel DSL) where bubblewrap requires pre-compiled BPF programs.

**Blast radius containment:**

If a command does something catastrophic inside nsjail, the namespace dies. The workspace bind-mount is the only thing affected. Restart the sandbox and carry on. Without nsjail, the host is damaged and recovery is manual.

**Graceful degradation:**

nsjail requires Linux namespaces. On environments where it is unavailable (restrictive VPS, containers-in-containers without nested namespace support), fall back to native exec with the Sandbox denylist — same as today. This is a config option (`exec.isolation: "nsjail" | "native"`).

### Spawn tool — unchanged

The spawn tool does not need isolation itself. It launches `quecto agent` as a child process. That child loads the same WASM-containerized tool registry and the same nsjail-wrapped exec. The isolation is inherited, not duplicated.

## WIT Interface Design

```wit
package quecto:tools;

interface host {
    // Filesystem (workspace-scoped, path-validated on host side)
    workspace-read: func(path: string) -> result<string, string>;
    workspace-write: func(path: string, content: string) -> result<string, string>;
    workspace-append: func(path: string, content: string) -> result<string, string>;
    workspace-list-dir: func(path: string) -> result<string, string>;

    // Network (allowlisted on host side)
    http-request: func(method: string, url: string, headers-json: string, body: string) -> result<string, string>;

    // Channel
    send-message: func(target: string, text: string) -> result<string, string>;

    // Storage adapters (JSON-serialized operations)
    cron-store-op: func(action: string, payload: string) -> result<string, string>;
    spill-store-op: func(action: string, payload: string) -> result<string, string>;

    // Logging
    log: func(level: string, message: string);
}

interface tool {
    // Tool contract — mirrors our existing Tool trait
    execute: func(params: string) -> result<string, string>;
    schema: func() -> string;
    description: func() -> string;
}

world sandboxed-tool {
    import host;
    export tool;
}
```

## Implementation Plan

### Phase 1: WIT contract and WASM runtime host

1. Add `wasmtime` (with `component-model` feature) and `wasmtime-wasi` as dependencies
2. Define `wit/tool.wit` with the `sandboxed-tool` world
3. Implement `WasmToolRuntime` in `infrastructure/tools/wasm/`:
   - `runtime.rs` — Wasmtime engine configuration, module cache (`RwLock<HashMap<String, Arc<PreparedModule>>>`), epoch ticker
   - `wrapper.rs` — `WasmToolWrapper` implementing `domain::Tool` trait, fresh `Store` per call
   - `host.rs` — host-side implementation of the `quecto:tools/host` interface, delegating to existing trait ports
4. Implement `WasmToolLoader` for loading `.wasm` files from a tools directory

### Phase 2: Port built-in tools to WASM

Port each tool as a standalone Rust crate compiled to `wasm32-wasip2`:

1. Filesystem tools (read, write, edit, append, list_dir) — depend on `workspace-read`/`workspace-write`/`workspace-list-dir` host imports
2. CronTool — depends on `cron-store-op` host import
3. RecallTool — depends on `spill-store-op` host import
4. MessageTool — depends on `send-message` host import
5. WebSearchTool — depends on `http-request` host import

Each tool crate uses `wit-bindgen` to generate bindings from `wit/tool.wit` and exports the `tool` interface.

### Phase 3: nsjail integration for exec

1. Add `NsjailExecTool` in `infrastructure/tools/exec_nsjail.rs`
2. Configure nsjail invocation: mount table, resource limits, timeout, seccomp policy
3. Add `exec.isolation` config option (`"nsjail"` | `"native"`)
4. Detect nsjail availability at startup, warn and fall back to native if absent
5. Keep the existing `ExecTool` as the `"native"` fallback

### Phase 4: Capabilities and third-party tool loading

1. Define capabilities schema (JSON sidecar files declaring HTTP allowlist, secret access, workspace prefixes)
2. Implement capability enforcement in the host-side WIT implementation
3. Add `quecto tools install <path>` CLI command for loading external `.wasm` tools
4. Add `quecto tools list` / `quecto tools remove` commands

### Phase 5: Agent-built tools (future)

1. Add a `build_tool` LLM tool that scaffolds, compiles, and registers WASM tools at runtime
2. Implement hot-swap via `RwLock<HashMap>` registry updates (IronClaw pattern)
3. Auto-register built tools with empty capabilities (require explicit capability grants)

## Dependencies Added

| Crate | Purpose | Size impact |
|---|---|---|
| `wasmtime` 28 | WASM Component Model runtime | ~15-20 MB to binary (significant, but one-time) |
| `wasmtime-wasi` 28 | WASI Preview 2 host implementation | Included with wasmtime |
| `wit-bindgen` 0.36 | Guest-side WIT bindings (dev dependency for tool crates) | Build-time only |

nsjail is an external binary, not a Rust dependency. Installed via distro package manager or pre-built in CI.

## Target Environments

| Environment | WASM tools | nsjail exec | Fallback exec |
|---|---|---|---|
| x86_64 VPS | Full support | Full support (install from distro repos) | Native + denylist |
| Raspberry Pi (aarch64) | Full support | Full support (pre-build in CI) | Native + denylist |
| Inside Docker | Full support | Needs `--privileged` or `--cap-add SYS_ADMIN` for cgroups; rlimits work without | Native + denylist |
| Environments without nsjail | Full support | Not available | Native + denylist |

## Reference

Architecture inspired by [nearai/ironclaw](https://github.com/nearai/ironclaw), which implements WASM tool containers with Wasmtime 28, Component Model, WIT contracts, capability-scoped host imports, and agent-built tool hot-swapping. Adapted for quecto's constraints (single static binary, minimal Linux, no database, no Docker).

# Extensions

Extensions let you add custom tools to Quecto without recompiling. Each extension is a directory containing a TOML manifest and an executable script. The agent discovers extensions at startup and makes them available as tools alongside the built-ins (`bash`, `read`, `write`, `edit`, `ls`, `grep`, `find`, etc.).

## Quick start

1. Create an extension directory under your workspace:

```bash
mkdir -p ~/.quecto/workspace/extensions/hello
```

2. Write the manifest (`extension.toml`):

```toml
name = "hello"
description = "Greet someone by name."
parameters_schema = '{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}'
command = "./hello.sh"
```

3. Write the script (`hello.sh`):

```bash
#!/bin/bash
input=$(cat)
name=$(echo "$input" | jq -r '.name // "World"')
echo "{\"content\": \"Hello, ${name}!\", \"is_error\": false}"
```

4. Make it executable:

```bash
chmod +x ~/.quecto/workspace/extensions/hello/hello.sh
```

The `hello` tool is now available. It will appear alongside the built-in tools in the next agent session.

## Directory layout

Extensions live under `<workspace>/extensions/`. The workspace path comes from your `config.json`:

```json
{
  "agents": {
    "defaults": {
      "workspace": "/home/you/.quecto/workspace"
    }
  }
}
```

The directory structure is:

```
<workspace>/extensions/
  hello/
    extension.toml     # Required — tool manifest
    hello.sh           # The executable referenced by `command`
  weather/
    extension.toml
    weather.py
```

Each subdirectory is treated as one extension. The directory must contain an `extension.toml` file. Directories without a valid manifest are silently skipped.

## Manifest reference

The `extension.toml` file defines how the tool appears to the LLM and how it is executed.

| Field | Required | Default | Description |
|---|---|---|---|
| `name` | yes | — | Tool name (used in LLM tool calls). Must be unique and must not shadow a core tool |
| `description` | yes | — | Description shown to the LLM. Supports multi-line via triple-quoted strings (`"""`) |
| `parameters_schema` | yes | — | JSON Schema string defining the tool's input parameters |
| `command` | yes | — | Path to the executable, relative to the extension directory. Must start with `./` |
| `timeout_secs` | no | `30` | Maximum execution time in seconds before the process is killed |
| `system_prompt` | no | none | Text injected into the agent's system prompt when this extension is loaded |

### Full example

```toml
name = "lookup_user"
description = """
Look up a user in the company directory.
Example: {"email": "alice@example.com"}
Returns the user's full name and department.
"""
parameters_schema = '{"type":"object","properties":{"email":{"type":"string","format":"email"}},"required":["email"]}'
command = "./lookup.sh"
timeout_secs = 10
system_prompt = "When looking up users, always verify the email format first."
```

### Manifest parser limitations

The manifest uses a minimal TOML parser. Supported features:

- Simple key-value pairs: `key = "value"` or `key = 'value'`
- Unquoted values: `timeout_secs = 30`
- Multi-line strings: `""" ... """`
- Comments: `# ...`

**Not supported:** escape sequences (`\n`, `\t`), inline tables, arrays, dotted keys, or other advanced TOML features. If you need newlines in a field, use the `"""` multi-line syntax.

## Script protocol

Extensions communicate via a stdin/stdout JSON protocol.

### Input

The tool arguments (a JSON string matching your `parameters_schema`) are written to the script's **stdin**. Your script reads from stdin, not from command-line arguments.

### Output

The script must print a single JSON object to **stdout**:

```json
{
  "content": "The result text shown to the LLM",
  "is_error": false
}
```

| Field | Type | Description |
|---|---|---|
| `content` | string | The text result returned to the agent |
| `is_error` | boolean | `true` if the result represents an error condition |

### Error handling

| Condition | Behavior |
|---|---|
| Non-zero exit code | Result is marked as error. If stderr has content, it's returned as the error message. Otherwise: `"extension '<name>' exited with code <N>"` |
| Invalid JSON on stdout | Result is marked as error: `"invalid output from extension '<name>': <raw stdout>"` |
| Timeout exceeded | Entire process group is killed with `SIGKILL`. Result: `"extension '<name>' timed out after <N>s"` |
| stdout or stderr exceeds 1 MiB | Process group is killed. Result: `"output exceeded 1MiB cap"` |

### Example scripts

**Bash:**

```bash
#!/bin/bash
input=$(cat)
name=$(echo "$input" | jq -r '.name')
echo "{\"content\": \"Hello, ${name}!\", \"is_error\": false}"
```

**Python:**

```python
#!/usr/bin/env python3
import json, sys

args = json.load(sys.stdin)
result = {"content": f"Hello, {args['name']}!", "is_error": False}
json.dump(result, sys.stdout)
```

**Node.js:**

```javascript
#!/usr/bin/env node
let input = '';
process.stdin.on('data', d => input += d);
process.stdin.on('end', () => {
  const args = JSON.parse(input);
  console.log(JSON.stringify({ content: `Hello, ${args.name}!`, is_error: false }));
});
```

## System prompt injection

Extensions can contribute to the agent's system prompt via the `system_prompt` manifest field. All non-empty snippets from loaded extensions are collected and injected into a clearly delimited section:

```
## Extensions
When looking up users, always verify the email format first.

Always format dates in ISO 8601.
## End Extensions
```

The delimiters prevent extension snippets from being misinterpreted as core system instructions by the LLM. Snippets from multiple extensions are separated by double newlines.

## Discovery and lifecycle

### Startup discovery

Extensions are discovered once when the agent starts. The agent scans `<workspace>/extensions/` and:

1. Lists all subdirectories (skipping symlinks — see [Security](#security))
2. Reads `extension.toml` from each subdirectory
3. Validates the manifest (all required fields present, valid `command` path)
4. Registers each extension tool in the tool registry (rejecting shadows — see [Tool name shadowing](#tool-name-shadowing))
5. Collects system prompt snippets from loaded extensions

After startup, the tool set is fixed unless explicitly reloaded.

### Manual reload via UDS

In UDS mode (`quecto agent --mode uds`), clients can reload extensions at runtime:

```json
{"type":"reload_extensions","id":"re-1"}
```

This re-scans `<workspace>/extensions/` from disk, replaces all script extension tools in the registry, and broadcasts an `extensions_changed` event to all connected clients. See [UDS Protocol Reference](uds-protocol.md) for details.

Clients can also query the current extension list:

```json
{"type":"get_extensions","id":"ge-1"}
```

### Hot-reload watcher

The infrastructure for automatic hot-reload (polling for changes to `extension.toml` files based on mtime and file size) is implemented but **not enabled by default** from the CLI. It is available for programmatic use via `UdsLoopArgs::hot_reload_interval`. When enabled, the watcher polls at the configured interval and triggers `reload_extensions` automatically when changes are detected.

### Tool deduplication

If multiple extensions define tools with the same name, the last one registered wins. Extension registration order follows filesystem directory listing order, which may vary by platform. Use unique tool names to avoid conflicts.

## Security

Extensions run as subprocesses of the Quecto agent. Several security measures are enforced:

### Command path restrictions

The `command` field **must** start with `./` (relative to the extension directory). This ensures extensions can only execute scripts within their own directory:

```toml
# ✓ Valid
command = "./run.sh"
command = "./bin/tool"

# ✗ Rejected at registration time
command = "/usr/bin/python3"    # absolute path
command = "../../etc/passwd"    # parent traversal
command = "curl"                # bare command (PATH lookup)
```

Parent directory traversal (`..`) anywhere in the path is rejected.

### Symlink rejection

Symlinked extension directories are skipped during discovery. This prevents extensions from pointing outside the trusted extensions directory. The check uses `symlink_metadata()` to detect symlinks before following them.

### Tool name shadowing

Extension tools cannot shadow core built-in tools. If an extension defines a tool with the same name as a built-in (e.g. `bash`, `read`, `write`, `edit`), it is rejected with a warning logged and **not** registered. The extension will not appear in `get_extensions` responses.

This applies both at startup and during reload/re-registration.

### Output cap

Script stdout **and** stderr are each capped at **1 MiB**. Both streams are read concurrently with size limits. If either stream exceeds this limit, the process and its entire process group are killed with `SIGKILL` and the tool returns an error.

### Timeout enforcement

When a script exceeds `timeout_secs`, the entire process group is killed with `SIGKILL` (via `kill -9 -<pgid>`), not just the lead process. This prevents orphaned child processes from lingering. Each extension runs in its own process group (via `process_group(0)` on spawn).

## Troubleshooting

### Extension not appearing

1. Verify the directory structure: `<workspace>/extensions/<name>/extension.toml`
2. Check that `extension.toml` has all required fields (`name`, `description`, `parameters_schema`, `command`)
3. Ensure `command` starts with `./`
4. Ensure the script is executable (`chmod +x`)
5. Check that the extension directory is not a symlink
6. Check that the extension name does not shadow a core tool (look for `"register_extension rejected: shadows core tool"` or `"extension tool rejected: shadows core tool"` in logs)

### Tool returns "invalid output"

The script must output valid JSON with `content` and `is_error` fields to stdout. Common causes:
- Debug/log output mixed into stdout — redirect logs to stderr instead
- Missing JSON encoding of special characters
- Script printing raw text instead of JSON

### Tool times out

Increase `timeout_secs` in the manifest, or optimize the script. The default is 30 seconds. On timeout, the entire process group is killed.

### Extension shadows a core tool

Extension tool names that conflict with built-in tools (`bash`, `read`, `write`, `edit`, `ls`, `grep`, `find`, `spawn`, `recall`, `web_search`, `workflow`) are silently rejected. Rename the `name` field in your `extension.toml` to something unique.

### Changes not taking effect

Extensions are discovered once at startup. To pick up new or modified extensions:
- **Restart** the agent, or
- **Send `reload_extensions`** via UDS (see [UDS Protocol Reference](uds-protocol.md))

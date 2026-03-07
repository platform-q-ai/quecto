# Extensions

Extensions let you add custom tools to Quecto without recompiling. Each extension is a directory containing a TOML manifest and an executable script. The agent discovers extensions at startup, makes them available as tools, and hot-reloads them when files change on disk.

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
# Arguments arrive as JSON on stdin
name=$(echo "$1" | jq -r '.name // "World"')
# Read from stdin since that's how arguments are actually passed
input=$(cat)
name=$(echo "$input" | jq -r '.name // "World"')

echo "{\"content\": \"Hello, ${name}!\", \"is_error\": false}"
```

4. Make it executable:

```bash
chmod +x ~/.quecto/workspace/extensions/hello/hello.sh
```

The `hello` tool is now available to the agent. It will appear alongside the built-in tools (`bash`, `read`, `write`, etc.) in the next agent session.

## Directory layout

```
~/.quecto/workspace/extensions/
  hello/
    extension.toml     # Required — tool manifest
    hello.sh           # The executable referenced by `command`
  weather/
    extension.toml
    weather.py
```

Each subdirectory of `extensions/` is treated as one extension. The directory must contain an `extension.toml` file. Directories without a manifest are silently skipped.

## Manifest reference

The `extension.toml` file defines how the tool appears to the LLM and how it is executed.

| Field | Required | Default | Description |
|---|---|---|---|
| `name` | Yes | — | Tool name (used in LLM tool calls). Must be unique across all extensions |
| `description` | Yes | — | Description shown to the LLM. Supports multi-line via triple-quoted strings (`"""`) |
| `parameters_schema` | Yes | — | JSON Schema string defining the tool's input parameters |
| `command` | Yes | — | Path to the executable, relative to the extension directory. Must start with `./` |
| `timeout_secs` | No | `30` | Maximum execution time in seconds before the process is killed |
| `system_prompt` | No | `None` | Text injected into the agent's system prompt when this extension is loaded |

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

## Script protocol

Extensions communicate via a simple stdin/stdout JSON protocol:

### Input

The tool arguments (as a JSON string matching your `parameters_schema`) are written to the script's **stdin**.

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
| `content` | `string` | The text result returned to the agent |
| `is_error` | `bool` | `true` if the result represents an error condition |

### Error handling

| Condition | Behavior |
|---|---|
| Non-zero exit code | Result is marked as error. `stderr` output is returned as the error content |
| Invalid JSON output | Result is marked as error with message: `"invalid output from extension '<name>': <stdout>"` |
| Timeout exceeded | Process group is killed (SIGKILL). Result: `"extension '<name>' timed out after <N>s"` |
| Output exceeds 1 MiB | Process is killed. Result: `"output exceeded 1MiB cap"` |

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

Extensions can contribute to the agent's system prompt via the `system_prompt` manifest field. All non-empty snippets from loaded extensions are collected and injected into a delimited section:

```
## Extensions
When looking up users, always verify the email format first.

Always format dates in ISO 8601.
## End Extensions
```

This gives extensions a way to provide behavioral instructions to the LLM alongside their tool definitions.

## Hot reload

Quecto watches extension directories for changes using fingerprint-based polling (file modification time + size). When a change is detected:

- **New extensions** are discovered and their tools become available
- **Removed extensions** have their tools unregistered
- **Modified manifests** cause the extension to be reloaded with the updated configuration

Core built-in tools (`bash`, `read`, `write`, `edit`, `ls`, `grep`, `find`, etc.) are never affected by extension reloads.

No restart is needed — changes take effect automatically.

## Security

Extensions run as subprocesses of the Quecto agent. Several security measures are enforced:

### Command path restrictions

- The `command` field **must** start with `./` (relative to the extension directory)
- Parent directory traversal (`..`) in command paths is rejected
- Absolute paths (e.g. `/usr/bin/env`) are rejected

```toml
# ✓ Valid
command = "./run.sh"
command = "./bin/tool"

# ✗ Rejected
command = "/usr/bin/python3"
command = "../../etc/passwd"
command = "curl"
```

### Output cap

Script stdout and stderr are each capped at **1 MiB**. If either stream exceeds this limit, the process (and its entire process group) is killed and the tool returns an error.

### Timeout enforcement

When a script exceeds `timeout_secs`, the entire process group is killed with `SIGKILL` (not just the lead process). This prevents orphaned child processes from lingering.

### Symlink rejection

Symlinked extension directories are skipped during discovery. This prevents extensions from pointing outside the trusted extensions directory.

### Tool name shadowing

Extension tools cannot shadow core built-in tools. If an extension defines a tool with the same name as a built-in (e.g. `bash`, `read`), it is rejected with a warning and not registered.

## Deduplication

If multiple extensions define tools with the same name, the **last one registered wins**. Extension registration order follows filesystem directory listing order, which may vary by platform. To avoid conflicts, use unique tool names.

## Troubleshooting

### Extension not appearing

1. Verify the directory structure: `~/.quecto/workspace/extensions/<name>/extension.toml`
2. Check that `extension.toml` has all required fields (`name`, `description`, `parameters_schema`, `command`)
3. Ensure `command` starts with `./`
4. Ensure the script is executable (`chmod +x`)
5. Check that the extension directory is not a symlink

### Tool returns "invalid output"

The script must output valid JSON with `content` and `is_error` fields to stdout. Common causes:
- Debug/log output mixed into stdout (redirect logs to stderr instead)
- Missing JSON encoding of special characters
- Script printing raw text instead of JSON

### Tool times out

Increase `timeout_secs` in the manifest, or optimize the script. The default is 30 seconds. On timeout, the entire process group is killed.

### Extension shadows a core tool

Check the agent logs for: `"extension tool rejected: shadows core tool"`. Rename the tool in your `extension.toml`.

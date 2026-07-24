# Extensions

Extensions add custom tools to Quecto beyond the built-in set (`bash`, `read`, `write`, `edit`, `ls`, `grep`, `find`, `rust_ast_graph`, `spawn`, `recall`, `workflow`). There are two extension mechanisms:

| Type | What it is | When to use |
|------|-----------|-------------|
| **Native** | Compiled-in Rust tools, enabled via `config.json` | Tools that ship with Quecto (e.g. `web_search`) |
| **UDS** | External processes that connect to the agent's Unix socket | Third-party tools, tools in other languages, stateful services |

Both types appear identically to the LLM — they show up in the tool list and are called the same way as built-in tools.

## Native extensions

Native extensions are Rust implementations compiled into the Quecto binary. They are registered conditionally at startup based on configuration and have zero overhead when disabled.

### Available native extensions

| Tool | Config key | Description |
|------|-----------|-------------|
| `web_search` | `tools.web.brave` / `tools.web.duckduckgo` | Search the web via Brave Search API or DuckDuckGo |
| `web_fetch` | `tools.web.fetch` | Fetch a URL and return its content as readable text |

When any web tool is enabled, they are registered together as a single `"web"` extension.

### Enabling web_search

Add to your `config.json`:

```json
{
  "tools": {
    "web": {
      "brave": {
        "enabled": true,
        "api_key": "YOUR_BRAVE_API_KEY"
      }
    }
  }
}
```

Or use DuckDuckGo (no API key required):

```json
{
  "tools": {
    "web": {
      "duckduckgo": {
        "enabled": true
      }
    }
  }
}
```

When both are enabled with a Brave API key, Brave is preferred. If Brave is enabled but no API key is set, DuckDuckGo is used as a fallback.

The API key can also be provided via the `BRAVE_API_KEY` environment variable.

### Enabling web_fetch

```json
{
  "tools": {
    "web": {
      "fetch": {
        "enabled": true,
        "max_response_kb": 32
      }
    }
  }
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable the `web_fetch` tool |
| `max_response_kb` | `32` | Maximum output size in KB returned to the LLM |

`web_fetch` strips HTML tags by default to produce clean text and save tokens. Pass `raw: true` in the tool call to get the original body (useful for JSON APIs or markdown files).

Safety limits:
- Only `http://` and `https://` URLs are allowed
- 10-second request timeout
- 5 MB raw download cap before text extraction
- Output truncated to `max_response_kb`

### Enabling both

```json
{
  "tools": {
    "web": {
      "brave": { "enabled": true, "api_key": "YOUR_KEY" },
      "fetch": { "enabled": true }
    }
  }
}
```

This registers a single `"web"` extension with both `web_search` and `web_fetch` tools. The agent can search for information and then fetch full page content from the results.

### Behavior

- Native extensions are loaded once at agent startup
- They share the process's HTTP client and connection pool
- Child agents (via `spawn`) inherit the same config, so they get the same native extensions
- Native extensions cannot be added or removed at runtime — changes require restarting the agent

## UDS extensions

UDS extensions are external processes that connect to the agent's Unix domain socket and register tools via the framed JSON protocol. This is the same socket used by TUIs, IDE plugins, and web UIs.

### How it works

1. Start the agent in UDS mode: `quecto agent --mode uds`
2. An external process connects to the socket
3. The process sends `register_tools` to register its tools
4. When the LLM calls a registered tool, the agent sends `execute_tool` to the process
5. The process sends `tool_result` back with the result
6. On disconnect, tools are automatically unregistered

### Protocol

All communication uses length-prefixed UTF-8 JSON frames over a Unix domain socket.

#### Registering tools

```json
{
  "type": "register_tools",
  "id": "rt-1",
  "tools": [
    {
      "name": "weather",
      "description": "Get current weather for a city",
      "parametersSchema": "{\"type\":\"object\",\"properties\":{\"city\":{\"type\":\"string\"}},\"required\":[\"city\"]}"
    }
  ]
}
```

**Response:**

```json
{"type":"response","id":"rt-1","command":"register_tools","success":true,"data":{"registered":["weather"]}}
```

On failure (e.g. shadowing a core tool):

```json
{"type":"response","id":"rt-1","command":"register_tools","success":false,"error":"tool 'bash' shadows a core tool"}
```

**Side effect:** An `extensions_changed` event is broadcast to all connected clients.

#### Receiving execution requests

When the LLM calls a UDS-registered tool, the agent sends an `execute_tool` event **only to the client that registered it** (routed, not broadcast):

```json
{
  "type": "execute_tool",
  "toolCallId": "uds-0000000abc-00000001",
  "toolName": "weather",
  "arguments": "{\"city\":\"London\"}"
}
```

#### Returning results

The extension process responds with `tool_result`:

```json
{
  "type": "tool_result",
  "toolCallId": "uds-0000000abc-00000001",
  "content": "London: 18°C, partly cloudy",
  "isError": false
}
```

| Field | Type | Description |
|-------|------|-------------|
| `toolCallId` | string | Must match the `toolCallId` from `execute_tool` |
| `content` | string | Result text returned to the LLM |
| `isError` | boolean | `true` if the result represents an error |

#### Unregistering tools

```json
{
  "type": "unregister_tools",
  "id": "ut-1",
  "tools": ["weather"]
}
```

**Response:**

```json
{"type":"response","id":"ut-1","command":"unregister_tools","success":true,"data":{"unregistered":["weather"]}}
```

### Lifecycle

- **Connect = available:** Tools are available as soon as `register_tools` succeeds
- **Disconnect = auto-unregister:** When a client disconnects, all its tools are immediately removed and an `extensions_changed` event is broadcast
- **Disconnect during execution:** If a client disconnects while a tool call is pending, the agent receives an error result: `"Extension disconnected during execution of tool '<name>'"`
- **Timeout:** If a tool doesn't respond within 30 seconds, the agent returns: `"Extension timed out after 30s executing tool '<name>'"`
- **Re-registration:** Sending `register_tools` for an already-registered tool updates its definition (idempotent)
- **Multiple clients:** Multiple extension processes can connect simultaneously, each registering different tools

### Shadow protection

Extension tools (both native and UDS) cannot shadow built-in tools. If an extension tries to register a tool with the same name as a built-in (`bash`, `read`, `write`, `edit`, `ls`, `grep`, `find`, `rust_ast_graph`, `spawn`, `recall`, `workflow`), the registration is rejected.

### Example: Rust UDS extension

A minimal Rust program that registers a `weather` tool:

```rust
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

fn main() {
    let socket_path = std::env::args().nth(1).expect("usage: weather <socket>");
    let stream = UnixStream::connect(&socket_path).expect("connect failed");
    let mut writer = stream.try_clone().unwrap();
    let reader = BufReader::new(stream);

    // Register our tool
    let register = r#"{"type":"register_tools","id":"r1","tools":[{"name":"weather","description":"Get weather for a city","parametersSchema":"{\"type\":\"object\",\"properties\":{\"city\":{\"type\":\"string\"}},\"required\":[\"city\"]}"}]}"#;
    writeln!(writer, "{register}").unwrap();

    // Listen for events
    for line in reader.lines() {
        let line = line.unwrap();
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();

        match event["type"].as_str() {
            Some("execute_tool") => {
                let call_id = event["toolCallId"].as_str().unwrap();
                let args = event["arguments"].as_str().unwrap();
                // Parse args, do work, return result
                let result = format!(
                    r#"{{"type":"tool_result","toolCallId":"{call_id}","content":"22°C, sunny","isError":false}}"#
                );
                writeln!(writer, "{result}").unwrap();
            }
            _ => {} // ignore other events
        }
    }
}
```

### Example: shell + socat

```bash
#!/bin/bash
SOCKET="$1"

# Register a tool
echo '{"type":"register_tools","tools":[{"name":"greet","description":"Greet someone","parametersSchema":"{\"type\":\"object\",\"properties\":{\"name\":{\"type\":\"string\"}},\"required\":[\"name\"]}"}]}' \
  | socat - UNIX-CONNECT:"$SOCKET"

# For a persistent connection that handles execute_tool:
socat - UNIX-CONNECT:"$SOCKET" | while IFS= read -r line; do
  type=$(echo "$line" | jq -r '.type')
  if [ "$type" = "execute_tool" ]; then
    call_id=$(echo "$line" | jq -r '.toolCallId')
    args=$(echo "$line" | jq -r '.arguments')
    name=$(echo "$args" | jq -r '.name // "World"')
    echo "{\"type\":\"tool_result\",\"toolCallId\":\"$call_id\",\"content\":\"Hello, $name!\",\"isError\":false}"
  fi
done
```

> **Note:** The shell example requires `socat` and `jq`. For production extensions, use a proper client library or a compiled binary.

## Querying extensions

Connected clients can query the current extension list:

```json
{"type":"get_extensions","id":"ge-1"}
```

Response:

```json
{
  "type": "response",
  "id": "ge-1",
  "command": "get_extensions",
  "success": true,
  "data": {
    "extensions": [
      {"name": "web_search", "description": "Search the web using Brave Search or DuckDuckGo"},
      {"name": "weather", "description": "Get current weather for a city"}
    ]
  }
}
```

This includes both native and UDS-registered extensions.

## System prompt injection

Native extensions can contribute text to the agent's system prompt. This is configured in the extension implementation (not via config). Currently no native extensions use this feature, but the mechanism exists for future extensions that need to influence LLM behavior.

## Choosing between native and UDS extensions

| Consideration | Native | UDS |
|--------------|--------|-----|
| **Language** | Rust only | Any language |
| **Deployment** | Compiled into binary | Separate process |
| **Overhead** | Zero when disabled | Process + socket I/O |
| **Lifecycle** | Config change + restart | Connect/disconnect |
| **State** | Shared process state | Own process state |
| **Dependencies** | None (single binary) | May need runtime |
| **Use case** | First-party tools | Third-party / external |

## See also

- Getting Started (`docs {"name":"getting-started"}`) — quickstart guide for UDS agent integration
- UDS Protocol Reference (`docs {"name":"uds-protocol"}`) — full protocol specification
- Subagents (`docs {"name":"subagents"}`) — spawning child agent processes
- Workflow Automation (`docs {"name":"workflow"}`) — structured development process
- Disabling Tools (`docs {"name":"disable-tools"}`) — restricting which tools the agent can access
- [Configuration](../README.md) — `config.json` reference

# Security Policy

Quecto is an agentic coding harness. It can execute local commands, call model providers, connect to MCP tools, proxy APIs, and manage runtimes. Treat it as powerful developer infrastructure, not as a sandbox for untrusted input by default.

## Reporting vulnerabilities

Please report suspected vulnerabilities privately instead of opening a public issue.

If GitHub private vulnerability reporting is enabled for this repository, use that feature. Otherwise, contact the project maintainers through the private channel listed by the repository owner.

Include as much detail as you can safely share:

- affected package, binary, version, commit, or deployment mode;
- reproduction steps or proof of concept;
- expected impact and attacker capabilities;
- whether credentials, tokens, files, network access, or runtime isolation are involved;
- any suggested mitigation.

Do not include real secrets in reports. Redact tokens and credentials before sharing logs or screenshots.

## Supported versions

This project is pre-1.0 in several companion crates and moves quickly. Security fixes are expected to target the current `master` branch and the latest released package versions unless maintainers explicitly announce a supported maintenance branch.

## Scope

Security-sensitive areas include, but are not limited to:

- provider API keys, OAuth tokens, credential storage, import/export, and refresh flows;
- command execution through `bash` or tool adapters;
- filesystem access and sandbox/path restrictions;
- subagent spawning and container runtime configuration;
- UDS protocol parsing, message framing, bounded reads, and recovery APIs;
- HTTP/WebSocket gateway authentication, CORS, proxying, and event exposure;
- MCP tool registration/proxying and tool argument handling;
- runtime-manager authentication, Kubernetes pod manifests, credential sync, and API proxying;
- audit logs, session history, tool output, error reporting, and secret redaction.

## Security model and limitations

### Local command execution

The harness can run commands as the invoking user. The `bash` tool is not a strong security boundary: commands may read files available to that user, access local credentials, and reach the network unless the surrounding deployment restricts them. Do not run untrusted prompts, tools, repositories, or commands in an environment that contains secrets you cannot risk exposing.

For untrusted work, use external isolation such as non-root containers, minimal or read-only mounts, cgroup limits, process limits, and restrictive network policy.

### Credentials and secrets

- Prefer the Quecto credential store, provider environment variables, or deployment secret stores.
- Never commit real `.env` files, provider keys, OAuth tokens, cookies, private keys, certificates, kubeconfigs, or cloud credentials.
- Keep secrets out of prompts, logs, screenshots, issue comments, docs, and tests.
- Rotate any credential that may have been committed, logged, pasted into a model conversation, or exposed to an untrusted tool.

### MCP and external tools

MCP tools and extension bridges may call third-party services or perform actions outside the repository. Use least-privilege MCP tokens, allowlist only the tools needed for an agent, and avoid passing actor identity or authorization decisions through model-controlled arguments.

### HTTP/WebSocket and runtime deployments

Do not expose `quecto-api` or `quecto-runtime-manager` to untrusted networks without an appropriate authentication, authorization, TLS, and network-isolation layer. Review CORS, bearer-token, proxy, and credential-sync behavior before deployment.

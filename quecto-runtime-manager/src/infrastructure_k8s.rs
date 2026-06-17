use super::*;

pub(super) fn runtime_pod_name(runtime_ref: &str) -> String {
    format!("quecto-runtime-{}", runtime_ref)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(63)
        .collect()
}

pub(super) fn runtime_target_url(target_base: &str, path: &str, query: Option<&str>) -> String {
    match query {
        Some(query) if !query.is_empty() => format!("{target_base}/{path}?{query}"),
        _ => format!("{target_base}/{path}"),
    }
}

pub(super) fn runtime_target_base(runtime: &ManagedRuntime) -> String {
    match runtime.pod_ip.as_deref() {
        Some(pod_ip) => format!("http://{pod_ip}:8080"),
        None => format!("http://127.0.0.1:{}", runtime.port),
    }
}

pub(super) fn runtime_target_ws(runtime: &ManagedRuntime) -> String {
    match runtime.pod_ip.as_deref() {
        Some(pod_ip) => format!("ws://{pod_ip}:8080/ws"),
        None => format!("ws://127.0.0.1:{}/ws", runtime.port),
    }
}

pub(super) fn agent_session_key(session_name: &str) -> String {
    format!("cli:{session_name}")
}

pub(super) fn runtime_workdir(body: &EnsureRuntimeRequest) -> String {
    body.repository
        .as_ref()
        .and_then(|repo| repo.working_dir.clone())
        .unwrap_or_else(|| "/home/appuser/workspace/repo".to_string())
}

pub(super) fn runtime_bootstrap_command() -> &'static str {
    r#"set -eu
mkdir -p /home/appuser/.config/gh /home/appuser/workspace /home/appuser/.quecto/runtime-configs
verify_runtime_toolchain() {
  command -v elixir >/dev/null 2>&1
  command -v mix >/dev/null 2>&1
  command -v node >/dev/null 2>&1
  command -v npm >/dev/null 2>&1
  command -v bun >/dev/null 2>&1
  command -v git >/dev/null 2>&1
  command -v gh >/dev/null 2>&1 || true
  command -v psql >/dev/null 2>&1
}

start_postgres_if_available() {
  if ! command -v initdb >/dev/null 2>&1 || ! command -v pg_ctl >/dev/null 2>&1; then
    return 0
  fi

  export PGDATA="${PGDATA:-/home/appuser/.quecto/postgres}"
  export PGHOST="${PGHOST:-127.0.0.1}"
  export PGPORT="${PGPORT:-5432}"
  export DATABASE_URL="${DATABASE_URL:-postgres://jarga:jarga@localhost/jarga_test}"

  if [ ! -s "$PGDATA/PG_VERSION" ]; then
    mkdir -p "$PGDATA"
    initdb -D "$PGDATA" --auth=trust >/tmp/quecto-postgres-initdb.log 2>&1
    printf "listen_addresses = '127.0.0.1'\nmax_connections = 400\n" >> "$PGDATA/postgresql.conf"
  fi

  pg_ctl -D "$PGDATA" -l /tmp/quecto-postgres.log -o "-c listen_addresses=127.0.0.1 -c port=$PGPORT -c unix_socket_directories=/tmp" start >/tmp/quecto-postgres-start.log 2>&1 || true
  until pg_isready -h "$PGHOST" -p "$PGPORT" >/dev/null 2>&1; do sleep 0.2; done
  createuser -h "$PGHOST" -p "$PGPORT" jarga >/dev/null 2>&1 || true
  psql -h "$PGHOST" -p "$PGPORT" -d postgres -v ON_ERROR_STOP=1 -q -c "alter user jarga with password 'jarga';" >/dev/null 2>&1 || true
  createdb -h "$PGHOST" -p "$PGPORT" -O jarga jarga_test >/dev/null 2>&1 || true
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -q -c "select 1;" >/tmp/quecto-postgres-ready.log 2>&1
}

setup_project_dependencies() {
  if [ "${QUECTO_SKIP_PROJECT_BOOTSTRAP:-false}" = "true" ]; then
    return 0
  fi

  if [ -f mix.exs ]; then
    mix deps.get
  fi

  find . -maxdepth 5 -name package.json \
    -not -path '*/node_modules/*' \
    -not -path './deps/*' \
    -not -path './_build/*' \
    -print | while IFS= read -r package_json; do
      package_dir="$(dirname "$package_json")"
      (
        cd "$package_dir"
        if [ -f bun.lock ] || [ -f bun.lockb ]; then
          bun install
        elif [ -f package-lock.json ] || [ -f npm-shrinkwrap.json ]; then
          npm ci
        else
          npm install
        fi
      )
    done
}

verify_runtime_toolchain
start_postgres_if_available
if [ -n "${GH_TOKEN:-}" ]; then
  printf '%s' "$GH_TOKEN" | gh auth login --with-token >/tmp/gh-auth.log 2>&1 || true
  gh auth setup-git >/tmp/gh-setup-git.log 2>&1 || true
  git config --global url."https://x-access-token:${GH_TOKEN}@github.com/".insteadOf "https://github.com/"
fi
if [ -n "${QUECTO_REPO_URL:-}" ]; then
  rm -rf "$QUECTO_WORKDIR"
  mkdir -p "$(dirname "$QUECTO_WORKDIR")"
  if [ -n "${QUECTO_REPO_REF:-}" ]; then
    git clone --branch "$QUECTO_REPO_REF" "$QUECTO_REPO_URL" "$QUECTO_WORKDIR"
  else
    git clone "$QUECTO_REPO_URL" "$QUECTO_WORKDIR"
  fi
else
  mkdir -p "$QUECTO_WORKDIR"
fi
cd "$QUECTO_WORKDIR"
setup_project_dependencies
if [ -n "${QUECTO_WORKFLOW_CONFIG_JSON:-}" ]; then
  printf '%s' "$QUECTO_WORKFLOW_CONFIG_JSON" > "$QUECTO_RUNTIME_CONFIG_PATH"
  if [ -n "${QUECTO_WORKFLOW_CONFIG_PATH:-}" ]; then
    mkdir -p "$(dirname "$QUECTO_WORKFLOW_CONFIG_PATH")"
    cp "$QUECTO_RUNTIME_CONFIG_PATH" "$QUECTO_WORKFLOW_CONFIG_PATH"
  fi
else
  cp /home/appuser/.quecto/config.json "$QUECTO_RUNTIME_CONFIG_PATH"
fi
exec quecto agent --config "$QUECTO_RUNTIME_CONFIG_PATH" --mode uds --no-sandbox --network --workflow --workflow-guards --socket /shared/quecto.sock --session "$QUECTO_SESSION_NAME" --persist --system "$(cat /etc/quecto/workflow-agent-system-prompt.txt)"
"#
}

pub(super) fn runtime_workflow_config_json(value: &Value) -> String {
    let mut config = value.clone();

    if let Value::Object(root) = &mut config {
        let tools = root.entry("tools").or_insert_with(|| json!({}));
        if let Value::Object(tools_map) = tools {
            let exec = tools_map.entry("exec").or_insert_with(|| json!({}));
            if let Value::Object(exec_map) = exec {
                exec_map
                    .entry("isolation")
                    .or_insert_with(|| Value::String("native".to_string()));
                exec_map
                    .entry("allow_native_fallback")
                    .or_insert_with(|| Value::Bool(true));
            }
        }
    }

    config.to_string()
}

pub(super) fn runtime_pod_manifest(
    config: &ManagerConfig,
    body: &EnsureRuntimeRequest,
    runtime_ref: &str,
    pod_name: &str,
) -> Value {
    let image_pull_secrets = config
        .pod_pull_secret
        .as_ref()
        .map(|name| json!([{ "name": name }]))
        .unwrap_or_else(|| json!([]));
    let workdir = runtime_workdir(body);
    let repo_url = body
        .repository
        .as_ref()
        .map(|repo| repo.url.clone())
        .unwrap_or_default();
    let repo_ref = body
        .repository
        .as_ref()
        .and_then(|repo| repo.ref_name.clone())
        .unwrap_or_default();
    let workflow_config_path = body
        .workflow
        .as_ref()
        .and_then(|workflow| workflow.config_path.clone())
        .unwrap_or_else(|| "workflow-config.json".to_string());
    let runtime_config_path = format!(
        "/home/appuser/.quecto/runtime-configs/{}.json",
        runtime_ref
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            })
            .collect::<String>()
    );
    let workflow_config_json = body
        .workflow
        .as_ref()
        .and_then(|workflow| workflow.config_json.as_ref())
        .map(runtime_workflow_config_json)
        .unwrap_or_default();
    let workflow_template = body
        .workflow
        .as_ref()
        .and_then(|workflow| workflow.template.clone())
        .unwrap_or_default();
    let workflow_stop_after = body
        .workflow
        .as_ref()
        .and_then(|workflow| workflow.stop_after_step_key.clone())
        .unwrap_or_default();

    json!({
      "apiVersion": "v1",
      "kind": "Pod",
      "metadata": {
        "name": pod_name,
        "namespace": config.kubernetes_namespace,
        "labels": {
          "app": "quecto-runtime",
          "runtime-ref": runtime_ref,
          "managed-by": "quecto-runtime-manager"
        }
      },
      "spec": {
        "restartPolicy": "Never",
        "securityContext": { "fsGroup": 1000 },
        "imagePullSecrets": image_pull_secrets,
        "initContainers": [
          {
            // Seed credentials.json from the read-only Secret into the writable
            // quecto-data volume. The runtime refreshes expired OAuth tokens and
            // persists them back to this file; mounting the Secret directly would
            // make it read-only and silently drop refreshed tokens, so the agent
            // would keep reading the stale (expired) token and report "token
            // expired" on every subsequent request.
            "name": "seed-credentials",
            "image": config.pod_image,
            "imagePullPolicy": "Always",
            "command": ["/bin/sh", "-c"],
            "args": ["cp /etc/quecto/seed/credentials.json /home/appuser/.quecto/credentials.json && chmod 600 /home/appuser/.quecto/credentials.json"],
            "volumeMounts": [
              { "name": "quecto-data", "mountPath": "/home/appuser/.quecto" },
              { "name": "credentials", "mountPath": "/etc/quecto/seed", "readOnly": true }
            ]
          }
        ],
        "containers": [
          {
            "name": "quecto",
            "image": config.pod_image,
            "imagePullPolicy": "Always",
            "command": ["/bin/sh", "-c"],
            "args": [runtime_bootstrap_command()],
            "env": [
              { "name": "QUECTO_BASE_DIR", "value": "/home/appuser/.quecto" },
              { "name": "QUECTO_AGENTS_DEFAULTS_WORKSPACE", "value": workdir },
              { "name": "QUECTO_SESSION_NAME", "value": body.session_name },
              { "name": "QUECTO_SESSION_KEY", "value": body.session_key },
              { "name": "QUECTO_MAX_CONTEXT_TOKENS", "value": "250000" },
              { "name": "QUECTO_REPO_URL", "value": repo_url },
              { "name": "QUECTO_REPO_REF", "value": repo_ref },
              { "name": "QUECTO_WORKDIR", "value": workdir },
              { "name": "DATABASE_URL", "value": "postgres://jarga:jarga@localhost/jarga_test" },
              { "name": "QUECTO_RUNTIME_CONFIG_PATH", "value": runtime_config_path },
              { "name": "QUECTO_WORKFLOW_CONFIG_PATH", "value": workflow_config_path },
              { "name": "QUECTO_WORKFLOW_CONFIG_JSON", "value": workflow_config_json },
              { "name": "QUECTO_WORKFLOW_TEMPLATE", "value": workflow_template },
              { "name": "QUECTO_WORKFLOW_STOP_AFTER", "value": workflow_stop_after },
              { "name": "GH_CONFIG_DIR", "value": "/home/appuser/.config/gh" },
              { "name": "GH_TOKEN", "valueFrom": { "secretKeyRef": { "name": "quecto-github-app-token", "key": "token", "optional": true } } },
              { "name": "GITHUB_TOKEN", "valueFrom": { "secretKeyRef": { "name": "quecto-github-app-token", "key": "token", "optional": true } } },
              { "name": "QUECTO_CREDENTIAL_SYNC_URL", "value": format!("{}/credentials", config.manager_self_url.trim_end_matches('/')) },
              { "name": "QUECTO_CREDENTIAL_SYNC_TOKEN", "value": config.manager_token.clone().unwrap_or_default() },
              { "name": "RUST_LOG", "value": "info,quecto=debug" }
            ],
            "volumeMounts": [
              { "name": "shared-socket", "mountPath": "/shared" },
              { "name": "quecto-data", "mountPath": "/home/appuser/.quecto" },
              { "name": "config", "mountPath": "/home/appuser/.quecto/config.json", "subPath": "config.json", "readOnly": true },
              { "name": "prompt", "mountPath": "/etc/quecto/workflow-agent-system-prompt.txt", "subPath": "workflow-agent-system-prompt.txt", "readOnly": true },
              { "name": "prompt", "mountPath": "/etc/quecto/agent-workflow-tools.md", "subPath": "agent-workflow-tools.md", "readOnly": true }
            ],
            "resources": {
              "requests": { "memory": "1Gi", "cpu": "500m" },
              "limits": { "memory": "4Gi", "cpu": "2000m" }
            }
          },
          {
            "name": "quecto-api",
            "image": config.pod_image,
            "imagePullPolicy": "Always",
            "command": ["/bin/sh", "-c"],
            "args": ["while [ ! -S /shared/quecto.sock ]; do sleep 0.2; done; exec quecto-api --socket /shared/quecto.sock --host 0.0.0.0 --port 8080"],
            "ports": [{ "containerPort": 8080 }],
            "readinessProbe": { "httpGet": { "path": "/health", "port": 8080 }, "periodSeconds": 1, "failureThreshold": 90 },
            "env": [
              { "name": "QUECTO_BASE_DIR", "value": "/home/appuser/.quecto" },
              { "name": "QUECTO_SESSION_KEY", "value": agent_session_key(&body.session_name) }
            ],
            "volumeMounts": [
              { "name": "shared-socket", "mountPath": "/shared" },
              { "name": "quecto-data", "mountPath": "/home/appuser/.quecto", "readOnly": true }
            ],
            "resources": {
              "requests": { "memory": "32Mi", "cpu": "25m" },
              "limits": { "memory": "128Mi", "cpu": "250m" }
            }
          }
        ],
        "volumes": [
          { "name": "shared-socket", "emptyDir": {} },
          { "name": "quecto-data", "emptyDir": {} },
          { "name": "config", "secret": { "secretName": "quecto-secrets", "items": [{ "key": "config.json", "path": "config.json" }] } },
          { "name": "credentials", "secret": { "secretName": "quecto-secrets", "items": [{ "key": "credentials.json", "path": "credentials.json" }] } },
          { "name": "prompt", "configMap": { "name": "quecto-config", "items": [
            { "key": "workflow-agent-system-prompt.txt", "path": "workflow-agent-system-prompt.txt" },
            { "key": "agent-workflow-tools.md", "path": "agent-workflow-tools.md" }
          ] } }
        ]
      }
    })
}

pub(super) async fn create_runtime_pod(
    state: &AppState,
    manifest: &Value,
) -> Result<(), ManagerError> {
    let url = kubernetes_url(&state.config, "/pods");
    let response = state
        .http
        .post(url)
        .bearer_auth(kubernetes_token().await?)
        .json(manifest)
        .send()
        .await?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(ManagerError::KubernetesApi(response.status().as_u16()))
    }
}

pub(super) async fn runtime_pod_status(
    state: &AppState,
    pod_name: &str,
) -> Result<Value, ManagerError> {
    let url = kubernetes_url(&state.config, &format!("/pods/{pod_name}"));
    let response = state
        .http
        .get(url)
        .bearer_auth(kubernetes_token().await?)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(ManagerError::KubernetesApi(response.status().as_u16()));
    }

    let pod: Value = response.json().await?;
    let phase = pod
        .pointer("/status/phase")
        .and_then(Value::as_str)
        .unwrap_or("Unknown");
    let containers = pod
        .pointer("/status/containerStatuses")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let terminated = containers.iter().find_map(|container| {
        let state = container.get("state")?;
        let terminated = state.get("terminated")?;
        Some(json!({
            "container": container.get("name").and_then(Value::as_str).unwrap_or("unknown"),
            "reason": terminated.get("reason").and_then(Value::as_str).unwrap_or("terminated"),
            "exit_code": terminated.get("exitCode").and_then(Value::as_i64).unwrap_or_default(),
            "message": terminated.get("message").and_then(Value::as_str).unwrap_or(""),
            "started_at": terminated.get("startedAt").cloned().unwrap_or(Value::Null),
            "finished_at": terminated.get("finishedAt").cloned().unwrap_or(Value::Null)
        }))
    });
    let healthy = phase == "Running" && terminated.is_none();

    Ok(json!({
        "data": {
            "healthy": healthy,
            "phase": phase,
            "terminated": terminated,
            "containers": containers
        }
    }))
}

pub(super) async fn delete_runtime_pod(
    state: &AppState,
    pod_name: &str,
) -> Result<(), ManagerError> {
    let url = kubernetes_url(&state.config, &format!("/pods/{pod_name}"));
    let response = state
        .http
        .delete(url)
        .bearer_auth(kubernetes_token().await?)
        .send()
        .await?;

    if response.status().is_success() || response.status().as_u16() == 404 {
        Ok(())
    } else {
        Err(ManagerError::KubernetesApi(response.status().as_u16()))
    }
}

pub(super) async fn wait_for_runtime_pod_ready(
    state: &AppState,
    pod_name: &str,
    timeout: Duration,
) -> Result<String, ManagerError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let url = kubernetes_url(&state.config, &format!("/pods/{pod_name}"));
        let response = state
            .http
            .get(url)
            .bearer_auth(kubernetes_token().await?)
            .send()
            .await?;

        if response.status().is_success() {
            let pod: Value = response.json().await?;
            if pod_ready(&pod) {
                if let Some(pod_ip) = pod.pointer("/status/podIP").and_then(Value::as_str) {
                    return Ok(pod_ip.to_string());
                }
            }
        }

        sleep(Duration::from_millis(500)).await;
    }

    Err(ManagerError::RuntimeUnhealthy)
}

pub(super) fn pod_ready(pod: &Value) -> bool {
    pod.pointer("/status/conditions")
        .and_then(Value::as_array)
        .is_some_and(|conditions| {
            conditions.iter().any(|condition| {
                condition.get("type").and_then(Value::as_str) == Some("Ready")
                    && condition.get("status").and_then(Value::as_str) == Some("True")
            })
        })
}

use super::*;

// ===========================================================================
// Sandbox Hardening Steps
// ===========================================================================

#[given("a sandboxed workspace at a temporary directory")]
fn given_sandboxed_workspace_temp(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    world.sandbox = Some(Sandbox::new(Some(ws.clone()), true));
    world.tool_workspace = Some(ws);
    world._extra_temp_dirs.push(td);
}

#[given(expr = "a symlink {string} in the workspace pointing to {string}")]
fn given_symlink_in_workspace(world: &mut QuectoWorld, link_name: String, target: String) {
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    let link_path = ws.join(&link_name);
    // If target is relative, it should be relative to the workspace
    let target_path = if target.starts_with('/') {
        PathBuf::from(&target)
    } else {
        ws.join(&target)
    };
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target_path, &link_path).unwrap_or_else(|e| {
        panic!(
            "failed to create symlink {} -> {}: {}",
            link_path.display(),
            target_path.display(),
            e
        )
    });
}

#[given(expr = "a file {string} exists in the workspace")]
fn given_file_exists_in_workspace(world: &mut QuectoWorld, filename: String) {
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    let path = ws.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, "test content").expect("write file");
}

#[when(expr = "the agent tries to validate path {string} resolved against the workspace")]
fn when_validate_path_resolved(world: &mut QuectoWorld, path: String) {
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    let full_path = ws.join(&path);
    world.validation_result = Some(
        sb.validate_path(full_path.to_str().unwrap())
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

// --- Allowlist steps ---

#[given(expr = "a workspace at {string}")]
fn given_workspace_at(world: &mut QuectoWorld, path: String) {
    world.tool_workspace = Some(PathBuf::from(path));
}

#[given("the agent has no command allowlist configured")]
fn given_agent_no_allowlist(world: &mut QuectoWorld) {
    let sb = Sandbox::new(world.tool_workspace.clone(), false);
    world.sandbox = Some(sb);
}

#[given(expr = "the agent has command allowlist {string}")]
fn given_agent_command_allowlist(world: &mut QuectoWorld, allowlist: String) {
    let commands: Vec<String> = if allowlist.is_empty() {
        vec![]
    } else {
        allowlist
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let sb =
        Sandbox::new(world.tool_workspace.clone(), false).with_command_allowlist(Some(commands));
    world.sandbox = Some(sb);
}

#[when(expr = "the agent validates the command {string}")]
fn when_agent_validates_command(world: &mut QuectoWorld, command: String) {
    let sb = world.sandbox.as_ref().expect("sandbox not set");
    world.validation_result = Some(
        sb.validate_command(&command)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

#[then(expr = "the command should be rejected with {string}")]
fn then_command_rejected_with(world: &mut QuectoWorld, expected: String) {
    let result = world
        .validation_result
        .as_ref()
        .expect("no validation result");
    let err = result
        .as_ref()
        .expect_err("expected command to be rejected");
    assert!(
        err.to_string().contains(&expected),
        "expected error to contain '{}', got: {}",
        expected,
        err
    );
}

#[then("the command should be permitted")]
fn then_command_permitted(world: &mut QuectoWorld) {
    let result = world
        .validation_result
        .as_ref()
        .expect("no validation result");
    assert!(
        result.is_ok(),
        "expected command to be permitted, got: {:?}",
        result
    );
}

#[given(expr = "a sandbox with command allowlist {string}")]
fn given_sandbox_with_allowlist(world: &mut QuectoWorld, allowlist: String) {
    let commands: Vec<String> = if allowlist.is_empty() {
        vec![]
    } else {
        allowlist.split(',').map(|s| s.trim().to_string()).collect()
    };
    let sb = Sandbox::new(None, false).with_command_allowlist(Some(commands));
    world.sandbox = Some(sb);
}

#[given("a sandbox without a command allowlist")]
fn given_sandbox_without_allowlist(world: &mut QuectoWorld) {
    let sb = Sandbox::new(None, false);
    // command_allowlist defaults to None
    world.sandbox = Some(sb);
}

// --- Exec timeout steps ---

#[given(expr = "an exec tool with a timeout of {int} seconds")]
fn given_exec_tool_with_timeout(world: &mut QuectoWorld, timeout: u64) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), false);
    let tool = ExecTool::with_timeout(
        Arc::new(ws.clone()),
        Arc::new(sandbox),
        std::time::Duration::from_secs(timeout),
    );
    world.exec_tool = Some(Arc::new(tool));
    world.tool_workspace = Some(ws);
    world._extra_temp_dirs.push(td);
}

#[given("an exec tool with no explicit timeout")]
fn given_exec_tool_no_timeout(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), false);
    let tool = ExecTool::new(Arc::new(ws.clone()), Arc::new(sandbox));
    world.exec_tool = Some(Arc::new(tool));
    world.tool_workspace = Some(ws);
    world._extra_temp_dirs.push(td);
}

#[when(expr = "the agent executes command {string}")]
fn when_agent_executes_command(world: &mut QuectoWorld, command: String) {
    let tool = world.exec_tool.as_ref().expect("exec tool not set");
    let args = serde_json::json!({"command": command}).to_string();
    let env_vars = world.exec_env_vars.clone();

    let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
        if env_vars.is_empty() {
            tool.execute(&args).await
        } else {
            tool.execute_with_env(&args, &env_vars).await
        }
    });

    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[then("the tool result should be an error")]
fn then_tool_result_is_error(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    if let Ok(tr) = result {
        assert!(
            tr.is_error,
            "expected tool result to be an error, got success: {}",
            tr.content
        );
    }
    // Err(_) is also an error — nothing to assert
}

#[then(expr = "the tool result should not contain {string}")]
fn then_tool_result_not_contains(world: &mut QuectoWorld, unexpected: String) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            !tr.content.contains(&unexpected),
            "expected tool result NOT to contain '{}', got: {}",
            unexpected,
            tr.content
        ),
        Err(e) => assert!(
            !e.contains(&unexpected),
            "expected error NOT to contain '{}', got: {}",
            unexpected,
            e
        ),
    }
}

#[then(expr = "the exec tool should have a default timeout of {int} seconds")]
fn then_exec_tool_default_timeout(world: &mut QuectoWorld, expected: u64) {
    let tool = world.exec_tool.as_ref().expect("exec tool not set");
    let actual = tool.timeout().as_secs();
    assert_eq!(
        actual, expected,
        "expected default timeout {}s, got {}s",
        expected, actual
    );
}

#[then("the exec tool should have no timeout")]
fn then_exec_tool_no_timeout(world: &mut QuectoWorld) {
    let tool = world.exec_tool.as_ref().expect("exec tool not set");
    assert_eq!(
        tool.timeout(),
        std::time::Duration::MAX,
        "expected no timeout (Duration::MAX)"
    );
}

// --- Env sanitization steps ---

#[given("an exec tool in a sandboxed workspace")]
fn given_exec_tool_in_sandbox(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), false);
    let tool = ExecTool::new(Arc::new(ws.clone()), Arc::new(sandbox));
    world.exec_tool = Some(Arc::new(tool));
    world.tool_workspace = Some(ws);
    world.exec_env_vars.clear();
    world._extra_temp_dirs.push(td);
}

#[given(expr = "the environment contains {string} set to {string}")]
fn given_exec_env_var(world: &mut QuectoWorld, key: String, value: String) {
    world.exec_env_vars.insert(key, value);
}

// --- Credential file permission steps ---

#[given("a credential store at a temporary directory")]
fn given_credential_store_at_temp(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let base = td.path().to_path_buf();
    world.credential_store = Some(CredentialStore::new(&base));
    world._extra_temp_dirs.push(td);
}

#[given(expr = "the credentials file exists with permissions {int}")]
fn given_credentials_file_with_permissions(world: &mut QuectoWorld, perms: u32) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    // Store a dummy credential to create the file
    store
        .store(Credential {
            provider: "dummy".to_string(),
            token: "dummy".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
    // Now change the permissions to the specified value (interpret as octal)
    let octal_perms = u32::from_str_radix(&format!("{}", perms), 8)
        .unwrap_or_else(|_| panic!("invalid octal permissions: {}", perms));
    let cred_path = store.path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(octal_perms);
        std::fs::set_permissions(cred_path, permissions).expect("set permissions");
    }
}

#[then(expr = "the credentials file should have permissions {int}")]
fn then_credentials_file_permissions(world: &mut QuectoWorld, expected: u32) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    // Interpret the expected value as octal (e.g., 0600 -> 0o600 = 384 decimal)
    let octal_expected = u32::from_str_radix(&format!("{}", expected), 8)
        .unwrap_or_else(|_| panic!("invalid octal permissions: {}", expected));
    let cred_path = store.path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(cred_path)
            .unwrap_or_else(|e| panic!("failed to read metadata for {:?}: {}", cred_path, e));
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(
            mode, octal_expected,
            "expected permissions {:04o}, got {:04o}",
            octal_expected, mode
        );
    }
}

// ===========================================================================

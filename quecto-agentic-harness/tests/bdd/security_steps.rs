use super::*;

// ===========================================================================
// Security / Sandbox Steps
// ===========================================================================

#[given(expr = "a sandboxed workspace at {string}")]
fn given_sandboxed_workspace(world: &mut QuectoWorld, path: String) {
    let ws = PathBuf::from(&path);
    world.sandbox = Some(Sandbox::new(Some(ws)));
}

#[when(expr = "the agent tries to validate path {string}")]
fn when_validate_path(world: &mut QuectoWorld, path: String) {
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    world.validation_result = Some(
        sb.validate_path(&path)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

#[when(expr = "the agent tries to validate command {string}")]
fn when_validate_command(world: &mut QuectoWorld, command: String) {
    let default_sb = Sandbox::new(None);
    let sb = world.sandbox.as_ref().unwrap_or(&default_sb);
    world.validation_result = Some(sb.validate_command(&command).map_err(|e| e.to_string()));
}

#[then("the validation should be an error")]
fn then_validation_is_error(world: &mut QuectoWorld) {
    let result = world
        .validation_result
        .as_ref()
        .expect("no validation result");
    assert!(result.is_err(), "expected validation error, got Ok");
}

#[then("the validation should be ok")]
fn then_validation_is_ok(world: &mut QuectoWorld) {
    let result = world
        .validation_result
        .as_ref()
        .expect("no validation result");
    assert!(
        result.is_ok(),
        "expected validation to succeed, got: {}",
        result.as_ref().unwrap_err()
    );
}

#[then(expr = "the error should mention {string}")]
fn then_error_should_mention(world: &mut QuectoWorld, expected: String) {
    let result = world
        .validation_result
        .as_ref()
        .expect("no validation result");
    let err_msg = result.as_ref().unwrap_err();
    assert!(
        err_msg.contains(&expected),
        "expected error to mention '{}', got: {}",
        expected,
        err_msg
    );
}

// ===========================================================================

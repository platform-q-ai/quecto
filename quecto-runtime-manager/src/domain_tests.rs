use super::*;

fn request() -> EnsureRuntimeRequest {
    EnsureRuntimeRequest {
        agent_profile_id: "agent-EANncutp8AIdZsDG5yZBsg".to_string(),
        user_id: Some("user".to_string()),
        project_id: "project-ahcu5C_pJUSBYiQLn7xzgw".to_string(),
        chat_id: "task-lDLtBzG0ERvp1jjTuVeuiA".to_string(),
        session_name: "session".to_string(),
        session_key: "key".to_string(),
        execution_model: None,
        repository: None,
        runtime: None,
        workflow: None,
    }
}

#[test]
fn runtime_ref_is_deterministic_safe_and_socket_friendly() {
    let body = request();
    let r = runtime_ref(&body);

    assert_eq!(r, runtime_ref(&body));
    assert!(r.len() <= 64);
    assert!(
        r.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
    );
    assert!(socket_path_within_uds_limit(Path::new("/data/sockets"), &r));
    // A pathologically long socket root pushes the path past the OS limit.
    let long_root = "/".to_string() + &"x".repeat(120);
    assert!(!socket_path_within_uds_limit(Path::new(&long_root), &r));
    assert_ne!(
        r,
        runtime_ref(&EnsureRuntimeRequest {
            chat_id: format!("{}-other", body.chat_id),
            ..body
        })
    );
}

#[test]
fn validation_rejects_missing_identity_fields() {
    let mut body = request();
    body.session_name = " ".to_string();

    assert_eq!(
        validate_ensure_request(&body),
        Err("missing session_name".to_string())
    );
}

#[test]
fn validation_accepts_pod_execution_model_for_background_board_runs() {
    let mut body = request();
    body.execution_model = Some("pod".to_string());

    assert_eq!(validate_ensure_request(&body), Ok(()));
}

#[test]
fn validation_rejects_unknown_execution_model() {
    let mut body = request();
    body.execution_model = Some("docker".to_string());

    assert_eq!(
        validate_ensure_request(&body),
        Err("invalid execution_model docker".to_string())
    );
}

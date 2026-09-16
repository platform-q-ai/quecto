use std::fs;
use std::path::Path;

const GUIDE: &str = "docs/folder-aware-sessions.md";

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

#[test]
fn folder_aware_session_operating_contract_is_documented() {
    let guide = read(GUIDE);
    for required in [
        "Global transcript authority",
        "Local and global discovery",
        "Exact-key compatibility",
        "Recovery and diagnostics",
        "Safe cross-folder actions",
        "Legacy unscoped sessions",
        "Retirement audit",
        "Ctrl+G",
    ] {
        assert!(guide.contains(required), "{GUIDE} must document {required:?}");
    }
}

#[test]
fn scope_domain_remains_pure_and_effects_stay_behind_application_ports() {
    let domain = read("src/domain/session_scope.rs");
    for forbidden in ["std::fs", "std::process", "tokio::", "crate::infrastructure", "crate::interface"] {
        assert!(!domain.contains(forbidden), "session scope domain must not contain {forbidden}");
    }

    let port = read("src/application/sessions/ports/scope_discovery.rs");
    assert!(port.contains("pub trait SessionScopeDiscovery"));
    let adapter = read("src/infrastructure/session_scope_discovery.rs");
    assert!(adapter.contains("impl SessionScopeDiscovery"));
}

#[test]
fn resume_transactions_are_application_owned_and_interface_only_delegates() {
    let owner = read("src/application/sessions/resume_decision.rs");
    for operation in ["LaunchOriginal", "ForkTranscriptOnly", "LocateAndReassociate"] {
        assert!(owner.contains(operation), "application transaction owner must expose {operation}");
    }

    let interface = read("src/interface/cli/uds_dispatch_session.rs");
    for forbidden in ["std::fs", "std::process", "Command::new", "set_current_dir"] {
        assert!(!interface.contains(forbidden), "session interface must not own effect {forbidden}");
    }
}

#[test]
fn unsafe_legacy_cross_folder_mechanisms_stay_retired() {
    for retired in [
        "src/interface/cli/session_chdir.rs",
        "src/application/sessions/use_cases/resume_in_current_folder.rs",
        "src/infrastructure/persistence/per_folder_session_store.rs",
    ] {
        assert!(!Path::new(retired).exists(), "retired unsafe mechanism returned: {retired}");
    }

    let event_loop = read("../quecto-tui/src/shell/app_event_loop.rs");
    assert!(event_loop.contains("Key::Ctrl('g')") && event_loop.contains("jump-to-latest"),
        "Ctrl+G regression guard must remain represented in the event loop");
}

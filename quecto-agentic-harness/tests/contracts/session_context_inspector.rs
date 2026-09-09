use quecto::domain::session::{CurrentFolderScope, SessionContextInspector};

struct KnownUnavailable;
impl SessionContextInspector for KnownUnavailable {
    fn inspect_current(&self) -> CurrentFolderScope {
        CurrentFolderScope::Unavailable(
            quecto::domain::session::FolderScopeUnavailableReason::CanonicalizationFailed,
        )
    }
}

#[test]
fn session_context_inspector_exposes_typed_unavailable_outcome() {
    assert!(matches!(
        KnownUnavailable.inspect_current(),
        CurrentFolderScope::Unavailable(_)
    ));
}

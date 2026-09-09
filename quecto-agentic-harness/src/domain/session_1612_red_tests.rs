//! RED contracts for #1612 execution scope metadata.
//!
//! These tests intentionally name the missing domain API. They must remain
//! pure: resolving a live working directory and looking up Git belong to
//! infrastructure, not to these value objects.

use super::{
    AgentDisplayName, ExecutionMetadata, FolderDisplayLabel, FolderIdentity, GitBranchDisplay,
    Session,
};

fn complete_metadata(folder_byte: u8, suffix: &str) -> ExecutionMetadata {
    ExecutionMetadata::new(
        Some(
            FolderIdentity::from_unix_bytes(vec![b'/', folder_byte])
                .expect("short native folder identity must be accepted"),
        ),
        FolderDisplayLabel::new(format!("/work/{suffix}")),
        AgentDisplayName::new(format!("agent-{suffix}")),
        GitBranchDisplay::new(format!("branch/{suffix}")),
    )
}

#[test]
fn folder_identity_is_lossless_platform_tagged_and_independent_of_display() {
    let unix_bytes = vec![b'/', b'w', 0xff, b'k'];
    let same_unix = FolderIdentity::from_unix_bytes(unix_bytes.clone()).unwrap();
    let sibling_unix = FolderIdentity::from_unix_bytes(vec![b'/', b'w', 0xfe, b'k']).unwrap();
    let nested_unix =
        FolderIdentity::from_unix_bytes(vec![b'/', b'w', 0xff, b'k', b'/', b'n']).unwrap();
    let windows_units = vec![b'C' as u16, b':' as u16, b'\\' as u16, 0xd800];
    let windows = FolderIdentity::from_windows_units(windows_units.clone()).unwrap();

    assert_eq!(same_unix.unix_bytes(), Some(unix_bytes.as_slice()));
    assert_eq!(same_unix.windows_units(), None);
    assert_eq!(windows.windows_units(), Some(windows_units.as_slice()));
    assert_eq!(windows.unix_bytes(), None);
    assert_ne!(
        same_unix, sibling_unix,
        "sibling folders must stay distinct"
    );
    assert_ne!(same_unix, nested_unix, "nested folders must stay distinct");
    assert_ne!(
        same_unix, windows,
        "a platform tag is part of durable identity"
    );

    let colliding_label = FolderDisplayLabel::new("/work/�k").unwrap();
    let left = ExecutionMetadata::new(Some(same_unix), Some(colliding_label.clone()), None, None);
    let right = ExecutionMetadata::new(Some(sibling_unix), Some(colliding_label), None, None);
    assert_ne!(
        left.folder_identity(),
        right.folder_identity(),
        "a lossy display-label collision must never become folder identity"
    );
}

#[test]
fn persisted_folder_key_round_trips_without_consulting_the_filesystem() {
    let original = FolderIdentity::from_unix_bytes(b"/already/deleted/folder".to_vec()).unwrap();
    let persisted = original.encoded_key().to_owned();

    // This pure decoder deliberately receives no Path and performs no I/O.
    let restored = FolderIdentity::from_encoded_key(&persisted).unwrap();

    assert_eq!(restored, original);
    assert_eq!(restored.encoded_key(), persisted);
    assert!(FolderIdentity::from_encoded_key("unix:0").is_none());
    assert!(FolderIdentity::from_encoded_key("unix:gg").is_none());
    assert!(FolderIdentity::from_encoded_key("unknown:00").is_none());
    assert!(FolderIdentity::from_encoded_key("windows-u16le:00").is_none());
    let windows = FolderIdentity::from_windows_units(vec![0x43, 0x3a, 0xd800]).unwrap();
    assert_eq!(
        FolderIdentity::from_encoded_key(&windows.encoded_key()),
        Some(windows)
    );
}

#[test]
fn complete_tagged_folder_identity_is_bounded_at_four_kibibytes() {
    assert_eq!(FolderIdentity::MAX_ENCODED_BYTES, 4 * 1024);

    let accepted = (1..=FolderIdentity::MAX_ENCODED_BYTES)
        .filter_map(|payload_len| FolderIdentity::from_unix_bytes(vec![b'x'; payload_len]))
        .next_back()
        .expect("at least one native identity must fit");
    assert!(
        accepted.encoded_key().len() <= FolderIdentity::MAX_ENCODED_BYTES,
        "the limit applies to the complete platform-tagged encoded value"
    );

    let first_rejected_payload_len = accepted.unix_bytes().unwrap().len() + 1;
    assert!(
        FolderIdentity::from_unix_bytes(vec![b'x'; first_rejected_payload_len]).is_none(),
        "the next representable identity above the cap becomes unknown, never truncated"
    );
}

#[test]
fn presentation_metadata_enforces_utf8_byte_limits_independently() {
    assert_eq!(AgentDisplayName::MAX_BYTES, 256);
    assert_eq!(GitBranchDisplay::MAX_BYTES, 256);
    assert_eq!(FolderDisplayLabel::MAX_BYTES, 512);

    for bytes in [255, 256] {
        assert!(AgentDisplayName::new("a".repeat(bytes)).is_some());
        assert!(GitBranchDisplay::new("b".repeat(bytes)).is_some());
    }
    assert!(AgentDisplayName::new("a".repeat(257)).is_none());
    assert!(GitBranchDisplay::new("b".repeat(257)).is_none());
    for bytes in [511, 512] {
        assert!(FolderDisplayLabel::new("f".repeat(bytes)).is_some());
    }
    assert!(FolderDisplayLabel::new("f".repeat(513)).is_none());

    let exact_multibyte_name = format!("{}a", "é".repeat(127));
    assert_eq!(exact_multibyte_name.len(), 255);
    assert!(AgentDisplayName::new(exact_multibyte_name.clone()).is_some());
    assert!(GitBranchDisplay::new(exact_multibyte_name).is_some());
    assert!(AgentDisplayName::new("é".repeat(128)).is_some());
    assert!(GitBranchDisplay::new("é".repeat(128)).is_some());
    let overlong_multibyte_name = format!("{}a", "é".repeat(128));
    assert!(AgentDisplayName::new(overlong_multibyte_name.clone()).is_none());
    assert!(GitBranchDisplay::new(overlong_multibyte_name).is_none());

    let exact_multibyte_label = "é".repeat(256);
    assert_eq!(exact_multibyte_label.len(), 512);
    assert!(FolderDisplayLabel::new(exact_multibyte_label).is_some());
    assert!(FolderDisplayLabel::new(format!("{}a", "é".repeat(256))).is_none());

    for actual in [
        AgentDisplayName::new("agent\n\u{1b}[31m\u{202e}")
            .unwrap()
            .as_str(),
        GitBranchDisplay::new("branch\n\u{1b}[31m\u{202e}")
            .unwrap()
            .as_str(),
        FolderDisplayLabel::new("/folder\n\u{1b}[31m\u{202e}")
            .unwrap()
            .as_str(),
    ] {
        assert!(
            actual.contains("\n\u{1b}[31m\u{202e}"),
            "bounded domain data is retained losslessly for UI escaping"
        );
    }
}

#[test]
fn execution_metadata_keeps_optional_components_independent() {
    let identity = FolderIdentity::from_unix_bytes(b"/work".to_vec()).unwrap();
    let label = FolderDisplayLabel::new("/work").unwrap();
    let agent = AgentDisplayName::new("builder").unwrap();
    let branch = GitBranchDisplay::new("main").unwrap();
    assert!(AgentDisplayName::new("x".repeat(AgentDisplayName::MAX_BYTES + 1)).is_none());

    let cases = [
        ExecutionMetadata::new(
            None,
            Some(label.clone()),
            Some(agent.clone()),
            Some(branch.clone()),
        ),
        ExecutionMetadata::new(
            Some(identity.clone()),
            None,
            Some(agent.clone()),
            Some(branch.clone()),
        ),
        ExecutionMetadata::new(
            Some(identity.clone()),
            Some(label.clone()),
            None,
            Some(branch.clone()),
        ),
        ExecutionMetadata::new(Some(identity), Some(label), Some(agent), None),
    ];

    for (unknown_index, metadata) in cases.iter().enumerate() {
        let known = [
            metadata.folder_identity().is_some(),
            metadata.folder_label().is_some(),
            metadata.agent_name().is_some(),
            metadata.git_branch().is_some(),
        ];
        for (index, is_known) in known.into_iter().enumerate() {
            assert_eq!(
                is_known,
                index != unknown_index,
                "only component {unknown_index} may degrade to unknown"
            );
        }
    }
}

#[test]
fn session_initializes_origin_once_and_replaces_only_latest_afterward() {
    let first = complete_metadata(b'a', "first");
    let second = complete_metadata(b'b', "second");
    let detached = ExecutionMetadata::new(
        second.folder_identity().cloned(),
        second.folder_label().cloned(),
        second.agent_name().cloned(),
        None,
    );
    let mut session = Session::new("cli:scope-lifecycle");

    assert_eq!(session.origin_execution_metadata(), None);
    assert_eq!(session.latest_execution_metadata(), None);

    session.initialize_execution_metadata(first.clone());
    assert_eq!(session.origin_execution_metadata(), Some(&first));
    assert_eq!(session.latest_execution_metadata(), Some(&first));

    session.initialize_execution_metadata(second.clone());
    assert_eq!(
        session.origin_execution_metadata(),
        Some(&first),
        "origin is immutable even if initialization is attempted twice"
    );
    assert_eq!(
        session.latest_execution_metadata(),
        Some(&first),
        "repeated initialization must not act like a later activation"
    );
    session.update_latest_execution_metadata(second.clone());
    assert_eq!(session.origin_execution_metadata(), Some(&first));
    assert_eq!(session.latest_execution_metadata(), Some(&second));

    session.update_latest_execution_metadata(detached.clone());
    assert_eq!(session.origin_execution_metadata(), Some(&first));
    assert_eq!(session.latest_execution_metadata(), Some(&detached));
    assert_eq!(
        session.latest_execution_metadata().unwrap().git_branch(),
        None,
        "a full latest snapshot explicitly clears a now-unknown branch"
    );
}

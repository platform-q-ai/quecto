use super::*;
use tempfile::TempDir;

#[test]
fn trust_is_by_canonical_path_and_exact_content_and_survives_a_restart() {
    let base = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let overlay = repo.path().join("config.json");
    std::fs::write(&overlay, "{}").unwrap();
    let trust = PersistentOverlayTrustStore::for_base_dir(base.path(), false);
    assert_eq!(
        trust.record_path(),
        base.path().join(TRUST_RECORD_FILE_NAME)
    );
    assert_eq!(
        trust.decide(&overlay, b"{}"),
        OverlayTrust::Untrusted {
            fingerprint: hex_sha256(b"{}")
        }
    );

    let approval = trust.approve(&overlay, b"{}").unwrap();
    assert_eq!(approval.path, overlay.canonicalize().unwrap());
    assert_eq!(approval.fingerprint, hex_sha256(b"{}"));

    let fresh = PersistentOverlayTrustStore::for_base_dir(base.path(), false);
    assert_eq!(fresh.decide(&overlay, b"{}"), OverlayTrust::Trusted);
    assert!(
        matches!(
            fresh.decide(&overlay, b"{ }"),
            OverlayTrust::Untrusted { .. }
        ),
        "one byte of difference is a different content"
    );
    let elsewhere = repo.path().join("other.json");
    assert!(
        matches!(
            fresh.decide(&elsewhere, b"{}"),
            OverlayTrust::Untrusted { .. }
        ),
        "the same content at another path is not trusted"
    );
}

#[test]
fn a_prompting_adapter_offers_nothing_without_a_terminal_and_records_nothing() {
    let base = TempDir::new().unwrap();
    let trust = PersistentOverlayTrustStore::for_base_dir(base.path(), true);
    let overlay = base.path().join("missing").join("config.json");
    assert!(matches!(
        trust.decide(&overlay, b"{}"),
        OverlayTrust::Untrusted { .. }
    ));
    assert!(!trust.offer(&overlay, "abc", b"{}"));
    assert!(!trust.record_path().exists());
    assert!(!prompt_approval(&overlay, "abc", b"{}"));
    let silent = PersistentOverlayTrustStore::for_base_dir(base.path(), false);
    assert!(!silent.offer(&overlay, "abc", b"{}"));
}

#[test]
fn the_prompt_shows_the_document_bounded_to_the_preview_limit() {
    assert_eq!(prompt_preview(b"{\"a\": 1}\n"), "{\"a\": 1}\n");
    assert_eq!(prompt_preview(b"{}"), "{}\n");
    let long = "x".repeat(PROMPT_PREVIEW_LIMIT + 10);
    let shown = prompt_preview(long.as_bytes());
    assert!(shown.starts_with(&"x".repeat(PROMPT_PREVIEW_LIMIT)));
    assert!(
        shown.ends_with("more bytes not shown)\n"),
        "{}",
        &shown[shown.len() - 40..]
    );
    assert!(!shown.contains(&"x".repeat(PROMPT_PREVIEW_LIMIT + 1)));
    // A multi-byte character at the boundary is never split.
    let accented = "é".repeat(PROMPT_PREVIEW_LIMIT + 1);
    assert!(prompt_preview(accented.as_bytes()).starts_with(&"é".repeat(PROMPT_PREVIEW_LIMIT)));
}

#[test]
fn an_approval_supersedes_the_previous_one_and_legacy_lists_read_their_last_entry() {
    let base = TempDir::new().unwrap();
    let trust = PersistentOverlayTrustStore::for_base_dir(base.path(), false);
    let path = base.path().join("x.json");
    trust.approve(&path, b"v1").unwrap();
    trust.approve(&path, b"v2").unwrap();
    assert_eq!(trust.decide(&path, b"v2"), OverlayTrust::Trusted);
    assert!(
        matches!(trust.decide(&path, b"v1"), OverlayTrust::Untrusted { .. }),
        "reverting to earlier content does not restore trust"
    );

    let legacy = serde_json::json!({
        "approved": { path.to_string_lossy(): [hex_sha256(b"old"), hex_sha256(b"new")] }
    });
    std::fs::write(trust.record_path(), serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(trust.decide(&path, b"new"), OverlayTrust::Trusted);
    assert_eq!(
        trust.decide(&path, b"old"),
        OverlayTrust::Trusted,
        "a legacy list keeps every entry"
    );
    trust.approve(&path, b"new").unwrap();
    assert!(
        matches!(trust.decide(&path, b"old"), OverlayTrust::Untrusted { .. }),
        "collapsed to one"
    );
}

#[test]
fn an_unresolvable_path_is_keyed_absolute_and_a_malformed_record_is_empty() {
    let base = TempDir::new().unwrap();
    let trust = PersistentOverlayTrustStore::for_base_dir(base.path(), false);
    let missing = base.path().join("nowhere").join("config.json");
    let approval = trust.approve(&missing, b"x").unwrap();
    assert_eq!(approval.path, missing);
    assert!(canonical_identity(Path::new("relative.json")).starts_with('/'));

    std::fs::write(trust.record_path(), "{ not json").unwrap();
    assert!(read_record(trust.record_path()).approved.is_empty());
    assert_eq!(record_parent_to_create(Path::new("bare.json")), None);
    assert_eq!(
        record_parent_to_create(Path::new("/a/b/t.json")),
        Some(Path::new("/a/b"))
    );
}

#[test]
fn an_unwritable_record_fails_approval_with_the_record_path() {
    let base = TempDir::new().unwrap();
    let blocker = base.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let trust = PersistentOverlayTrustStore::for_base_dir(&blocker, false);
    let error = trust.approve(Path::new("/tmp/x.json"), b"{}").unwrap_err();
    assert!(error.contains(TRUST_RECORD_FILE_NAME), "{error}");
}

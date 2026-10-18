use super::*;

#[test]
fn a_rejection_is_reused_only_at_its_stamp_and_forgotten_with_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let (kept, gone) = (dir.path().join("kept.json"), dir.path().join("gone.json"));
    std::fs::write(&kept, b"x").unwrap();
    let mut rejections = Rejections::default();
    rejections.record(&kept, vec![1], &DomainError::Session("bad".into()), 7);
    rejections.record(&gone, vec![1], &DomainError::Provider("bad".into()), 6);
    assert!(rejections.at(&kept, &[1], 7).is_some() && rejections.at(&kept, &[2], 7).is_none());
    assert!(rejections.at(&gone, &[1], 6).is_some());
    rejections.retain_seen(7);
    assert_eq!(rejections.0.keys().collect::<Vec<_>>(), [&kept]);
    assert_eq!(rejections.len(), 1);
}

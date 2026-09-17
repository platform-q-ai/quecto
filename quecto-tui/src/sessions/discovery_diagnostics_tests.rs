use super::*;

#[test]
fn each_diagnostic_toasts_once_per_process_and_batches_name_every_source() {
    let mut shown = BTreeSet::new();
    let a = "a.json: session record unavailable: expected value".to_string();
    let b = "b.json: session record unavailable: EOF".to_string();
    assert_eq!(
        unseen_diagnostics_toast(&mut shown, std::slice::from_ref(&a)),
        Some(a.clone())
    );
    // Same listing again (other scope, or a second /resume): silent.
    assert_eq!(
        unseen_diagnostics_toast(&mut shown, std::slice::from_ref(&a)),
        None
    );
    assert_eq!(unseen_diagnostics_toast(&mut shown, &[]), None);
    // Several new ones (duplicates within a batch count once): one summary
    // that names each source, so no file name is lost to the batching.
    let c = "c.json: home needs repair: missing".to_string();
    assert_eq!(
        unseen_diagnostics_toast(&mut shown, &[a.clone(), b.clone(), b.clone(), c.clone()]),
        Some(format!(
            "2 session discovery problems: b.json, c.json; first: {b}"
        ))
    );
    assert_eq!(unseen_diagnostics_toast(&mut shown, &[b, c]), None);
}

#[test]
fn large_batches_name_the_first_sources_and_count_the_rest() {
    let mut shown = BTreeSet::new();
    let batch: Vec<String> = (0..5)
        .map(|i| format!("{i}.json: session record unavailable: EOF"))
        .collect();
    let line = unseen_diagnostics_toast(&mut shown, &batch).unwrap();
    assert!(
        line.starts_with(
            "5 session discovery problems: 0.json, 1.json, 2.json, +2 more; first: 0.json:"
        ),
        "{line}"
    );
}

#[test]
fn non_record_diagnostics_are_summarised_without_claiming_a_record() {
    let mut shown = BTreeSet::new();
    let batch = [
        "workspace discovery unavailable".to_string(),
        "home catalogue unreadable; rebuilt from authority".to_string(),
    ];
    let line = unseen_diagnostics_toast(&mut shown, &batch).unwrap();
    assert_eq!(
        line,
        "2 session discovery problems: workspace discovery unavailable, home catalogue unreadable; rebuilt from authority; first: workspace discovery unavailable"
    );
}

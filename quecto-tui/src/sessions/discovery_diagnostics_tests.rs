use super::*;

#[test]
fn each_diagnostic_toasts_once_per_process_and_batches_summarise() {
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
    // Several new ones (duplicates within a batch count once): one summary.
    let c = "c.json: home needs repair: missing".to_string();
    assert_eq!(
        unseen_diagnostics_toast(&mut shown, &[a.clone(), b.clone(), b.clone(), c.clone()]),
        Some(format!("2 session records need repair; first: {b}"))
    );
    assert_eq!(unseen_diagnostics_toast(&mut shown, &[b, c]), None);
}

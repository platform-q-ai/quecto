use super::*;

fn offer(action: ResumeAction, available: bool, reason: Option<&str>) -> ResumeActionOffer {
    ResumeActionOffer {
        action,
        available,
        reason: reason.map(str::to_string),
    }
}

fn decision(kind: ResumeDecisionKind, actions: Vec<ResumeActionOffer>) -> ResumeDecision {
    ResumeDecision {
        session: "cli:foreign".into(),
        session_key: "cli:foreign".into(),
        kind,
        home_version: "h1-0123456789abcdef".into(),
        execution_path: Some("/work/other".into()),
        detail: None,
        actions,
    }
}

fn cross_folder(open_available: bool) -> ResumeDecisionDialog {
    ResumeDecisionDialog::new(decision(
        ResumeDecisionKind::CrossFolder,
        vec![
            offer(
                ResumeAction::OpenOriginal,
                open_available,
                Some("not yet (#2012)"),
            ),
            offer(ResumeAction::ForkCurrent, false, None),
            offer(ResumeAction::Cancel, true, None),
        ],
    ))
}

fn frame(dialog: &mut ResumeDecisionDialog) -> String {
    let (lines, _) = dialog.render_overlay(120, 30);
    crate::components::ansi::strip_ansi(&lines.join("\n"))
}

#[test]
fn the_dialog_shows_every_offer_in_order_with_unavailable_reasons() {
    let mut dialog = cross_folder(false);
    let text = frame(&mut dialog);
    let open = text.find("Open original folder").expect("open");
    let fork = text.find("Fork into current folder").expect("fork");
    let cancel = text.find("Cancel").expect("cancel");
    assert!(open < fork && fork < cancel, "{text}");
    assert!(text.contains("Unavailable: not yet (#2012)"), "{text}");
    assert!(
        text.contains("Open original folder — unavailable")
            && text.contains("Fork into current folder — unavailable"),
        "every unavailable row says so, not only the one under the cursor: {text}"
    );
    assert!(!text.contains("Cancel —"), "{text}");
    assert!(text.contains("belongs to another folder"), "{text}");
    assert!(
        text.contains("cli:foreign") && text.contains("/work/other"),
        "{text}"
    );
    assert!(text.contains("nothing is restored or linked"), "{text}");
    assert_eq!(dialog.decision().kind, ResumeDecisionKind::CrossFolder);
}

#[test]
fn every_kind_has_its_own_title() {
    let titles: std::collections::BTreeSet<_> = [
        ResumeDecisionKind::CrossFolder,
        ResumeDecisionKind::HomeMissing,
        ResumeDecisionKind::HomeChanged,
        ResumeDecisionKind::HomeUnknown,
        ResumeDecisionKind::LegacyUnscoped,
    ]
    .into_iter()
    .map(title)
    .collect();
    assert_eq!(titles.len(), 5);
}

#[test]
fn every_action_has_its_own_label() {
    let labels: std::collections::BTreeSet<_> = [
        ResumeAction::OpenOriginal,
        ResumeAction::ForkCurrent,
        ResumeAction::Locate,
        ResumeAction::Associate,
        ResumeAction::Cancel,
    ]
    .into_iter()
    .map(label)
    .collect();
    assert_eq!(labels.len(), 5);
}

#[test]
fn an_unavailable_action_is_explained_and_never_chosen() {
    let mut dialog = cross_folder(false);
    assert_eq!(
        dialog.handle_key(&Key::Enter),
        ResumeDecisionEvent::Unavailable(
            "Open original folder is unavailable: not yet (#2012)".into()
        )
    );
    assert_eq!(dialog.handle_key(&Key::Down), ResumeDecisionEvent::Pending);
    assert_eq!(
        dialog.handle_key(&Key::Enter),
        ResumeDecisionEvent::Unavailable("Fork into current folder is unavailable".into())
    );
}

#[test]
fn cancel_and_escape_close_without_a_selection() {
    let mut dialog = cross_folder(false);
    dialog.handle_key(&Key::Down);
    dialog.handle_key(&Key::Down);
    assert_eq!(
        dialog.handle_key(&Key::Enter),
        ResumeDecisionEvent::Cancelled
    );
    let mut dialog = cross_folder(true);
    assert_eq!(
        dialog.handle_key(&Key::Escape),
        ResumeDecisionEvent::Cancelled
    );
}

#[test]
fn an_available_action_sends_identity_action_and_the_decided_version() {
    let mut dialog = cross_folder(true);
    assert_eq!(
        dialog.handle_key(&Key::Enter),
        ResumeDecisionEvent::Chosen(ResumeSelection {
            session: "cli:foreign".into(),
            action: Some(ResumeAction::OpenOriginal),
            expected_home_version: Some("h1-0123456789abcdef".into()),
        })
    );
}

#[test]
fn a_decision_without_a_folder_shows_the_detail_or_a_placeholder() {
    let mut legacy = decision(
        ResumeDecisionKind::LegacyUnscoped,
        vec![
            offer(ResumeAction::Associate, false, Some("later (#2014)")),
            offer(ResumeAction::Cancel, true, None),
        ],
    );
    legacy.execution_path = None;
    let mut dialog = ResumeDecisionDialog::new(legacy.clone());
    assert!(frame(&mut dialog).contains("no folder recorded"));
    legacy.detail = Some("home path is not admissible".into());
    let mut dialog = ResumeDecisionDialog::new(legacy);
    let text = frame(&mut dialog);
    assert!(text.contains("home path is not admissible"), "{text}");
    assert!(text.contains("Associate with a folder"), "{text}");
}

#[test]
fn hostile_metadata_is_made_safe_and_bounded_before_it_is_rendered() {
    let mut hostile = decision(
        ResumeDecisionKind::HomeUnknown,
        vec![
            offer(ResumeAction::Locate, false, Some("why\u{1b}[31m")),
            offer(ResumeAction::Cancel, true, None),
        ],
    );
    hostile.session = "evil\u{1b}[2Jname".into();
    hostile.execution_path = Some(format!("/x\u{7}{}", "y".repeat(2000)));
    hostile.detail = Some("line\r\nbreak\u{1b}]0;title\u{7}".into());
    let mut dialog = ResumeDecisionDialog::new(hostile);
    let decision = dialog.decision().clone();
    for text in [
        decision.session.clone(),
        decision.execution_path.clone().unwrap(),
        decision.detail.clone().unwrap(),
        decision.actions[0].reason.clone().unwrap(),
    ] {
        assert!(!text.chars().any(char::is_control), "{text:?}");
        assert!(text.chars().count() <= 512);
    }
    let (lines, _) = dialog.render_overlay(120, 30);
    let raw = lines.join("\n");
    assert!(
        !raw.contains("\u{1b}[2J") && !raw.contains('\u{7}'),
        "{raw:?}"
    );
}

#[path = "resume_decision_render_tests.rs"]
mod render;

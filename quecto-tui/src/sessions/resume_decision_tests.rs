use super::*;
use crate::protocol::resume_decision_payloads::{ResumeActionOffer, ResumeDecisionKind};

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
    ResumeDecisionDialog::new(
        decision(
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
        ),
        None,
    )
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
    let fork = text
        .find("Copy into this folder as a new session")
        .expect("fork");
    let cancel = text.find("Cancel").expect("cancel");
    assert!(open < fork && fork < cancel, "{text}");
    assert!(text.contains("not yet (#2012)"), "{text}");
    assert!(
        text.contains("Open original folder — unavailable")
            && text.contains("Copy into this folder as a new session — unavailable"),
        "every unavailable row says so, not only the one under the cursor: {text}"
    );
    assert!(!text.contains("Cancel —"), "{text}");
    assert!(text.contains("belongs to another folder"), "{text}");
    assert!(
        text.contains("cli:foreign") && text.contains("/work/other"),
        "{text}"
    );
    assert!(text.contains("your current session is untouched"), "{text}");
    assert_eq!(dialog.decision().kind, ResumeDecisionKind::CrossFolder);
}

/// Review R2-T8: the reason is under the cursor already, so the event carries
/// no second copy of it for a one-line toast to truncate.
#[test]
fn an_unavailable_action_is_never_chosen_and_the_dialog_stays() {
    let mut dialog = cross_folder(false);
    assert_eq!(
        dialog.handle_key(&Key::Enter),
        ResumeDecisionEvent::Unavailable
    );
    assert!(frame(&mut dialog).contains("not yet (#2012)"));
    assert_eq!(dialog.handle_key(&Key::Down), ResumeDecisionEvent::Pending);
    assert_eq!(
        dialog.handle_key(&Key::Enter),
        ResumeDecisionEvent::Unavailable
    );
    assert!(frame(&mut dialog).contains("Not available in this version"));
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
    let mut dialog = ResumeDecisionDialog::new(legacy.clone(), None);
    assert!(frame(&mut dialog).contains("No folder on record"));
    legacy.detail = Some("home path is not admissible".into());
    let mut dialog = ResumeDecisionDialog::new(legacy, None);
    let text = frame(&mut dialog);
    assert!(text.contains("home path is not admissible"), "{text}");
    assert!(text.contains("Attach to a folder"), "{text}");
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
    let mut dialog = ResumeDecisionDialog::new(hostile, None);
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

/// Review R2-T7: the dialog names the session the way the picker did — the
/// title the user just chose, its key beneath; a typed key shows the key.
#[test]
fn the_picked_title_is_shown_with_the_key_beneath_it() {
    let mut picked = ResumeDecisionDialog::new(
        decision(
            ResumeDecisionKind::CrossFolder,
            vec![offer(ResumeAction::Cancel, true, None)],
        ),
        Some("hello from A\u{1b}[2J\u{202e}"),
    );
    let text = frame(&mut picked);
    let title = text.find("hello from A").expect("the picked title");
    let key = text.find("cli:foreign").expect("the key");
    assert!(title < key, "{text}");
    assert!(
        !text.contains("[2J") && !text.contains('\u{202e}'),
        "{text}"
    );
    // A title that only repeats the key, or an empty one, adds nothing.
    for title in ["cli:foreign", ""] {
        let mut plain = ResumeDecisionDialog::new(
            decision(
                ResumeDecisionKind::CrossFolder,
                vec![offer(ResumeAction::Cancel, true, None)],
            ),
            Some(title),
        );
        assert_eq!(frame(&mut plain).matches("cli:foreign").count(), 1);
    }
}

#[path = "resume_decision_render_tests.rs"]
mod render;
#[path = "resume_decision_wording_tests.rs"]
mod wording_tests;

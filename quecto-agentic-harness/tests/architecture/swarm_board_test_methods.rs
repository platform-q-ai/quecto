//! The board dispatcher's test-only methods (#2270 review M2): `create_run`
//! and `bootstrap_run` are the transactional halves of `create` and
//! `_bootstrap` the differential harness drives. A production build (no
//! `test` and no `test-support`) must not serve them: their `Method`
//! variants and their `Method::parse` arms exist only in test builds, so
//! the name falls through to the unknown-method refusal.
use syn::visit::Visit;

use super::cfg_test_gated;

const DISPATCH: &str = "src/infrastructure/tools/swarm_board_dispatch.rs";
const METHODS: &str = "src/infrastructure/tools/swarm_board_dispatch_method.rs";

/// Method names only a test or `test-support` build serves, by the
/// `Method` variant each parses to.
const TEST_ONLY: [(&str, &str); 4] = [
    ("create_run", "CreateRun"),
    ("bootstrap_run", "BootstrapRun"),
    ("bootstrap_join", "BootstrapJoin"),
    ("task_raw", "TaskRaw"),
];

/// Method names every build serves.
const SERVED: [(&str, &str); 54] = [
    ("_status", "Status"),
    ("_event_cursor", "EventCursor"),
    ("_snapshot", "Snapshot"),
    ("_watch", "Watch"),
    ("_run_totals", "RunTotals"),
    ("_admit", "Admit"),
    ("_activate", "Activate"),
    ("_record_launch", "RecordLaunch"),
    ("_release_unlaunched", "ReleaseUnlaunched"),
    ("_socket", "Socket"),
    ("task_create", "TaskCreate"),
    ("dependencies", "Dependencies"),
    ("claim", "Claim"),
    ("release", "Release"),
    ("block", "Block"),
    ("unblock", "Unblock"),
    ("submit", "Submit"),
    ("verify_task", "VerifyTask"),
    ("pause", "Pause"),
    ("resume", "Resume"),
    ("_resume_external", "ResumeExternal"),
    ("_close", "Close"),
    ("_extend_deadline", "ExtendDeadline"),
    ("stop", "Stop"),
    ("_control_status", "ControlStatus"),
    ("usage_report", "UsageReport"),
    ("complete", "Complete"),
    ("revalidate_task", "RevalidateTask"),
    ("amend", "Amend"),
    ("evidence", "Evidence"),
    ("usage_budget", "UsageBudget"),
    ("_record_request", "RecordRequest"),
    ("_request_admission", "RequestAdmission"),
    ("reserve", "Reserve"),
    ("release_files", "ReleaseFiles"),
    ("file_owners", "FileOwners"),
    ("recover", "Recover"),
    ("revoke", "Revoke"),
    ("send", "Send"),
    ("withdraw", "Withdraw"),
    ("inbox", "Inbox"),
    ("ack", "Ack"),
    ("_notifications", "Notifications"),
    ("_accept_wake", "AcceptWake"),
    ("_quarantine", "Quarantine"),
    ("_confirmed_dead", "ConfirmedDead"),
    ("_lose_coordinator", "LoseCoordinator"),
    ("summary", "Summary"),
    ("events", "Events"),
    ("task", "Task"),
    ("tasks", "Tasks"),
    ("create", "Create"),
    ("_bootstrap", "Bootstrap"),
    ("_join", "Join"),
];

/// Each `Method` variant and whether it is test-gated.
fn variants(file: &syn::File) -> Vec<(String, bool)> {
    let method = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "Method" => Some(item),
            _ => None,
        })
        .expect("the dispatcher declares enum Method");
    method
        .variants
        .iter()
        .map(|variant| (variant.ident.to_string(), cfg_test_gated(&variant.attrs)))
        .collect()
}

/// The arms of the `match` in `Method::parse`: each string pattern with
/// whether it is test-gated, and whether the arms end in `_ => None`.
#[derive(Default)]
struct ParseArms {
    names: Vec<(String, bool)>,
    falls_through_to_none: bool,
}

impl<'ast> Visit<'ast> for ParseArms {
    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        match &arm.pat {
            syn::Pat::Lit(syn::ExprLit {
                lit: syn::Lit::Str(name),
                ..
            }) => self.names.push((name.value(), cfg_test_gated(&arm.attrs))),
            syn::Pat::Wild(_) => {
                let none =
                    matches!(&*arm.body, syn::Expr::Path(path) if path.path.is_ident("None"));
                self.falls_through_to_none = none && !cfg_test_gated(&arm.attrs);
            }
            _ => {}
        }
        syn::visit::visit_arm(self, arm);
    }
}

fn parse_arms(file: &syn::File) -> ParseArms {
    let parse = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(block) => Some(block),
            _ => None,
        })
        .filter(|block| {
            matches!(&*block.self_ty, syn::Type::Path(path) if path.path.is_ident("Method"))
        })
        .flat_map(|block| &block.items)
        .find_map(|item| match item {
            syn::ImplItem::Fn(function) if function.sig.ident == "parse" => Some(function),
            _ => None,
        })
        .expect("the dispatcher declares Method::parse");
    let mut arms = ParseArms::default();
    arms.visit_block(&parse.block);
    arms
}

fn gating(entries: &[(String, bool)], name: &str) -> Option<bool> {
    entries
        .iter()
        .find(|(entry, _)| entry == name)
        .map(|(_, gated)| *gated)
}

#[test]
fn test_only_board_methods_are_refused_by_a_production_build() {
    let dispatch = std::fs::read_to_string(DISPATCH).expect("read the dispatcher");
    let dispatch = syn::parse_file(&dispatch).expect("the dispatcher parses");
    assert!(
        dispatch.items.iter().any(|item| matches!(
            item,
            syn::Item::Mod(module)
                if module.ident == "method"
                    && module.attrs.iter().any(|attr| matches!(
                        &attr.meta,
                        syn::Meta::NameValue(value)
                            if value.path.is_ident("path")
                                && matches!(
                                    &value.value,
                                    syn::Expr::Lit(syn::ExprLit {
                                        lit: syn::Lit::Str(path), ..
                                    }) if path.value() == "swarm_board_dispatch_method.rs"
                                )
                    ))
        )),
        "the dispatcher must load the method definitions being checked"
    );
    let source = std::fs::read_to_string(METHODS).expect("read the method sibling");
    let file = syn::parse_file(&source).expect("the method sibling parses");
    let variants = variants(&file);
    let arms = parse_arms(&file);
    for (name, variant) in TEST_ONLY {
        assert_eq!(
            gating(&variants, variant),
            Some(true),
            "Method::{variant} exists only in test and test-support builds"
        );
        assert_eq!(
            gating(&arms.names, name),
            Some(true),
            "Method::parse serves {name} only in test and test-support builds"
        );
    }
    for (name, variant) in SERVED {
        assert_eq!(gating(&variants, variant), Some(false), "Method::{variant}");
        assert_eq!(gating(&arms.names, name), Some(false), "{name}");
    }
    assert_eq!(
        arms.names.len(),
        TEST_ONLY.len() + SERVED.len(),
        "every parsed name is classified: {:?}",
        arms.names
    );
    assert!(
        arms.falls_through_to_none,
        "any other name, a gated one in a production build included, parses to None \
         and so meets the unknown-method refusal"
    );
}

/// The checker itself: an ungated arm, a gate on another feature and a
/// `not(test)` gate are each caught.
#[test]
fn the_test_method_checker_rejects_ungated_arms() {
    for (gate, gated) in [
        ("", false),
        ("#[cfg(feature = \"other\")]", false),
        ("#[cfg(not(test))]", false),
        ("#[cfg(test)]", true),
        ("#[cfg(any(test, feature = \"test-support\"))]", true),
    ] {
        let source = format!(
            "impl Method {{ fn parse(name: &str) -> Option<Self> {{ match name {{ \
             \"_status\" => Some(Self::Status), {gate} \"create_run\" => Some(Self::CreateRun), \
             _ => None }} }} }}"
        );
        let file = syn::parse_file(&source).unwrap();
        let arms = parse_arms(&file);
        assert_eq!(gating(&arms.names, "create_run"), Some(gated), "{gate}");
        assert_eq!(gating(&arms.names, "_status"), Some(false), "{gate}");
        assert!(arms.falls_through_to_none, "{gate}");
    }
    let file = syn::parse_file(
        "impl Method { fn parse(name: &str) -> Option<Self> { match name { _ => Some(Self::Status) } } }",
    )
    .unwrap();
    assert!(!parse_arms(&file).falls_through_to_none);
}

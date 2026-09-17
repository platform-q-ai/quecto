use super::*;
use crate::application::catalogue::dto::{ModelLimits, ModelSwitchPlan};
use crate::domain::catalogue::{ModelRef, TransportKind};

fn switched(verdict: ModelSelectionVerdict) -> ModelSwitched {
    ModelSwitched {
        plan: ModelSwitchPlan {
            model: "acme/m".into(),
            limits: ModelLimits::default(),
            verdict,
        },
        effort_changed: false,
        persisted: None,
    }
}

#[test]
fn runnable_unknown_and_not_runnable_render_the_legacy_selection_shapes() {
    assert_eq!(
        render_switch(&switched(ModelSelectionVerdict::Runnable {
            provider: "acme".into(),
            generation: 3
        })),
        Some(
            serde_json::json!({"selection": {"status": "ok", "provider": "acme", "generation": 3}})
        )
    );
    assert_eq!(
        render_switch(&switched(ModelSelectionVerdict::Unknown {
            reference: "acme/x".into()
        })),
        Some(serde_json::json!({"selection": {"status": "unknown_model", "model": "acme/x"}}))
    );
    let json = render_switch(&switched(ModelSelectionVerdict::NotRunnable {
        reference: ModelRef::parse_qualified("acme/m").unwrap(),
        reasons: vec![
            UnavailableReason::MissingCredential,
            UnavailableReason::UnsupportedTransport {
                transport: TransportKind::Unsupported {
                    declared: "grpc".into(),
                },
            },
            UnavailableReason::InvalidConfiguration("dup".into()),
            UnavailableReason::PolicyDenied("no".into()),
        ],
    }))
    .unwrap();
    assert_eq!(json["selection"]["status"], "not_runnable");
    assert_eq!(json["selection"]["model"], "acme/m");
    assert_eq!(
        json["selection"]["reasons"],
        serde_json::json!([
            "missing-credential",
            "unsupported-transport: grpc",
            "invalid-configuration: dup",
            "policy-denied: no"
        ])
    );
}

#[test]
fn no_runtime_keeps_the_legacy_payload_free_reply() {
    assert_eq!(
        render_switch(&switched(ModelSelectionVerdict::NoRuntime)),
        None
    );
}

#[test]
fn a_recorded_default_is_rendered_beside_the_selection_and_alone_without_a_runtime() {
    use crate::application::catalogue::dto::{DefaultScope, PersistedDefault};
    let persisted = PersistedDefault {
        scope: DefaultScope::Global,
        path: "/home/u/.quecto/config.json".into(),
    };
    let mut with_runtime = switched(ModelSelectionVerdict::Runnable {
        provider: "acme".into(),
        generation: 3,
    });
    with_runtime.persisted = Some(persisted.clone());
    assert_eq!(
        render_switch(&with_runtime),
        Some(serde_json::json!({
            "selection": { "status": "ok", "provider": "acme", "generation": 3 },
            "persisted": { "scope": "global", "path": "/home/u/.quecto/config.json" }
        }))
    );
    let mut without_runtime = switched(ModelSelectionVerdict::NoRuntime);
    without_runtime.persisted = Some(persisted);
    assert_eq!(
        render_switch(&without_runtime),
        Some(serde_json::json!({
            "persisted": { "scope": "global", "path": "/home/u/.quecto/config.json" }
        }))
    );
}

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

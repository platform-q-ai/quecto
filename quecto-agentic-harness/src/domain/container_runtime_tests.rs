use super::container_runtime::*;
use std::collections::HashMap;

#[test]
fn parses_container_requests() {
    assert_eq!(
        SpawnContainerRequest::parse(None).unwrap(),
        SpawnContainerRequest::Local
    );
    assert_eq!(
        SpawnContainerRequest::parse(Some(&serde_json::json!(null))).unwrap(),
        SpawnContainerRequest::Local
    );
    assert_eq!(
        SpawnContainerRequest::parse(Some(&serde_json::json!(false))).unwrap(),
        SpawnContainerRequest::Local
    );
    assert_eq!(
        SpawnContainerRequest::parse(Some(&serde_json::json!(true))).unwrap(),
        SpawnContainerRequest::New {
            repo: None,
            container_script: None
        }
    );
    assert_eq!(SpawnContainerRequest::parse(Some(&serde_json::json!({"mode":"new","repo":"https://github.com/acme/api","container_script":"api-dev"}))).unwrap(), SpawnContainerRequest::New { repo: Some("https://github.com/acme/api".into()), container_script: Some("api-dev".into()) });
    assert_eq!(
        SpawnContainerRequest::parse(Some(&serde_json::json!({"mode":"existing","ref":"C1"})))
            .unwrap(),
        SpawnContainerRequest::Existing {
            reference: ExistingContainerRef::Ref("C1".into())
        }
    );
    assert_eq!(
        SpawnContainerRequest::parse(Some(&serde_json::json!({"mode":"existing","name":"pr-1"})))
            .unwrap(),
        SpawnContainerRequest::Existing {
            reference: ExistingContainerRef::Name("pr-1".into())
        }
    );
    assert!(
        SpawnContainerRequest::parse(Some(&serde_json::json!({"mode":"existing"})))
            .unwrap_err()
            .contains("requires ref or name")
    );
    assert!(
        SpawnContainerRequest::parse(Some(
            &serde_json::json!({"mode":"existing","ref":"C1","name":"pr-1"})
        ))
        .unwrap_err()
        .contains("either ref or name")
    );
    assert!(
        SpawnContainerRequest::parse(Some(&serde_json::json!({"mode":"surprise"})))
            .unwrap_err()
            .contains("unsupported")
    );
    assert!(
        SpawnContainerRequest::parse(Some(&serde_json::json!({"mode":"new","repo":123})))
            .unwrap_err()
            .contains("container.repo must be a string")
    );
    for forbidden in ["branch", "pr", "image"] {
        let err = SpawnContainerRequest::parse(Some(
            &serde_json::json!({"mode":"new", forbidden:"value"}),
        ))
        .unwrap_err();
        assert!(err.contains("unknown container field"), "{err}");
    }
}

#[test]
fn resolves_default_deterministically_and_rejects_invalid() {
    let mut cfg = ContainerScriptsConfig {
        default: Some("dev".into()),
        scripts: HashMap::new(),
    };
    cfg.scripts.insert(
        "dev".into(),
        ContainerScriptSet {
            create: "c".into(),
            exec: "e".into(),
            inspect: "i".into(),
            kill: "k".into(),
        },
    );
    let req = SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    };
    assert_eq!(req.resolve_script(&cfg).unwrap().unwrap().0, "dev");
    cfg.default = Some("missing".into());
    assert!(
        req.resolve_script(&cfg)
            .unwrap_err()
            .contains("not configured")
    );
    cfg.scripts.insert(
        "broken".into(),
        ContainerScriptSet {
            create: "c".into(),
            exec: String::new(),
            inspect: "i".into(),
            kill: "k".into(),
        },
    );
    cfg.default = Some("broken".into());
    assert!(req.resolve_script(&cfg).unwrap_err().contains("incomplete"));
}

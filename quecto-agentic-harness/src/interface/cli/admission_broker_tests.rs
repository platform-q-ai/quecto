//! `admission-broker` administration paths over a real in-process authority.
use super::*;
use crate::interface::cli::CliContext;

fn ctx(dir: &std::path::Path) -> CliContext {
    CliContext {
        base_dir: Some(dir.to_path_buf()),
        config_path: Some(dir.join("config.json")),
        ..CliContext::default()
    }
}

const ENABLED: &str = r#"{"providers":{"openai_compatible":{"endpoints":[{"prefix":"fake","api_key":"k","api_base":"http://127.0.0.1:1","allow_remote_http":true}]}},
"admission":{"directory":DIR,"groups":{"g":{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":8,"queue_timeout_ms":1000,"attempt_timeout_ms":2000,"fallback_base_ms":50,"max_cooldown_ms":500}},"aliases":{"acct":"g"},"bindings":{"fake":"acct"}}}"#;

fn run(ctx: &CliContext, args: &[&str]) -> (i32, String, String) {
    let (mut out, mut err) = (String::new(), String::new());
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let code = cmd_admission_broker(ctx, &args, &mut out, &mut err);
    (code, out, err)
}

#[test]
fn administration_requires_a_section_an_action_and_a_running_authority() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = ctx(temp.path());
    std::fs::write(
        temp.path().join("config.json"),
        r#"{"providers":{"anthropic":{"api_key":"k"}}}"#,
    )
    .unwrap();
    let (code, _, err) = run(&ctx, &["status"]);
    assert_eq!(code, 1);
    assert!(err.contains("no `admission` section"), "{err}");
    let (code, _, err) = run(&ctx, &[]);
    assert_eq!(code, 2);
    assert!(err.contains("expected one of"), "{err}");
    let authority = temp.path().join("authority");
    let config = ENABLED.replace("DIR", &format!("{:?}", authority.to_string_lossy()));
    std::fs::write(temp.path().join("config.json"), config).unwrap();
    let (code, _, err) = run(&ctx, &["status"]);
    assert_eq!(code, 1);
    assert!(err.contains("not running"), "{err}");
    let (code, _, err) = run(&ctx, &["bogus"]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown action"), "{err}");
    let (code, _, err) = run(&ctx, &["run", "--what"]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown option"), "{err}");
    std::fs::write(
        temp.path().join("config.json"),
        ENABLED.replace("DIR", "\"relative\""),
    )
    .unwrap();
    let (code, _, err) = run(&ctx, &["status"]);
    assert_eq!(code, 1);
    assert!(err.contains("absolute"), "{err}");
}

#[test]
fn status_and_reset_talk_to_the_running_authority() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = ctx(temp.path());
    let authority = temp.path().join("authority");
    let config = ENABLED.replace("DIR", &format!("{:?}", authority.to_string_lossy()));
    std::fs::write(temp.path().join("config.json"), config).unwrap();
    let loaded = Config::load(temp.path().join("config.json").to_str().unwrap()).unwrap();
    let (_, proposal) = loaded.admission_proposal().unwrap().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = rt
        .block_on(AuthorityServer::start(
            AuthorityDirectory::open(&authority).unwrap(),
            proposal,
        ))
        .unwrap();
    let (code, out, _) = run(&ctx, &["status"]);
    assert_eq!(code, 0);
    let status: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(status["epoch"], 1);
    assert_eq!(status["live_scopes"], 0);
    assert_eq!(status["groups"]["g"]["active"], 0);
    let (code, out, err) = run(&ctx, &["reset"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), r#"{"epoch":2}"#);
    assert!(err.contains("reset acknowledged"), "{err}");
    rt.block_on(server.shutdown());
}

#[test]
fn run_until_serves_then_stops_and_refuses_a_second_authority() {
    let temp = tempfile::tempdir().unwrap();
    let authority = temp.path().join("authority");
    let config = ENABLED.replace("DIR", &format!("{:?}", authority.to_string_lossy()));
    std::fs::write(temp.path().join("config.json"), config).unwrap();
    let loaded = Config::load(temp.path().join("config.json").to_str().unwrap()).unwrap();
    let (_, proposal) = loaded.admission_proposal().unwrap().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (mut out, mut err) = (String::new(), String::new());
    let dir = AuthorityDirectory::open(&authority).unwrap();
    let outcome = rt.block_on(async {
        let serve = run_until(
            dir.clone(),
            proposal.clone(),
            false,
            async move {
                let _ = stop_rx.await;
            },
            &mut out,
            &mut err,
        );
        tokio::pin!(serve);
        // While serving, a second authority is refused as busy and status works.
        let socket = dir.client_socket();
        let probe = async {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while tokio::net::UnixStream::connect(&socket).await.is_err() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "authority never accepted"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            let (mut o2, mut e2) = (String::new(), String::new());
            let busy = run_until(
                dir.clone(),
                proposal.clone(),
                false,
                async {},
                &mut o2,
                &mut e2,
            )
            .await;
            assert_eq!(busy, 3, "{e2}");
            assert!(e2.contains("another admission authority"), "{e2}");
            let _ = stop_tx.send(());
        };
        let ((), code) = tokio::join!(probe, &mut serve);
        code
    });
    assert_eq!(outcome, 0, "{err}");
    assert!(out.contains("stopped"), "{out}");
    assert!(!dir.client_socket().exists(), "sockets removed on stop");
}

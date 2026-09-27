//! #1942 third review: the production graph reads the process's proxy
//! variables, so it is built in a child test process that inherits one
//! (the parent's own environment is never changed).
use super::*;
use std::process::{Command, Stdio};

const IN_CHILD: &str = "composition::web_fetch::tests::build_under_the_inherited_proxy_environment";

/// Set only in the child process, which builds the graph.
const CHILD: &str = "QUECTO_WEB_FETCH_BUILD_CHILD";

/// In the child (see [`build_survives_every_unusable_proxy_value`]) builds
/// the production graph under the inherited proxy variable; elsewhere does
/// nothing.
#[test]
fn build_under_the_inherited_proxy_environment() {
    if std::env::var_os(CHILD).is_some() {
        let tool = build(reqwest::Client::builder(), 32);
        assert_eq!(tool.definition().name, "web_fetch");
        println!("child built the graph");
    }
}

#[test]
fn build_survives_every_unusable_proxy_value() {
    let exe = std::env::current_exe().expect("the test binary");
    for value in [
        "http://:3128",
        ":3128",
        "http://user@:3128",
        "socks5://:1",
        "http://[]:3128",
        "http://.:3128",
    ] {
        let output = Command::new(&exe)
            .args([IN_CHILD, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env("HTTP_PROXY", value)
            .env_remove("REQUEST_METHOD")
            .stdin(Stdio::null())
            .output()
            .expect("the child test runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout.contains("1 passed")
                && stdout.contains("child built the graph"),
            "HTTP_PROXY={value:?}: {}\n{stdout}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

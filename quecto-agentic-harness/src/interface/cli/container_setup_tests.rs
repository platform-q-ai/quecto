use super::*;
use crate::interface::cli::CliOutput;

fn run(args: &[&str], ctx: &CliContext) -> CliOutput {
    let mut argv = vec!["quecto".to_string()];
    argv.extend(args.iter().map(|s| s.to_string()));
    crate::interface::cli::run_with_output(argv, ctx)
}

#[test]
fn init_and_status_parse_their_flags_and_refuse_unknown_ones() {
    let ctx = CliContext::default();
    for (args, needle) in [
        (
            &["container", "init", "--nope"][..],
            "unknown argument --nope",
        ),
        (
            &["container", "init", "--repo"][..],
            "--repo requires a value",
        ),
        (
            &["container", "init", "--image", ""][..],
            "--image requires a value",
        ),
        (
            &["container", "init", "--project", "relative"][..],
            "--project must be an absolute path",
        ),
        (
            &["container", "status", "--nope"][..],
            "unknown argument --nope",
        ),
        (
            &["container", "status", "--project"][..],
            "--project requires a value",
        ),
        (
            &["container", "frobnicate"][..],
            "usage: quecto container init",
        ),
        (
            &["container", "status", "--project", "rel"][..],
            "--project must be an absolute path",
        ),
    ] {
        let output = run(args, &ctx);
        assert_eq!(output.exit_code, 1, "{args:?}");
        assert!(
            output.stderr.contains(needle),
            "{args:?}: {}",
            output.stderr
        );
    }
}

#[test]
fn init_and_status_refuse_to_run_uncomposed() {
    let dir = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        cwd: Some(dir.path().to_path_buf()),
        ..Default::default()
    };
    let output = run(&["container", "init"], &ctx);
    assert_eq!(output.exit_code, 1);
    assert!(
        output.stderr.contains("container init not composed"),
        "{}",
        output.stderr
    );
    let output = run(&["container", "status"], &ctx);
    assert_eq!(output.exit_code, 1);
    assert!(
        output.stderr.contains("container status not composed"),
        "{}",
        output.stderr
    );
}

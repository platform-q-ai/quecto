//! The fixture scripts' process owner (#2283): `<cfg_dir>/fixture-processes`
//! runs the Rust `quecto-test-fixture processes` (`track`, `live`, `clean`),
//! where a Python helper was.
use std::path::Path;

/// `text` as one single-quoted POSIX shell word, whatever it holds.
fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Installs `<cfg_dir>/fixture-processes`, which the fixture scripts call
/// to own the processes they start.
pub(crate) fn install_fixture_processes(cfg_dir: &Path) {
    let fixture = env!("CARGO_BIN_EXE_quecto-test-fixture");
    assert!(
        Path::new(fixture).is_absolute(),
        "the fixture is named by path"
    );
    quecto::infrastructure::test_support::executable::write_executable(
        &cfg_dir.join("fixture-processes"),
        format!(
            "#!/bin/sh\nexec {} processes \"$@\"\n",
            shell_quoted(fixture)
        ),
    );
}

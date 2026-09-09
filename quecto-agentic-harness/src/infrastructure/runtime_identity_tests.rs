use super::*;

#[cfg(target_os = "linux")]
#[test]
fn runtime_digest_tracks_running_inode_after_executable_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("running");
    std::fs::copy("/bin/sleep", &executable).unwrap();
    let expected = format!("{:x}", Sha256::digest(std::fs::read(&executable).unwrap()));
    let mut process = std::process::Command::new(&executable)
        .arg("30")
        .spawn()
        .unwrap();
    let replacement = directory.path().join("replacement");
    std::fs::copy("/bin/true", &replacement).unwrap();
    std::fs::rename(replacement, executable).unwrap();
    let result = executable_digest_for(process.id());
    // Reap before asserting so a failed assertion never leaves a 30 s child.
    let _ = process.kill();
    let _ = process.wait();
    assert_eq!(result.unwrap(), expected);
}

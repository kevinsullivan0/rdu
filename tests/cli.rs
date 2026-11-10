use std::process::Command;

fn assert_output_failure(mut command: Command) {
    let out = command.output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.starts_with("rdu: "), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn failed_summary_read_only_stdout_exits_one_without_panicking() {
    let d = tempfile::tempdir().unwrap();
    let destination = d.path().join("read-only-output");
    std::fs::write(&destination, b"preserve me").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rdu"));
    command
        .arg("--summary")
        .arg(d.path())
        .stdout(std::fs::File::open(&destination).unwrap());
    assert_output_failure(command);
    assert_eq!(std::fs::read(&destination).unwrap(), b"preserve me");
}

#[test]
fn failed_summary_closed_pipe_exits_one_without_panicking() {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    let d = tempfile::tempdir().unwrap();
    let (reader, writer) = UnixStream::pair().unwrap();
    drop(reader);
    let mut command = Command::new(env!("CARGO_BIN_EXE_rdu"));
    command
        .arg("--summary")
        .arg(d.path())
        .stdout(std::process::Stdio::from(OwnedFd::from(writer)));
    assert_output_failure(command);
}

#[test]
fn failed_help_and_version_closed_pipe_exit_one_without_panicking() {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    for option in ["--help", "--version"] {
        let (reader, writer) = UnixStream::pair().unwrap();
        drop(reader);
        let mut command = Command::new(env!("CARGO_BIN_EXE_rdu"));
        command
            .arg(option)
            .stdout(std::process::Stdio::from(OwnedFd::from(writer)));
        assert_output_failure(command);
    }
}

#[test]
fn failed_table_read_only_stdout_exits_one_without_panicking() {
    let d = tempfile::tempdir().unwrap();
    let destination = d.path().join("read-only-output");
    std::fs::write(&destination, b"preserve me").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rdu"));
    command
        .arg("--non-interactive")
        .arg(d.path())
        .stdout(std::fs::File::open(&destination).unwrap());
    assert_output_failure(command);
    assert_eq!(std::fs::read(&destination).unwrap(), b"preserve me");
}
#[test]
fn summary_is_machine_readable_and_bad_flags_fail() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("hi"), b"hi").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .arg("--summary")
        .arg(d.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["files"], 1);
    assert_eq!(v["errors"], 0);
    assert!(v["apparent"].as_u64().unwrap() >= 2);
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_rdu"))
            .args(["--threads", "0"])
            .output()
            .unwrap()
            .status
            .success()
    );
}
#[test]
fn partial_scan_exits_nonzero_with_json() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("hi"), b"hi").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--summary", "--max-entries", "1"])
        .arg(d.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["errors"].as_u64().unwrap() > 0);
}

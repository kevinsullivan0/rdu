use std::{
    fs,
    process::{Command, Output},
};
fn assert_safe_failure(out: Output) {
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.starts_with("rdu: "));
    assert!(stderr.ends_with('\n'));
    assert_eq!(
        stderr.chars().filter(|&c| c == '\n').count(),
        1,
        "diagnostic structure must be one escaped line: {stderr:?}"
    );
    assert!(
        !stderr
            .chars()
            .any(|c| (c.is_control() && c != '\n')
                || matches!(c as u32,0x2028..=0x202e|0x2060..=0x206f)),
        "raw terminal control in diagnostic: {stderr:?}"
    );
}
#[test]
fn malformed_snapshot_diagnostics_escape_decoded_unknown_fields_and_variants() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file"), b"fixture").unwrap();
    let original = d.path().join("original.json");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_rdu"))
            .args(["--read-only", "--summary", "-o"])
            .arg(&original)
            .arg(&root)
            .output()
            .unwrap()
            .status
            .success()
    );
    let original: serde_json::Value = serde_json::from_slice(&fs::read(original).unwrap()).unwrap();
    let input = d.path().join("malformed.json");
    for payload in [
        "\x1b[2J",
        "\x1b]52;c;Y2xpcGJvYXJk\x07",
        "\r\n\t\x7f",
        "\u{202e}hidden",
    ] {
        let mut value = serde_json::Map::new();
        value.insert(payload.into(), serde_json::json!(0));
        fs::write(&input, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_safe_failure(
            Command::new(env!("CARGO_BIN_EXE_rdu"))
                .args(["--read-only", "--summary", "-f"])
                .arg(&input)
                .output()
                .unwrap(),
        );
        let mut value = original.clone();
        value["dirs"][0]["entries"][0]["kind"] = serde_json::json!(payload);
        fs::write(&input, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_safe_failure(
            Command::new(env!("CARGO_BIN_EXE_rdu"))
                .args(["--read-only", "--summary", "-f"])
                .arg(&input)
                .output()
                .unwrap(),
        );
    }
}
#[test]
fn regex_parser_and_path_errors_escape_controls_at_the_final_boundary() {
    let d = tempfile::tempdir().unwrap();
    for payload in [
        "\x1b[2J",
        "\x1b]52;c;Y2xpcGJvYXJk\x07",
        "\r\n\t\x7f",
        "\u{202e}hidden",
    ] {
        assert_safe_failure(
            Command::new(env!("CARGO_BIN_EXE_rdu"))
                .args(["--read-only", "--summary", "-I"])
                .arg(format!("({payload}"))
                .arg(d.path())
                .output()
                .unwrap(),
        );
        assert_safe_failure(
            Command::new(env!("CARGO_BIN_EXE_rdu"))
                .arg(format!("--unknown-{payload}"))
                .output()
                .unwrap(),
        );
        assert_safe_failure(
            Command::new(env!("CARGO_BIN_EXE_rdu"))
                .args(["--read-only", "--summary"])
                .arg(d.path().join(payload))
                .output()
                .unwrap(),
        );
        assert_safe_failure(
            Command::new(env!("CARGO_BIN_EXE_rdu"))
                .args(["--read-only", "--summary", "-X"])
                .arg(d.path().join(payload))
                .arg(d.path())
                .output()
                .unwrap(),
        );
    }
}

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn run(args: &[&str], root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(args)
        .arg(root)
        .output()
        .unwrap()
}
fn text(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn config(directory: &Path, value: serde_json::Value) -> PathBuf {
    let path = directory.join("settings.json");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    path
}
fn fixture(directory: &Path) -> PathBuf {
    let root = directory.join("root");
    fs::create_dir(&root).unwrap();
    for name in ["skip", "keep", "other", ".hidden"] {
        fs::create_dir(root.join(name)).unwrap();
        fs::write(root.join(name).join("file.TXT"), vec![0; 2048]).unwrap();
        fs::write(root.join(name).join("file.bin"), b"x").unwrap();
    }
    root
}
#[test]
fn explicit_config_sets_scan_text_and_filter_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(directory.path());
    let path = config(
        directory.path(),
        serde_json::json!({"version":1,"threads":1,"apparent":true,"units":"KiB","depth":2,"sort":"name","show_count":true,"no_hidden":true,"ignore_dirs":["skip"],"include_types":[".txt"]}),
    );
    let output = text(run(
        &[
            "--read-only",
            "--summary",
            "--config-file",
            path.to_str().unwrap(),
        ],
        &root,
    ));
    let summary: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(summary["files"], 2);
    assert_eq!(summary["directories"], 3);
    let output = text(run(
        &["--read-only", "-n", "--config-file", path.to_str().unwrap()],
        &root,
    ));
    assert!(output.contains("KiB"));
    assert!(output.contains("keep/file.TXT"));
    assert!(!output.contains("skip/"));
    assert!(!output.contains(".hidden/"));
    assert!(!output.contains("file.bin"));
}
#[test]
fn cli_values_override_config_independently_of_argument_position_and_replace_lists() {
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(directory.path());
    let path = config(
        directory.path(),
        serde_json::json!({"version":1,"threads":64,"units":"KiB","depth":0,"no_hidden":true,"ignore_dirs":["skip"]}),
    );
    let path = path.to_str().unwrap();
    let options = [
        "--read-only",
        "-n",
        "--depth",
        "2",
        "--units",
        "B",
        "--threads",
        "1",
        "-i",
        "keep",
        "-i",
        "other",
    ];
    for config_first in [true, false] {
        let mut args = Vec::new();
        if config_first {
            args.extend(["--config-file", path]);
        }
        args.extend(options);
        if !config_first {
            args.extend(["--config-file", path]);
        }
        let output = text(run(&args, &root));
        assert!(output.contains("skip/file.TXT"), "{output}");
        assert!(!output.contains("keep/"));
        assert!(!output.contains("other/"));
        assert!(!output.contains("KiB"));
    }
}
#[test]
fn cli_regex_sources_combine_after_replacing_configured_patterns() {
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(directory.path());
    let path = config(
        directory.path(),
        serde_json::json!({"version":1,"no_hidden":true,"ignore_patterns":["/skip$"]}),
    );
    let patterns = directory.path().join("patterns");
    fs::write(&patterns, "/other$\n").unwrap();
    let output = text(run(
        &[
            "--read-only",
            "--summary",
            "--config-file",
            path.to_str().unwrap(),
            "-I",
            "/keep$",
            "-X",
            patterns.to_str().unwrap(),
        ],
        &root,
    ));
    let summary: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(summary["files"], 2);
    assert_eq!(summary["directories"], 2);
}
#[test]
fn cli_type_lists_replace_configured_type_lists() {
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(directory.path());
    let path = config(
        directory.path(),
        serde_json::json!({"version":1,"no_hidden":true,"include_types":["bin"],"exclude_types":["txt"]}),
    );
    let output = text(run(
        &[
            "--read-only",
            "--summary",
            "--config-file",
            path.to_str().unwrap(),
            "-T",
            "txt",
            "-E",
            "bin",
        ],
        &root,
    ));
    let summary: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(summary["files"], 3);
}
#[test]
fn minimum_versioned_config_reproduces_current_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(directory.path());
    let path = config(directory.path(), serde_json::json!({"version":1}));
    let mut baseline: serde_json::Value =
        serde_json::from_str(&text(run(&["--read-only", "--summary"], &root))).unwrap();
    let mut configured: serde_json::Value = serde_json::from_str(&text(run(
        &[
            "--read-only",
            "--summary",
            "--config-file",
            path.to_str().unwrap(),
        ],
        &root,
    )))
    .unwrap();
    baseline.as_object_mut().unwrap().remove("elapsed_seconds");
    configured
        .as_object_mut()
        .unwrap()
        .remove("elapsed_seconds");
    assert_eq!(configured, baseline);
}
#[test]
fn config_rejects_unknown_action_keys_versions_types_and_invalid_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(directory.path());
    let values = [
        serde_json::json!({}),
        serde_json::json!({"version":2}),
        serde_json::json!({"version":1,"threads":0}),
        serde_json::json!({"version":1,"threads":65}),
        serde_json::json!({"version":1,"threads":"8"}),
        serde_json::json!({"version":1,"max_entries":0}),
        serde_json::json!({"version":1,"units":"invalid"}),
        serde_json::json!({"version":1,"backend":"invalid"}),
        serde_json::json!({"version":1,"sort":"invalid"}),
        serde_json::json!({"version":1,"path":"root"}),
        serde_json::json!({"version":1,"input":"data"}),
        serde_json::json!({"version":1,"output":"data"}),
        serde_json::json!({"version":1,"shell":"command"}),
        serde_json::json!({"version":1,"delete":true}),
        serde_json::json!({"version":1,"summary":true}),
    ];
    for value in values {
        let path = config(directory.path(), value);
        let out = run(
            &[
                "--read-only",
                "--summary",
                "--config-file",
                path.to_str().unwrap(),
            ],
            &root,
        );
        assert_eq!(
            out.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stdout.is_empty());
    }
    let path = directory.path().join("duplicate.json");
    fs::write(&path, r#"{"version":1,"threads":1,"threads":2}"#).unwrap();
    assert_eq!(
        run(
            &[
                "--read-only",
                "--summary",
                "--config-file",
                path.to_str().unwrap()
            ],
            &root
        )
        .status
        .code(),
        Some(1)
    );
}
#[test]
fn config_input_is_bounded_regular_and_does_not_follow_final_symlinks() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
        process::Stdio,
        time::{Duration, Instant},
    };
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(directory.path());
    let path = config(directory.path(), serde_json::json!({"version":1}));
    let link = directory.path().join("settings.link");
    symlink(&path, &link).unwrap();
    let oversized = directory.path().join("oversized");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(65537)
        .unwrap();
    for input in [&link, &oversized, directory.path()] {
        let out = run(
            &[
                "--read-only",
                "--summary",
                "--config-file",
                input.to_str().unwrap(),
            ],
            &root,
        );
        assert_eq!(out.status.code(), Some(1));
    }
    let fifo = directory.path().join("settings.pipe");
    let raw = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
    let mut child = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--read-only", "--summary", "--config-file"])
        .arg(&fifo)
        .arg(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(1));
            break;
        }
        if start.elapsed() > Duration::from_secs(2) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("configuration input must reject FIFO without blocking")
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn long_equals_syntax_respects_values_and_the_positional_delimiter() {
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(directory.path());
    let path = config(
        directory.path(),
        serde_json::json!({"version":1,"no_hidden":true,"ignore_dirs":["skip"]}),
    );
    let arg = format!("--config-file={}", path.display());
    let output = text(run(
        &[
            "--read-only",
            "--summary",
            "--threads=1",
            "--backend=portable",
            "--depth=0",
            "--ignore-dirs=",
            &arg,
        ],
        &root,
    ));
    let summary: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(summary["files"], 6);
    let literal = directory.path().join("--threads=1");
    fs::create_dir(&literal).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .current_dir(directory.path())
        .args(["--read-only", "--summary", "--", "--threads=1"])
        .output()
        .unwrap();
    text(out);
    fs::create_dir(root.join("--config-file")).unwrap();
    fs::write(root.join("--config-file/file"), b"x").unwrap();
    let output = text(run(
        &["--read-only", "--summary", "--ignore-dirs=--config-file"],
        &root,
    ));
    let summary: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(summary["files"], 8);
    assert_eq!(run(&["--summary=true"], &root).status.code(), Some(1));
}
#[test]
fn top_and_summary_modes_are_explicit_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(directory.path());
    for args in [
        ["--read-only", "--summary", "--top", "1"],
        ["--read-only", "--summarize", "--top", "0"],
    ] {
        let out = run(&args, &root);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("top"));
    }
}

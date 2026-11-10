use std::{
    fs,
    process::{Command, Output},
};
fn run(args: &[&str], path: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(args)
        .arg(path)
        .output()
        .unwrap()
}
fn text(out: Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
#[test]
fn exact_and_hidden_directory_exclusions_preserve_hidden_files() {
    let d = tempfile::tempdir().unwrap();
    for name in ["skip", "keep", ".hidden"] {
        fs::create_dir(d.path().join(name)).unwrap();
        fs::write(d.path().join(name).join("file"), b"hello").unwrap();
    }
    fs::write(d.path().join(".file"), b"yes").unwrap();
    let out = text(run(
        &["--summary", "--read-only", "-H", "-i", "skip"],
        d.path(),
    ));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["files"], 2);
    assert_eq!(v["directories"], 2);
}
#[test]
fn extension_filters_are_case_insensitive_and_keep_directories_traversable() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("nested")).unwrap();
    for name in ["A.RS", "b.txt", "no-extension"] {
        fs::write(d.path().join("nested").join(name), b"ok").unwrap();
    }
    let out = text(run(&["--summary", "-T", "rs,txt", "-E", "txt"], d.path()));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["files"], 1);
    assert_eq!(v["directories"], 2);
}
#[test]
fn text_summary_units_sort_count_and_depth_do_not_limit_scan() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("nested")).unwrap();
    fs::write(d.path().join("nested/file"), vec![0; 2048]).unwrap();
    let summary = text(run(&["-s", "-a", "--no-prefix", "--read-only"], d.path()));
    assert!(!summary.starts_with('{'));
    assert!(summary.contains(d.path().to_str().unwrap()));
    let out = text(run(
        &[
            "-n",
            "--depth",
            "2",
            "--sort",
            "name",
            "--reverse-sort",
            "-C",
            "--units",
            "KiB",
        ],
        d.path(),
    ));
    assert!(out.contains("nested/file"));
    assert!(out.contains("KiB"));
    let out = text(run(
        &[
            "--summary",
            "--depth",
            "0",
            "--max-cores",
            "1",
            "--no-progress",
            "--no-color",
            "--no-unicode",
        ],
        d.path(),
    ));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["files"], 1);
}
#[test]
fn top_ranks_files_recursively_and_keeps_thread_short_flag() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("nested")).unwrap();
    fs::write(d.path().join("small"), b"x").unwrap();
    fs::write(d.path().join("nested/big"), vec![0; 3000]).unwrap();
    let out = text(run(
        &["-n", "-a", "-t", "1", "--top", "1", "--bytes"],
        d.path(),
    ));
    assert!(out.contains("nested/big"));
    assert!(!out.contains("small"));
}
#[test]
fn snapshot_roundtrip_is_offline_and_malformed_graph_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    let live = d.path().join("live");
    fs::create_dir(&live).unwrap();
    fs::write(live.join("a"), b"data").unwrap();
    let snapshot = d.path().join("scan.json");
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--summary", "--read-only", "--output-file"])
        .arg(&snapshot)
        .arg(&live)
        .output()
        .unwrap();
    let expected = text(out);
    fs::remove_file(live.join("a")).unwrap();
    fs::remove_dir(&live).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--summary", "--input-file"])
        .arg(&snapshot)
        .output()
        .unwrap();
    let actual = text(out);
    let expected: serde_json::Value = serde_json::from_str(&expected).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&actual).unwrap();
    assert_eq!(actual["allocated"], expected["allocated"]);
    assert_eq!(actual["files"], 1);
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&snapshot).unwrap()).unwrap();
    v["dirs"][0]["parent"] = serde_json::json!(0);
    fs::write(&snapshot, serde_json::to_vec(&v).unwrap()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--summary", "--input-file"])
        .arg(&snapshot)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}
#[test]
fn regex_patterns_are_full_path_regexes_combined_with_file_lines() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    fs::create_dir(&root).unwrap();
    for name in ["cache123", "node_modules", "keep"] {
        fs::create_dir(root.join(name)).unwrap();
        fs::write(root.join(name).join("file"), b"ok").unwrap();
    }
    let patterns = d.path().join("ignore.txt");
    fs::write(&patterns, "# ignore regexes\n\n/node_modules$\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--summary", "-I", r"/cache\d+$", "-X"])
        .arg(&patterns)
        .arg(&root)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&text(out)).unwrap();
    assert_eq!(v["files"], 1);
    assert_eq!(v["directories"], 2);
    let out = run(&["--summary", "-I", "["], &root);
    assert_eq!(out.status.code(), Some(1));
}
#[test]
fn snapshot_preserves_non_utf8_names_and_refuses_overwrite() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("x"), b"ok").unwrap();
    let snapshot = d.path().join("scan.json");
    let export = || {
        Command::new(env!("CARGO_BIN_EXE_rdu"))
            .args(["--summary", "-o"])
            .arg(&snapshot)
            .arg(&root)
            .output()
            .unwrap()
    };
    assert!(export().status.success());
    let bytes = fs::read(&snapshot).unwrap();
    assert_eq!(export().status.code(), Some(1));
    assert_eq!(fs::read(&snapshot).unwrap(), bytes);
    // APFS rejects non-UTF8 filenames; imported Unix snapshots must still retain raw bytes.
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["dirs"][0]["entries"][0]["name"] = serde_json::json!([120, 255]);
    fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["-n", "-f"])
        .arg(&snapshot)
        .output()
        .unwrap();
    let out = text(out);
    assert!(out.contains(r"x\xff"), "{out}");
}
#[test]
fn malformed_snapshots_and_document_budget_fail_cleanly() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/file"), b"x").unwrap();
    let snapshot = d.path().join("snapshot.json");
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--summary", "-o"])
        .arg(&snapshot)
        .arg(&root)
        .output()
        .unwrap();
    text(out);
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(&snapshot).unwrap()).unwrap();
    for kind in ["child", "name", "integer", "duplicate", "orphan"] {
        let mut value = original.clone();
        match kind {
            "child" => value["dirs"][0]["entries"][0]["child"] = serde_json::json!(999999),
            "name" => value["dirs"][1]["entries"][0]["name"] = serde_json::json!([46, 46]),
            "integer" => value["dirs"][1]["entries"][0]["apparent"] = serde_json::json!(-1),
            "duplicate" => {
                let e = value["dirs"][1]["entries"][0].clone();
                value["dirs"][1]["entries"].as_array_mut().unwrap().push(e);
            }
            _ => value["dirs"][0]["entries"] = serde_json::json!([]),
        }
        fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
            .args(["--summary", "-f"])
            .arg(&snapshot)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{kind}");
        assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
    }
    let file = fs::File::create(&snapshot).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--summary", "-f"])
        .arg(&snapshot)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("64 MiB"));
}
#[test]
fn imported_snapshots_reject_scan_filters_instead_of_silently_ignoring_them() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    fs::create_dir(&root).unwrap();
    let snapshot = d.path().join("snapshot.json");
    text(
        Command::new(env!("CARGO_BIN_EXE_rdu"))
            .args(["--summary", "-o"])
            .arg(&snapshot)
            .arg(&root)
            .output()
            .unwrap(),
    );
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--summary", "-H", "-f"])
        .arg(&snapshot)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("scan filters"));
}
#[test]
fn listing_sort_and_recursive_counts_are_observable() {
    let d = tempfile::tempdir().unwrap();
    for name in ["aa", "zz"] {
        fs::create_dir(d.path().join(name)).unwrap();
    }
    fs::write(d.path().join("aa/one"), b"x").unwrap();
    fs::create_dir(d.path().join("zz/inner")).unwrap();
    fs::write(d.path().join("zz/inner/two"), b"x").unwrap();
    fs::write(d.path().join("zz/three"), b"x").unwrap();
    let out = text(run(&["-n", "-C", "--sort", "count"], d.path()));
    let rows: Vec<_> = out.lines().collect();
    assert!(rows[0].ends_with("zz/"), "{out}");
    assert_eq!(rows[0].split_whitespace().next(), Some("3"));
    assert!(rows[1].ends_with("aa/"));
    let out = text(run(&["-n", "--sort", "name", "--reverse-sort"], d.path()));
    assert!(out.lines().next().unwrap().ends_with("zz/"));
}
#[test]
fn imported_incomplete_snapshot_exits_partial_even_without_counted_errors() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    fs::create_dir(&root).unwrap();
    let snapshot = d.path().join("snapshot.json");
    text(
        Command::new(env!("CARGO_BIN_EXE_rdu"))
            .args(["--summary", "-o"])
            .arg(&snapshot)
            .arg(&root)
            .output()
            .unwrap(),
    );
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&snapshot).unwrap()).unwrap();
    value["complete"] = serde_json::json!(false);
    fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--summary", "-f"])
        .arg(&snapshot)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["errors"], 0);
    assert!(String::from_utf8_lossy(&out.stderr).contains("partial"));
}

#[test]
fn ignore_from_rejects_fifo_without_waiting_for_a_writer() {
    use std::{
        ffi::CString,
        os::unix::ffi::OsStrExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    let d = tempfile::tempdir().unwrap();
    let fifo = d.path().join("patterns.pipe");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let mut child = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args(["--read-only", "--summary", "-X"])
        .arg(&fifo)
        .arg(d.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(2) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("ignore-from must reject FIFO input without waiting for a writer")
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(1));
}
#[test]
fn ignore_from_rejects_final_symlinks_and_directory_inputs() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let patterns = d.path().join("patterns.txt");
    fs::write(&patterns, "/ignored$\n").unwrap();
    let link = d.path().join("patterns.link");
    symlink(&patterns, &link).unwrap();
    for input in [&link, d.path()] {
        let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
            .args(["--read-only", "--summary", "-X"])
            .arg(input)
            .arg(d.path())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{}", input.display());
    }
}

struct DeepFixture {
    _directory: tempfile::TempDir,
    root: std::path::PathBuf,
    leaf: fs::File,
    depth: usize,
}
impl DeepFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root");
        fs::create_dir(&root).unwrap();
        let leaf = fs::File::open(&root).unwrap();
        Self {
            _directory: directory,
            root,
            leaf,
            depth: 0,
        }
    }
    fn descend(&mut self) {
        use std::os::fd::{AsRawFd, FromRawFd};
        assert_eq!(
            unsafe { libc::mkdirat(self.leaf.as_raw_fd(), c"d".as_ptr(), 0o700) },
            0
        );
        let next = unsafe {
            libc::openat(
                self.leaf.as_raw_fd(),
                c"d".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(next >= 0);
        self.leaf = unsafe { fs::File::from_raw_fd(next) };
        self.depth += 1;
    }
}
impl Drop for DeepFixture {
    fn drop(&mut self) {
        use std::os::fd::{AsRawFd, FromRawFd};
        for _ in 0..self.depth {
            let parent = unsafe {
                libc::openat(
                    self.leaf.as_raw_fd(),
                    c"..".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if parent < 0 {
                break;
            }
            let parent = unsafe { fs::File::from_raw_fd(parent) };
            let result =
                unsafe { libc::unlinkat(parent.as_raw_fd(), c"d".as_ptr(), libc::AT_REMOVEDIR) };
            self.leaf = parent;
            if result != 0 {
                break;
            }
        }
    }
}
#[test]
fn snapshot_export_and_import_share_depth_limit_before_creating_output() {
    let mut fixture = DeepFixture::new();
    for _ in 0..1024 {
        fixture.descend()
    }
    let allowed = fixture._directory.path().join("boundary.json");
    let export = |output: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_rdu"))
            .args([
                "--read-only",
                "--summary",
                "--max-depth",
                "2000",
                "--threads",
                "1",
                "-o",
            ])
            .arg(output)
            .arg(&fixture.root)
            .output()
            .unwrap()
    };
    text(export(&allowed));
    text(
        Command::new(env!("CARGO_BIN_EXE_rdu"))
            .args(["--summary", "-f"])
            .arg(&allowed)
            .output()
            .unwrap(),
    );
    fixture.descend();
    let rejected = fixture._directory.path().join("too-deep.json");
    let out = Command::new(env!("CARGO_BIN_EXE_rdu"))
        .args([
            "--read-only",
            "--summary",
            "--max-depth",
            "2000",
            "--threads",
            "1",
            "-o",
        ])
        .arg(&rejected)
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !rejected.exists(),
        "invalid export must be rejected before creating its destination"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("depth exceeds 1024"));
}

#![cfg(target_os = "macos")]
use rdu::{
    delete::DeletionSession,
    filter::Filters,
    scan::{Options, Tree},
};
use std::{fs, path::PathBuf, sync::Arc};
fn regex_literal(path: &std::path::Path) -> String {
    let mut result = String::new();
    for c in path.to_str().unwrap().chars() {
        if "\\.^$|?*+()[]{}".contains(c) {
            result.push('\\')
        }
        result.push(c);
    }
    result
}
fn check(relative: bool, absolute: bool, regex: bool) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    fs::create_dir(&root).unwrap();
    for name in ["skip", "keep"] {
        fs::create_dir(root.join(name)).unwrap();
        fs::write(root.join(name).join("fixture"), b"observational fixture").unwrap();
    }
    let alias = directory.path().join("alias");
    std::os::unix::fs::symlink(directory.path(), &alias).unwrap();
    let spelled = alias.join("root");
    assert_ne!(spelled, fs::canonicalize(&spelled).unwrap());
    let ignored = if relative {
        vec![PathBuf::from("skip")]
    } else if absolute {
        vec![spelled.join("skip")]
    } else {
        vec![]
    };
    let patterns = if regex {
        vec![format!("^{}/skip$", regex_literal(&spelled))]
    } else {
        vec![]
    };
    let options = Options {
        threads: 1,
        filters: Some(Arc::new(
            Filters::new(&spelled, false, &ignored, &patterns, vec![], vec![]).unwrap(),
        )),
        ..Default::default()
    };
    let (read_only, session) = DeletionSession::scan(&spelled, &options, true).unwrap();
    assert!(session.is_none());
    let (writable, session) = DeletionSession::scan(&spelled, &options, false).unwrap();
    assert!(
        session.is_some(),
        "fixture must exercise the deletion-capable scan route"
    );
    let scope = |tree: &Tree| {
        let mut names: Vec<_> = tree.dirs[0]
            .entries
            .iter()
            .map(|e| e.name.clone())
            .collect();
        names.sort();
        (tree.files, tree.dirs.len(), names)
    };
    assert_eq!(read_only.files, 1);
    assert_eq!(read_only.dirs.len(), 2);
    assert_eq!(
        scope(&writable),
        scope(&read_only),
        "canonical root acquisition must preserve caller filter spelling"
    );
    assert_eq!(writable.root, read_only.root);
    assert_eq!(writable.dirs[0].identity, read_only.dirs[0].identity);
    for name in ["skip", "keep"] {
        assert!(root.join(name).join("fixture").is_file());
    }
}
#[test]
fn writable_and_read_only_scans_agree_on_relative_exact_exclusions() {
    check(true, false, false)
}
#[test]
fn writable_and_read_only_scans_agree_on_absolute_exact_exclusions() {
    check(false, true, false)
}
#[test]
fn writable_and_read_only_scans_agree_on_full_path_regex_exclusions() {
    check(false, false, true)
}

//! Repeat the ordinary scanner for native sampling, with no profiling feature.
use rdu::scan::{Options, scan};
use std::{hint::black_box, path::PathBuf};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().expect("PATH [REPEATS] [THREADS]"));
    let repeats = args
        .next()
        .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
        .unwrap_or(50usize);
    let threads = args
        .next()
        .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
        .unwrap_or(8usize);
    assert!(repeats > 0);
    let mut expected = None;
    for _ in 0..repeats {
        let tree = scan(
            &path,
            &Options {
                threads,
                ..Default::default()
            },
        )
        .unwrap();
        let totals = (
            tree.dirs[0].allocated,
            tree.dirs[0].apparent,
            tree.files,
            tree.dirs.len(),
            tree.errors,
        );
        if let Some(expected) = expected {
            assert_eq!(totals, expected, "metadata changed during diagnostic loop");
        }
        expected = Some(totals);
        black_box(&tree);
    }
    let (allocated, apparent, files, directories, errors) = expected.unwrap();
    println!(
        "{}",
        serde_json::json!({"allocated":allocated,"apparent":apparent,"files":files,"directories":directories,"errors":errors,"repeats":repeats,"instrumented":false})
    );
}

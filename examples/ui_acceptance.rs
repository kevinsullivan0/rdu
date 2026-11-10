//! Real-PTY acceptance driver. Writable mode accepts only owned /tmp test roots.
use rdu::{
    scan::{Backend, Options},
    ui::{self, UiOptions},
};
fn main() -> std::process::ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next().expect("fixture root required");
    let writable = first == "--fixture-writable";
    let path = std::path::PathBuf::from(if writable {
        arguments.next().expect("fixture root required")
    } else {
        first
    });
    let path = path.canonicalize().unwrap_or(path);
    if writable {
        assert!(
            path.starts_with("/private/tmp") || path.starts_with("/tmp"),
            "writable acceptance roots must be in /tmp"
        );
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("rdu-pty-"),
            "owned fixture prefix required"
        );
    }
    let options = Options {
        backend: Backend::Portable,
        threads: 2,
        ..Default::default()
    };
    match ui::run_path(
        path,
        options,
        UiOptions {
            read_only: !writable,
            ..Default::default()
        },
    ) {
        Ok(code) => std::process::ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

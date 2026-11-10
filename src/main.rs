mod cli;
mod input;
mod output;
mod snapshot;
use rdu::{
    scan::{self},
    ui,
};
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    process::ExitCode,
    sync::atomic::{AtomicBool, Ordering},
};
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}
fn cancelled() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}
fn stdout_writer() -> Result<io::BufWriter<std::fs::File>, String> {
    use std::os::fd::AsFd;
    // Stdout suppresses EBADF for detached consoles. A duplicated owned
    // descriptor preserves redirected-output errors without new unsafe code.
    let stdout = io::stdout();
    let fd = stdout
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| format!("stdout: {e}"))?;
    Ok(io::BufWriter::new(std::fs::File::from(fd)))
}
fn write_stdout(text: std::fmt::Arguments<'_>) -> Result<(), String> {
    let mut writer = stdout_writer()?;
    writer.write_fmt(text).map_err(|e| format!("stdout: {e}"))?;
    writer.flush().map_err(|e| format!("stdout: {e}"))
}
fn execute() -> Result<u8, String> {
    let mut cli = cli::Cli::parse(std::env::args_os().skip(1))?;
    if cli.help {
        write_stdout(format_args!("{}", cli::HELP))?;
        return Ok(0);
    }
    if cli.version {
        write_stdout(format_args!("rdu {}\n", env!("CARGO_PKG_VERSION")))?;
        return Ok(0);
    }
    let path = cli.path.clone().unwrap_or_else(|| PathBuf::from("."));
    if cli.no_hidden
        || !cli.ignore.is_empty()
        || !cli.patterns.is_empty()
        || !cli.include_types.is_empty()
        || !cli.exclude_types.is_empty()
    {
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(&path)
        };
        cli.scan.filters = Some(std::sync::Arc::new(rdu::filter::Filters::new(
            &absolute,
            cli.no_hidden,
            &cli.ignore,
            &cli.patterns,
            cli.include_types.clone(),
            cli.exclude_types.clone(),
        )?));
    }
    let interactive = !cli.summary
        && !cli.noninteractive
        && cli.top.is_none()
        && cli.output.is_none()
        && io::stdout().is_terminal()
        && io::stdin().is_terminal();
    if cli.interactive && !interactive {
        return Err(
            "interactive mode requires terminal stdin and stdout and no output-file".into(),
        );
    }
    let ui_options = ui::UiOptions {
        apparent: cli.apparent,
        read_only: cli.read_only,
        no_color: cli.no_color,
        ascii: cli.ascii,
        sort: match cli.sort {
            cli::Sort::Size => ui::UiSort::Size,
            cli::Sort::Name => ui::UiSort::Name,
            cli::Sort::Count => ui::UiSort::Count,
        },
        reverse: cli.reverse,
        no_progress: cli.no_progress,
        show_count: cli.show_count,
        filtered: cli.scan.filters.is_some(),
    };
    if interactive {
        return if let Some(input) = &cli.input {
            ui::run_snapshot(
                snapshot::import(input).map_err(|e| format!("snapshot import: {e}"))?,
                ui_options,
            )
        } else {
            ui::run_path(path, cli.scan, ui_options)
        }
        .map_err(|e| format!("terminal: {e}"));
    }
    cli.scan.cancelled = Some(cancelled);
    // Handler does only a lock-free atomic store. Restore previous dispositions before UI takes ownership.
    let mut old = Vec::new();
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        let handler = unsafe { libc::signal(signal, interrupt as *const () as libc::sighandler_t) };
        old.push((signal, handler));
    }
    let result = if let Some(input) = &cli.input {
        snapshot::import(input)
    } else {
        scan::scan(&path, &cli.scan)
    };
    for (signal, handler) in old {
        unsafe { libc::signal(signal, handler) };
    }
    if cancelled() {
        return Ok(130);
    }
    let source = cli.input.as_deref().unwrap_or(&path);
    let tree = result.map_err(|e| format!("{}: {e}", ui::safe_text(source.as_os_str())))?;
    if let Some(output) = &cli.output {
        snapshot::export(&tree, output).map_err(|e| format!("snapshot export: {e}"))?;
    }
    let mut out = stdout_writer()?;
    output::write(&tree, &cli, &mut out).map_err(|e| format!("stdout: {e}"))?;
    out.flush().map_err(|e| format!("stdout: {e}"))?;
    if tree.errors > 0 || !tree.complete {
        if tree.errors > 0 {
            eprintln!(
                "rdu: partial results ({} metadata, traversal, or limit errors)",
                tree.errors
            );
        } else {
            eprintln!("rdu: partial results (snapshot is marked incomplete)");
        }
        Ok(2)
    } else {
        Ok(0)
    }
}
fn main() -> ExitCode {
    match execute() {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("rdu: {}", ui::safe_text(std::ffi::OsStr::new(&message)));
            ExitCode::FAILURE
        }
    }
}

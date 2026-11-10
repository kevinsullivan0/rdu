//! Explicit JSON defaults only. Configuration cannot select roots or execute actions.
use super::{Cli, normalize_type, parse_backend, parse_sort, parse_units};
use serde::Deserialize;
use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    threads: Option<usize>,
    backend: Option<String>,
    one_file_system: Option<bool>,
    max_entries: Option<usize>,
    max_depth: Option<usize>,
    no_hidden: Option<bool>,
    ignore_dirs: Option<Vec<PathBuf>>,
    ignore_patterns: Option<Vec<String>>,
    include_types: Option<Vec<String>>,
    exclude_types: Option<Vec<String>>,
    apparent: Option<bool>,
    sort: Option<String>,
    reverse: Option<bool>,
    show_count: Option<bool>,
    no_color: Option<bool>,
    ascii: Option<bool>,
    no_progress: Option<bool>,
    read_only: Option<bool>,
    units: Option<String>,
    depth: Option<usize>,
}
pub(super) fn apply(path: &Path, cli: &mut Cli) -> Result<(), String> {
    let bytes = crate::input::read_regular(path, 65536, "config-file", "64 KiB")
        .map_err(|e| format!("config-file: {e}"))?;
    let config: Config = serde_json::from_slice(&bytes).map_err(|e| format!("config-file: {e}"))?;
    if config.version != 1 {
        return Err("config-file: unsupported version (expected 1)".into());
    }
    if config.threads.is_some_and(|v| v == 0 || v > 64) || config.max_entries == Some(0) {
        return Err("config-file: threads must be 1..64 and max_entries positive".into());
    }
    macro_rules! set {
        ($from:ident,$to:expr) => {
            if let Some(value) = config.$from {
                $to = value;
            }
        };
    }
    set!(threads, cli.scan.threads);
    set!(one_file_system, cli.scan.one_file_system);
    set!(max_entries, cli.scan.max_entries);
    set!(max_depth, cli.scan.max_depth);
    set!(no_hidden, cli.no_hidden);
    set!(ignore_dirs, cli.ignore);
    set!(ignore_patterns, cli.patterns);
    set!(apparent, cli.apparent);
    set!(reverse, cli.reverse);
    set!(show_count, cli.show_count);
    set!(no_color, cli.no_color);
    set!(ascii, cli.ascii);
    set!(no_progress, cli.no_progress);
    set!(read_only, cli.read_only);
    set!(depth, cli.depth);
    if let Some(value) = config.backend {
        cli.scan.backend = parse_backend(&value)?;
    }
    if let Some(value) = config.sort {
        cli.sort = parse_sort(&value)?;
    }
    if let Some(value) = config.units {
        cli.units = parse_units(&value)?;
    }
    if let Some(values) = config.include_types {
        cli.include_types = values.iter().map(|v| normalize_type(v)).collect();
    }
    if let Some(values) = config.exclude_types {
        cli.exclude_types = values.iter().map(|v| normalize_type(v)).collect();
    }
    Ok(())
}
fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "--config-file"
            | "--units"
            | "--sort"
            | "--threads"
            | "-t"
            | "-m"
            | "--max-cores"
            | "--max-entries"
            | "--max-depth"
            | "--depth"
            | "--top"
            | "--backend"
            | "-i"
            | "--ignore-dirs"
            | "-I"
            | "--ignore-dirs-pattern"
            | "-X"
            | "--ignore-from"
            | "-T"
            | "--type"
            | "-E"
            | "--exclude-type"
            | "-f"
            | "--input-file"
            | "-o"
            | "--output-file"
    )
}
/// Normalize value-taking long options and extract one explicitly selected config.
/// Consuming option values here preserves literal --config-file values and -- paths.
pub(super) fn arguments(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(Option<PathBuf>, Vec<OsString>), String> {
    let mut args = args.into_iter();
    let mut config = None;
    let mut normalized = Vec::new();
    while let Some(argument) = args.next() {
        if argument == "--" {
            normalized.push(argument);
            normalized.extend(args);
            break;
        }
        let bytes = argument.as_encoded_bytes();
        let (flag, inline) = if bytes.starts_with(b"--") {
            if let Some(index) = bytes.iter().position(|&b| b == b'=') {
                (
                    OsString::from_vec(bytes[..index].to_vec()),
                    Some(OsString::from_vec(bytes[index + 1..].to_vec())),
                )
            } else {
                (argument, None)
            }
        } else {
            (argument, None)
        };
        let text = flag.to_str().unwrap_or("");
        if takes_value(text) {
            let value = inline
                .or_else(|| args.next())
                .ok_or_else(|| format!("missing value for {text}"))?;
            if text == "--config-file" {
                if config.replace(PathBuf::from(value)).is_some() {
                    return Err("only one config-file is supported".into());
                }
            } else {
                normalized.push(flag);
                normalized.push(value);
            }
        } else {
            if inline.is_some() {
                return Err(format!(
                    "option does not accept a value: {}",
                    super::safe_text(&flag)
                ));
            }
            normalized.push(flag);
        }
    }
    Ok((config, normalized))
}

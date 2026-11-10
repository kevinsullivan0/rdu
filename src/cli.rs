//! Explicit command-line configuration. No implicit config files or shell hooks.
mod config;
use crate::{
    scan::{Backend, Options},
    ui::safe_text,
};
use std::{ffi::OsString, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Size,
    Name,
    Count,
}
#[derive(Clone, Copy, Debug, Default)]
pub enum Units {
    #[default]
    Bytes,
    Human,
    Si,
    Fixed(u64, &'static str),
}
#[derive(Default)]
pub struct Cli {
    pub scan: Options,
    pub path: Option<PathBuf>,
    pub summary: bool,
    pub summarize: bool,
    pub noninteractive: bool,
    pub interactive: bool,
    pub apparent: bool,
    pub read_only: bool,
    pub no_progress: bool,
    pub no_color: bool,
    pub ascii: bool,
    pub quiet: bool,
    pub sort: Sort,
    pub reverse: bool,
    pub show_count: bool,
    pub units: Units,
    pub depth: usize,
    pub top: Option<usize>,
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub no_hidden: bool,
    pub ignore: Vec<PathBuf>,
    pub patterns: Vec<String>,
    pub include_types: Vec<String>,
    pub exclude_types: Vec<String>,
    pub help: bool,
    pub version: bool,
}
fn number(value: OsString, flag: &str) -> Result<usize, String> {
    value
        .to_str()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("{flag} needs a nonnegative integer"))
}
fn string(value: OsString, flag: &str) -> Result<String, String> {
    value
        .into_string()
        .map_err(|_| format!("{flag} requires UTF-8"))
}
fn normalize_type(value: &str) -> String {
    value.trim().trim_start_matches('.').to_ascii_lowercase()
}
fn types(value: OsString, flag: &str) -> Result<Vec<String>, String> {
    Ok(string(value, flag)?
        .split(',')
        .map(normalize_type)
        .collect())
}
fn parse_backend(value: &str) -> Result<Backend, String> {
    match value {
        "auto" => Ok(Backend::Auto),
        "portable" => Ok(Backend::Portable),
        "bulk" => Ok(Backend::Bulk),
        _ => Err("backend must be auto, bulk, or portable".into()),
    }
}
fn parse_sort(value: &str) -> Result<Sort, String> {
    match value {
        "size" => Ok(Sort::Size),
        "name" => Ok(Sort::Name),
        "count" => Ok(Sort::Count),
        _ => Err("sort must be size, name, or count".into()),
    }
}
fn parse_units(value: &str) -> Result<Units, String> {
    match value {
        "B" | "bytes" => Ok(Units::Bytes),
        "KiB" => Ok(Units::Fixed(1024, "KiB")),
        "MiB" => Ok(Units::Fixed(1024 * 1024, "MiB")),
        "GiB" => Ok(Units::Fixed(1024 * 1024 * 1024, "GiB")),
        "human" => Ok(Units::Human),
        "si" => Ok(Units::Si),
        _ => Err("units must be B, KiB, MiB, GiB, human, or si".into()),
    }
}
impl Cli {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let (config, args) = config::arguments(args)?;
        let mut cli = Self {
            depth: 1,
            ..Self::default()
        };
        if let Some(path) = config {
            config::apply(&path, &mut cli)?;
        }
        let (mut ignore_cli, mut patterns_cli, mut include_cli, mut exclude_cli) =
            (false, false, false, false);
        let mut args = args.into_iter();
        let mut positional = false;
        while let Some(arg) = args.next() {
            if positional {
                if cli.path.replace(arg.into()).is_some() {
                    return Err("only one scan path is supported".into());
                }
                continue;
            }
            let text = arg.to_str().unwrap_or("");
            macro_rules! value {
                () => {
                    args.next()
                        .ok_or_else(|| format!("missing value for {text}"))?
                };
            }
            match text {
                "--" => positional = true,
                "-h" | "--help" => cli.help = true,
                "-V" | "-v" | "--version" => cli.version = true,
                "--summary" => cli.summary = true,
                "-s" | "--summarize" => {
                    cli.summarize = true;
                    cli.noninteractive = true;
                }
                "-n" | "--non-interactive" => cli.noninteractive = true,
                "--interactive" => cli.interactive = true,
                "-a" | "--apparent-size" | "--show-apparent-size" => cli.apparent = true,
                "--read-only" | "--no-delete" => cli.read_only = true,
                "-p" | "--no-progress" => cli.no_progress = true,
                "-q" | "--quiet" => {
                    cli.quiet = true;
                    cli.no_progress = true;
                    cli.noninteractive = true;
                }
                "-c" | "--no-color" => cli.no_color = true,
                "--ascii" | "-u" | "--no-unicode" => cli.ascii = true,
                "--reverse" | "--reverse-sort" => cli.reverse = true,
                "-C" | "--show-item-count" => cli.show_count = true,
                "--bytes" | "-b" | "--no-prefix" => cli.units = Units::Bytes,
                "--human-readable" => cli.units = Units::Human,
                "--si" => cli.units = Units::Si,
                "-k" | "--show-in-kib" => cli.units = Units::Fixed(1024, "KiB"),
                "--units" => cli.units = parse_units(&string(value!(), text)?)?,
                "--sort" => cli.sort = parse_sort(&string(value!(), text)?)?,
                "--threads" | "-t" | "-m" | "--max-cores" => {
                    cli.scan.threads = number(value!(), text)?
                }
                "--sequential" => cli.scan.threads = 1,
                "--max-entries" => cli.scan.max_entries = number(value!(), text)?,
                "--max-depth" => cli.scan.max_depth = number(value!(), text)?,
                "--depth" => cli.depth = number(value!(), text)?,
                "--top" => cli.top = Some(number(value!(), text)?),
                "-x" | "--one-file-system" | "--no-cross" => cli.scan.one_file_system = true,
                "--backend" => cli.scan.backend = parse_backend(&string(value!(), text)?)?,
                "-H" | "--no-hidden" => cli.no_hidden = true,
                "-i" | "--ignore-dirs" => {
                    if !ignore_cli {
                        cli.ignore.clear();
                        ignore_cli = true;
                    }
                    let v = value!();
                    for p in v.as_encoded_bytes().split(|&c| c == b',') {
                        use std::os::unix::ffi::OsStrExt;
                        if !p.is_empty() {
                            cli.ignore.push(std::ffi::OsStr::from_bytes(p).into());
                        }
                    }
                }
                "-I" | "--ignore-dirs-pattern" => {
                    if !patterns_cli {
                        cli.patterns.clear();
                        patterns_cli = true;
                    }
                    cli.patterns.push(string(value!(), text)?);
                }
                "-X" | "--ignore-from" => {
                    if !patterns_cli {
                        cli.patterns.clear();
                        patterns_cli = true;
                    }
                    let path = PathBuf::from(value!());
                    let bytes = crate::input::read_regular(&path, 65_536, "ignore-from", "64 KiB")
                        .map_err(|e| format!("ignore-from: {e}"))?;
                    let contents =
                        String::from_utf8(bytes).map_err(|_| "ignore-from requires UTF-8")?;
                    for line in contents
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    {
                        cli.patterns.push(line.into());
                    }
                }
                "-T" | "--type" => {
                    if !include_cli {
                        cli.include_types.clear();
                        include_cli = true;
                    }
                    cli.include_types.extend(types(value!(), text)?);
                }
                "-E" | "--exclude-type" => {
                    if !exclude_cli {
                        cli.exclude_types.clear();
                        exclude_cli = true;
                    }
                    cli.exclude_types.extend(types(value!(), text)?);
                }
                "-f" | "--input-file" => cli.input = Some(value!().into()),
                "-o" | "--output-file" => cli.output = Some(value!().into()),
                _ if arg.as_encoded_bytes().starts_with(b"-") => {
                    return Err(format!("unknown option: {}", safe_text(&arg)));
                }
                _ => {
                    if cli.path.replace(arg.into()).is_some() {
                        return Err("only one scan path is supported".into());
                    }
                }
            }
        }
        if cli.input.is_some() {
            if cli.no_hidden
                || !cli.ignore.is_empty()
                || !cli.patterns.is_empty()
                || !cli.include_types.is_empty()
                || !cli.exclude_types.is_empty()
            {
                return Err("scan filters cannot be applied to imported snapshots".into());
            }
            cli.read_only = true;
            if cli.path.is_some() {
                return Err("input-file cannot be combined with a scan path".into());
            }
        }
        if cli.top.is_some() && (cli.summary || cli.summarize) {
            return Err("top cannot be combined with summary or summarize".into());
        }
        if cli.summary && cli.summarize {
            return Err("choose --summary JSON or --summarize text".into());
        }
        if cli.interactive && (cli.noninteractive || cli.summary || cli.top.is_some()) {
            return Err("interactive conflicts with noninteractive output options".into());
        }
        if cli.scan.threads == 0 || cli.scan.threads > 64 || cli.scan.max_entries == 0 {
            return Err("threads must be 1..64 and max-entries positive".into());
        }
        if cli.patterns.len() > 256 || cli.patterns.iter().map(String::len).sum::<usize>() > 65_536
        {
            return Err("directory regex patterns exceed 256 patterns or 64 KiB".into());
        }
        Ok(cli)
    }
}
pub const HELP: &str = "rdu — disk usage browser\n\nUsage: rdu [OPTIONS] [--] [PATH]\n\n  --summary                    JSON totals (partial results exit 2)\n  -s, --summarize               Text root total\n  -n, --non-interactive         Text listing; automatic for pipes\n  --interactive                Require a terminal browser\n  -a, --apparent-size           Apparent rather than allocated sizes\n  -t, --threads N               1..64 workers (default 8); --max-cores alias\n  --sequential                 One directory worker\n  --backend auto|bulk|portable  Metadata backend\n  -x, --one-file-system         Do not cross devices (--no-cross alias)\n  --max-entries N               Retained-entry budget (default 10000000)\n  --max-depth N                 Scan descent budget (default 1024; partial on limit)\n  -H, --no-hidden               Exclude hidden directories; keep hidden files\n  -i, --ignore-dirs PATHS       Comma-separated exact directory paths, root-relative\n  -I, --ignore-dirs-pattern RE  Full-path regex directory exclusion; repeatable\n  -X, --ignore-from FILE        UTF-8 regex lines; # comments, max 64 KiB\n  -T, --type EXTS               Include extensions, comma separated, case insensitive\n  -E, --exclude-type EXTS       Exclude extensions; directories remain traversable\n  --sort size|name|count        Listing/browser order\n  --reverse-sort               Reverse selected ordering\n  -C, --show-item-count         Descendant item count column\n  --depth N                    Text display depth only (default 1)\n  --top N                      Recursively rank largest files (-t stays threads)\n  -b, --bytes, --no-prefix      Raw byte output (default for text listings)\n  --human-readable, --si        IEC or decimal human units\n  -k, --show-in-kib             Fixed KiB units\n  --units B|KiB|MiB|GiB|human|si Explicit text units\n  --read-only, --no-delete      Disable all mutation controls\n  -p, --no-progress             Suppress loading animations\n  -q, --quiet                   Suppress ordinary text output\n  -c, --no-color                Monochrome (also honors NO_COLOR)\n  -u, --no-unicode, --ascii     ASCII UI glyphs\n  -o, --output-file FILE        Write RDU lossless snapshot (create new file)\n  -f, --input-file FILE         Open bounded RDU snapshot offline, always read-only\n  --config-file FILE           Explicit versioned JSON defaults (64 KiB; CLI wins)\n  -h, --help                   Help\n  -V, -v, --version            Version\n\nValue-taking long flags also accept --flag=value. JSON totals always use bytes.\nNo implicit config, config writes, shell hooks, or web listener.\nExclusions change scan scope and totals. Imports never authorize filesystem actions.\nDescendant and final-root symlinks are not followed. Live scans observe changes;\nallocated bytes are not a promise of reclaimable space.\n";

#[cfg(test)]
mod config_tests {
    use super::*;
    #[test]
    fn explicit_cli_read_only_wins_over_config_in_either_position() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"version":1,"read_only":false}"#).unwrap();
        for args in [
            vec![
                OsString::from("--config-file"),
                path.as_os_str().to_owned(),
                OsString::from("--read-only"),
            ],
            vec![
                OsString::from("--read-only"),
                OsString::from("--config-file"),
                path.as_os_str().to_owned(),
            ],
        ] {
            assert!(Cli::parse(args).unwrap().read_only);
        }
        std::fs::write(&path, r#"{"version":1,"read_only":true}"#).unwrap();
        assert!(
            Cli::parse([OsString::from("--config-file"), path.into_os_string()])
                .unwrap()
                .read_only
        );
    }
}

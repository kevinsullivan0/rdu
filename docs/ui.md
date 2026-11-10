# Terminal browser

Running `rdu PATH` in a terminal first displays live scan progress, then a Ratatui browser. Use `--read-only` to disable staging and deletion. Imported snapshots and incomplete/error results are always read-only. The browser never launches programs, opens file contents or follows symlink entries.

| Key | Action |
| --- | --- |
| Up / Down, `k` / `j` | Select an entry |
| Enter / Right / `l` | Enter the selected directory |
| Left / Backspace / `h` | Return to the parent directory |
| Page Up / Down, Ctrl-B / Ctrl-F | Move by a screen |
| Home / End, `g` / `G` | Select the first / last entry |
| `s` / `n` / `c` | Sort by size / filename / descendant count |
| `a` | Switch allocated and apparent sizes |
| `/` | Edit a case-insensitive substring filter |
| Enter while filtering | Accept the filter |
| Backspace / Ctrl-U while filtering | Remove one character / clear the filter |
| Escape while filtering | Clear the filter and leave editing |
| `i` | Open or close scrollable details |
| `?` | Open scrollable help; arrows/pages scroll, Escape returns |
| `r` | Discard the old scan/basket and rescan live data |
| Space / `d` | Stage or unstage a selection without filesystem changes |
| `b` | Open or close the staged-target basket |
| `D` | Open final deletion review after fresh preparation |
| `q`, Escape, Ctrl-C, Ctrl-D | Exit the browser |

The loading screen shows elapsed time, actual observed entries/files, completed/discovered directories, active workers, errors and recent entry rate. The final byte totals require hardlink accounting, so progress does not claim a percentage or reclaimed-space estimate. `x` requests stop and keeps partial results. Metadata calls can still block inside the kernel.

The header identifies the size metric, sort order and current directory total. Rows show size, percentage and a usage bar, with textual type and state markers. Wide terminals show a detail pane, medium terminals a drawer and narrow terminals tabs. Short layouts keep essential controls visible. `--no-color` or a nonempty `NO_COLOR` disables color; `--ascii` replaces decorative Unicode. Sizes use binary units.

Final review shows full fresh scope, paths, object count, selected disk usage and policy limitations. A folder includes scan-excluded contents. Press `m` to inspect individual manifest paths. Every review page must have been displayed before a matching typed phrase can confirm; resizing clears the phrase and review history. Terminals smaller than 60 columns by 14 rows cannot confirm. Empty Enter and Escape cancel. During execution, `x`, `q`, Escape or Ctrl-C requests stop; completed removals are not rolled back. The result screen distinguishes purged, preserved, not-attempted and uncertain objects, including truncated details and recovery storage. See [deletion](deletion.md).

Names and paths escape terminal control characters, bidi formatting marks, backslashes, and invalid UTF-8 bytes before display. For example, a literal escape byte becomes `\x1b`; invalid byte `ff` becomes `\xff`. Printable Unicode is preserved. Filtering matches these displayed escaped names. Name sorting uses original filename bytes, so it is deterministic and independent of locale. Width calculations handle common combining marks and East Asian wide characters; complex emoji or unusual terminal Unicode-width settings can still differ.

The terminal guard restores terminal settings, cursor visibility, alternate screen and installed signal handlers on normal exit and I/O errors. SIGINT, SIGQUIT, SIGTERM and SIGHUP request cooperative shutdown. Scans and read-only preparation are cancelled before joining their workers; destructive execution stops admission and retains a report/recovery state. A kernel call must return before its worker can finish. SIGKILL cannot be intercepted.

Run the UI checks with:

```sh
cargo +stable test --lib ui::tests
cargo +stable build
python3 tests/ui_pty.py target/debug/rdu
```

The real PTY suite exercises navigation, sorting, metrics, filtering, help, resize, hostile names and terminal cleanup. Deletion acceptance uses disposable fixtures exclusively: unchanged staging/cancel, scope inspection, exact confirmation, error/cancel outcomes and rescan. It compares terminal modes while excluding macOS's kernel-managed `PENDIN` status bit. Invalid UTF-8 and bidi escaping are also unit tested, including on filesystems that reject invalid UTF-8 filenames. Final release evidence states which checks were actually executed for its exact source and binary.

# RDU

RDU 0.3.0 is a Rust disk usage browser for macOS. Its Ratatui interface provides live scan statistics, directory navigation, usage bars, sorting, filters, offline snapshots and staged permanent deletion. Native bulk metadata reads reduce per-file metadata calls; a portable Unix backend is also available.

## Build and run

Rust 1.88 or newer is required. The selected release was checked with Rust 1.94.1 on Intel macOS/APFS. Apple Silicon and Linux have not been runtime-verified; Windows is unsupported. Linux uses the portable scanner and cannot delete.

```sh
cargo +stable build --release --locked
./target/release/rdu --read-only ~/Downloads
./target/release/rdu --summary ~/Downloads
cargo +stable test --all-features --locked
cargo +stable clippy --all-targets --all-features --locked -- -D warnings
python3 tests/ui_pty.py target/release/rdu
```

For a user-local installation, copy the selected executable into `~/bin` and ensure that directory is on `PATH`, or use `cargo install --path . --locked`.

## Browse and clean up

Arrow keys or `hjkl` navigate; Enter opens a directory and left/Backspace goes up. `a` switches allocated/apparent sizes; `s`, `n`, `c` sort; `/` filters; `i` opens details; `?` shows help; `r` rescans. `x` stops a scan and retains marked partial, read-only results. `q` exits.

Space or `d` stages a selection without changing files. `b` opens the basket; `D` prepares final review. Inspect the full fresh scope, including hidden and scan-excluded contents, then type the displayed `DELETE N` phrase to confirm. Deletion is permanent unlink/rmdir, with no Trash or undo. It is available only after a complete live scan on supported local writable APFS with ownership enforcement. `--read-only` disables deletion. See [deletion policy and recovery](docs/deletion.md) and [terminal controls](docs/ui.md).

```sh
rdu -s --human-readable PATH        # text total
rdu -n --sort count -C PATH         # root entries and counts
rdu --top 20 --human-readable PATH  # largest regular files
rdu -o scan.json PATH              # new offline snapshot
rdu -f scan.json                   # browse snapshot, read-only
rdu --backend portable --threads 1 PATH
rdu --help
```

Successful scans exit 0; partial scans exit 2; argument/setup/output errors exit 1; noninteractive interruption exits 130. [CLI compatibility](docs/cli-parity.md) explains supported flags and omissions. Snapshots never grant deletion authority.

## Accounting, safety and evidence

Allocated size sums `st_blocks * 512`; apparent size sums `st_size`. Directory and symlink metadata count. Regular hardlinks are charged once globally, but every name counts as an entry. Shared hardlink bytes may land in different directory totals with concurrency. Sparse files, compression, APFS clones and filesystem policy complicate physical space accounting; allocated size is not guaranteed reclaimed space.

The final root component must be a directory, not a symlink; ancestor resolution follows ordinary Unix lookup. Child directories are opened with no-follow identity checks. A live scan is not an atomic snapshot. Defaults are 8 workers, a descriptor cache of twice the worker count, ten million retained entries and depth 1,024. Memory grows with names/tree; set `--max-entries` for large or untrusted inputs. Kernel filesystem calls may block cancellation.

This repository uses the simpler original integrated scanner with the complete 0.3 UI, deletion and configuration features. The exact runtime/source binding is in [the source manifest](evidence/publication/source-manifest.json). The [final comparison](evidence/performance/performance.md) retains all 256 observations: the source-equivalent integrated control beat the tested correct GDU full-tree policy across four warm inputs, while default Diskus and fixed-eight dua were faster on the SDK input. The delivered privacy-remapped executable was not independently retimed. The rejected owner prototype’s retained-output gains do not describe this release. Static Standard and exact fallback-diff security reviews recorded zero findings; their scope and limits are in the security policy. [Benchmark method](docs/benchmark-method.md), [security scope](SECURITY.md), [publication provenance](docs/publication.md), and [third-party notices](THIRD_PARTY.md) state the evidence boundaries.

## Selected release checks

The Intel macOS executable is **1,340,928 bytes**, SHA-256 `5741f105c72f32a1f51f9a5844ddf2397837fd1ea8b8bdda53ac4ca1042b524e`. Its exact build source is `ea5969853f52770e627dfb60333194a182f9edc2`; later commits curate documentation/evidence without changing runtime. The [release verification receipt](evidence/release/verification.json) records 123 passing all-features Rust tests, strict Clippy, 18 executable checks and the complete terminal/deletion fixture suite. Compiler paths were remapped; the [executable privacy check](evidence/release/binary-privacy.json) found zero checked personal-path or credential signatures. A single read-only SDK observation validated the usable frame, navigation and terminal restoration; it is functional evidence rather than a latency benchmark. Compiler path remapping changes machine code, so measured source-equivalent control timings are not final-executable timings. Package consumer checks and installation/publication are recorded separately.

Source is MIT licensed.

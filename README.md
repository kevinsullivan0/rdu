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

This repository uses the simpler original integrated scanner with the complete 0.3 UI, deletion and configuration features. The exact runtime/source binding is in [the source manifest](evidence/publication/source-manifest.json). Final comparison and frozen-source security review are pending; this snapshot makes no final benchmark or audit-completion claim. [Benchmark method](docs/benchmark-method.md), [security scope](SECURITY.md), [publication provenance](docs/publication.md), and [third-party notices](THIRD_PARTY.md) state the evidence boundaries.

Source is MIT licensed.

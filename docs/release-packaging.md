# Release packaging

`tools/package_release.py` creates deterministic source and Intel Mac archives from one committed tree, an explicit final binary freeze, locked dependency metadata and the existing notice inventory. It never builds, installs, publishes or replaces an output directory. Run its `--help` and subcommand help for arguments.

Build and product checks are separate from archive integrity. Before packaging, bind the selected normal binary to the committed runtime/Cargo bytes; freeze its hash, byte count, target, build command and toolchain. Preserve complete dependency notices. Package from this curated repository, not a private research checkout.

Verify `SHA256SUMS`, then use `package_release.py verify` on both archives before extracting into new owned directories. Rebuild the source with `cargo +stable build --release --locked`, run the all-features tests and Clippy, and run `bench/verify_binary.py` plus `tests/ui_pty.py` against both supplied and rebuilt binaries. Deletion checks must use owned disposable fixtures. Record actual results and differences; archive verification alone does not prove functionality, source/binary equivalence or security coverage.

Crate source is not vendored. A compatible existing Rust toolchain and available locked dependencies are required.

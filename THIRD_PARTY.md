# Third-party notices and provenance

RDU is MIT licensed. Its Rust implementation is independently authored; it is not a fork or translation of GDU. Native macOS attribute layouts were checked against Apple SDK headers and documentation. Unix bindings come from `libc`.

Direct runtime dependencies are `libc`, `serde`, `serde_json` and `regex-lite` (MIT OR Apache-2.0), and `ratatui` (MIT) with its selected Crossterm backend. Development dependencies include `tempfile`. Exact versions and transitive packages are recorded in `Cargo.lock`.

The `licenses/` directory preserves complete copied copyright, license and notice texts. Its `DEPENDENCIES.json` binds all locked packages, including platform-specific packages, to crate archive checksums and copied notice hashes. Rust standard-library notices from the tested toolchain are included. Copyright holders and attribution in third-party notices are preserved.

The inventory documents three upstream packaging exceptions: r-efi 6.0.0 uses `AUTHORS` as its license record; the two winapi GNU target packages omit standalone licenses, so their original license headers accompany project licenses from the locked winapi 0.3.9 sibling. Donor identities and original source hashes are recorded explicitly.

GDU by Daniel Milde (MIT), Diskus (MIT OR Apache-2.0), and dua (MIT) are independent benchmark controls. Their binaries and implementation source are not redistributed here. Benchmark evidence records exact versions, executable hashes and upstream release references. See [benchmark method](docs/benchmark-method.md) for accounting and comparability limits.

# Publication contents and provenance

This repository is a curated release snapshot. The private research repository and its Git history are preserved separately and are not ancestors of this repository. The published history starts with a sanitized project contributor identity.

Runtime source, Cargo files, source tests and profiling examples are copied byte for byte from the selected source commit recorded in `evidence/publication/source-manifest.json`. Third-party license and copyright text is preserved. Publication documentation is written for this standalone snapshot. Benchmark helpers are explicitly selected; generated fixtures and executable dependencies must be supplied separately.

Task messages, personal author metadata, raw host logs, archived source duplicates, private workspace paths, compiler caches and generated fixtures are omitted. Selected benchmark and security evidence is transformed only for publication privacy. Each transformed artifact has an original SHA-256, a published SHA-256 and an explicit transformation description. Sanitized copies are not the byte-identical authoritative sealed report. The sealed originals remain private and unchanged.

Hashes bind the retained bytes and recorded source version. They do not authenticate an external audit service, certify an executable, imply a signature, or prove that every test has run. The release and security documents state the actual checks and limits.

Final verification is in [the release receipt](../evidence/release/verification.json). The final fallback executable passes the bounded privacy-signature check. Compiler path remapping changes executable machine code, so source-equivalent control timings are explicitly separated from this final artifact. The [performance manifest](../evidence/performance/manifest.json) records exact helper/data provenance; final benchmark helper revisions lie outside the sealed source-review range. No personal research history is an ancestor of this repository.

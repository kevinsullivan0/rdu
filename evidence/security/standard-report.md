> Publication-derived copy of the sealed Standard source review. Original and published hashes are in provenance.json. Internal orchestration/context references are omitted; security conclusions and limitations are retained. This review covers e591e3f (owner scanner), which is superseded by the selected original-scanner fallback; see the separate fallback diff review for the final scanner.

# Security Review: rdu-publication

## Scope

One Standard single-pass static audit of clean sanitized publication commit e591e3f1992fef70039dc5ea9b578c84d6aa3c93, whole repository.84 full-content audited source/config/docs/helper files;198 copied notices classified nonimplementation; all282 tracked files classified and reviewed for embedded credentials.

- Scan mode: repository
- Target kind: git_revision
- Target ID: target_sha256_fe94382fc612b9579319aeb1a531ff9fb4fa0d3457ff17293d7122095ef35d47
- Revision: e591e3f1992fef70039dc5ea9b578c84d6aa3c93
- Inventory strategy: repository
- Included paths: .
- Excluded paths: none

Limitations and exclusions:
- No third-party implementation or fresh vulnerability-advisory audit.
- No foreign-account/child-ACL sticky precedence runtime tests, privileged mount transitions, ownership-disabled real volumes, device opens, arbitrary power-loss, runtime sanitizers, coverage-guided fuzzing or broad hardware/OS certification. Trusted OS semantics are prerequisites, not independently certified by this source pass.
- Live scans are observations; same-UID/root/compromised filesystem adversaries excluded by policy. Pre-pin inode reuse, open writers and nonatomic current-path containment remain documented limits.
- No executable certification or signature implied; publication copies must preserve sealed originals and clearly describe privacy transformations.
- Excluded third-party dependency implementation (not vendored): Locked Cargo dependency names/checksums and notice data reviewed; external crate implementation and fresh advisory database were not audited.
- Excluded runtime/privileged validation beyond static source: No scan-owned builds, tests, PoCs, fuzzing/sanitizers, foreign-account writer, privileged mount/device, arbitrary power-loss, or broad platform/hardware execution. Separate release coordinator validation is distinct.

### Scan Summary

| Field | Value |
| --- | --- |
| Scan outcome | completed |
| Reportable findings | 0 |
| Severity mix | none |
| Confidence mix | none |
| Coverage | complete |
| Validation mode | offline static source review and nonexecuting structured metadata/hash checks |

Canonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.

## Threat Model

RDU 0.3 is an ordinary-user Unix disk metadata browser, with macOS bulk/portable scan, noninteractive text/JSON/snapshot output, a TTY Ratatui browser, and conditional macOS APFS permanent deletion. CLI startup chooses import or live scan and interactive versus output paths (src/main.rs:38, src/main.rs:71, src/main.rs:97, src/main.rs:119); noninteractive scanning never constructs a deletion session, while live UI calls DeletionSession::scan (src/ui/terminal.rs:534). Linux uses portable scanning and returns no deletion session (src/native.rs:375, src/delete.rs:126); Windows is documented unsupported (README.md:7). Package generation is a separate operator tool using immutable Git blobs and explicit binary/freeze/metadata inputs (tools/package_release.py:99, tools/package_release.py:317).

### Assets

- User filesystem object integrity, including unselected/replaced/newly inserted objects; deletion authority is private DeletionSession -\> PreparedPlan -\> consumed ConfirmedPlan rather than imported rows or a phrase alone (src/delete.rs:25, src/delete.rs:172, src/delete.rs:184).
- Scan tree names, identities, allocated/apparent totals and completeness; root is explicit path or '.', normalized to an absolute path and pinned by directory fd; descendants are metadata-only (src/main.rs:47, src/scan.rs:151, src/native.rs:58, src/native.rs:68).
- Private recovery state and raw-name journal are \<canonical scan root\>/.rdu-recovery-\<128-bit random hex\>/journal.jsonl, and captured slots are object-\<16-digit node-index hex\> in that same directory; journal created 0600, recovery verified current-UID owned, 0700, empty ACL before capture (src/delete/macos.rs:860, src/delete/macos.rs:875, src/delete/macos.rs:904, src/delete/macos.rs:933).
- Explicit snapshots contain filesystem names/metadata and are created at exact CLI output path using create_new and mode0600, after bounded serialization; no existing destination is overwritten (src/cli.rs:221, src/snapshot.rs:246).
- Terminal state, finite process resources, release source/binary provenance and notices (src/ui/terminal.rs:37, src/ui/terminal.rs:83, src/scan.rs:82, tools/package_release.py:323).

### Trust Boundaries

- Filesystem-controlled names/metadata cross libc/kernel FFI into Rust. Native child names must be single components; stat uses AT_SYMLINK_NOFOLLOW; macOS bulk records use aligned initialized buffer, checked byte slices/offsets/validity fields and stat fallback for unsupported/nonregular metadata (src/native.rs:25, src/native.rs:58, src/native.rs:168, src/native.rs:185, src/native.rs:249, src/native.rs:279). OS/libc ABI are trusted per SECURITY.md:3.
- Chosen root and mutable descendants cross into directory capabilities. Scan opens directories O_DIRECTORY|O_NOFOLLOW; queued jobs carry names and expected identity, not fds; cached parent opens or component-by-component pinned-root reopens must match dev/inode before enumeration (src/native.rs:68, src/scan.rs:103, src/scan.rs:209, src/scan.rs:247). Ancestors of the caller-supplied scan root may resolve links; capability checks are observation rather than snapshot containment (SECURITY.md:7).
- Explicit config/ignore/snapshot files cross into process data only: config is selected only by --config-file, applied before CLI overrides, bounded64KiB and deny_unknown_fields/version validated; ignore file64KiB; snapshot64MiB with 1million nodes/16MiB names/depth1024/ordered rooted graph validation; final links and nonregular inputs refused (src/cli/config.rs:10, src/cli/config.rs:34, src/cli.rs:102, src/cli.rs:189, src/input.rs:8, src/snapshot.rs:16, src/snapshot.rs:126, src/snapshot.rs:274). Imported origin and read-only CLI/UI path cannot construct a deletion capability (src/cli.rs:233, src/snapshot.rs:311, src/ui/terminal.rs:169).
- Display strings cross into terminal output through safe_text escaping controls, bidi/zero-width controls, backslashes and invalid UTF8; terminal raw/screen modes and handled signal dispositions owned/restored by RAII session (src/ui/format.rs:2, src/ui/format.rs:19, src/output.rs:81, src/main.rs:152, src/ui/terminal.rs:37, src/ui/terminal.rs:83).
- Live UI session -\> immutable review -\> executor: incomplete/error/cancelled/read-only/root/setuid scans cannot mint a session; canonical deletion root requires trusted mode/ACL ancestors and local writable APFS with ownership enabled; UI generation invalidates preparation on stage change/refresh and only matching prepared confirmation is consumed (src/delete/macos.rs:147, src/delete/macos.rs:228, src/delete/macos.rs:268, src/delete/macos.rs:360, src/delete.rs:172, src/ui/terminal.rs:385, src/ui/terminal.rs:435, src/ui/terminal.rs:484).
- Source namespace -\> private purge namespace: selected folders are freshly enumerated independently of scan filters, stamps/type/volume checked, nondirectory opens additionally require trusted current parent mode/ACL (or narrow sticky-owned-object rule); atomic RENAME_EXCL captures each pinned object into an immediate private slot, compares captured identity with retained fd, then unlinks/rmdirs only that slot; restoration also never overwrites (src/delete/macos.rs:243, src/delete/macos.rs:658, src/delete/macos.rs:1034, src/delete/macos.rs:1209, src/delete/macos.rs:1274, src/delete/macos.rs:1323).
- Deletion journaling is recovery evidence, not restart authority. Raw-component graph/all capture slots are appended and fsynced with directory/root before moves; append failure stops admission, final sync failure reports error; bottom-up processing preserves nonempty directories and cleanup refuses unexpected occupants (src/delete/macos.rs:956, src/delete/macos.rs:967, src/delete/macos.rs:1004, src/delete/macos.rs:1089, src/delete/macos.rs:1154, src/delete/macos.rs:1185).
- Operator packaging -\> new release output: exact source helper, locked notice graph, explicit binary hash/header, and final source/tree/toolchain/binary freeze are checked; output is a new operator-selected directory (preview constrained to /tmp), two manifest-bound archives plus RELEASE.json/SHA256SUMS, no implicit publish/install (tools/package_release.py:317, tools/package_release.py:339, tools/package_release.py:348, tools/package_release.py:356, tools/package_release.py:360).

### Attacker Capabilities

- Can supply hostile filenames, symlinks, hardlinks, metadata, directory layout and mutations in namespaces they already control, and explicit malicious display/config/import data when selected by user; cannot initially control process UID, trusted protected source parent/anchor, private recovery directory, kernel/libc or terminal emulator (SECURITY.md:3, SECURITY.md:21; src/delete/macos.rs:243).
- May retain a descriptor to a selected directory and insert children; purge must still be constrained to protected immediate slots and nonrecursive rmdir, preserving new contents (src/delete/macos.rs:1274, src/delete/macos.rs:1328).
- No remote service/tenant/credential issuance surface is established. Same-UID active adversaries/root/trusted owner weakening permissions and malicious filesystem drivers are explicitly excluded; a0700 directory cannot isolate shared UID (SECURITY.md:21).
- A boundary failure could add traversal/content access, terminal control injection, incorrect object purge, silent partial-result or recovery loss, or process resource exhaustion; supplying a path/config/snapshot does not itself grant deletion authority (src/delete.rs:25, src/input.rs:8).

### Security Objectives

- Metadata traversal never follows child symlinks, opens entry contents or invokes a shell; child directory dev/inode must match observed identity (src/native.rs:58, src/native.rs:68, src/scan.rs:209).
- Bound scan workers1..64 (default8), retained entries default10million/depth1024, per-worker directory cache2 (default16 total), descriptor-free queues and registered-work completion; cancellation continues finishing registered jobs and marks incomplete (src/scan.rs:82, src/scan.rs:122, src/scan.rs:138, src/scan.rs:203, src/scan.rs:315, src/scan.rs:372, src/scan.rs:449; src/scan/owner.rs:30, src/scan/owner.rs:82, src/scan/owner.rs:94).
- Count regular hardlink allocation/apparent bytes once globally by(dev,ino), every name as entry; saturating accumulation; displayed allocation is not reclaimed-space promise (src/scan.rs:326, src/scan.rs:389, src/scan.rs:403, README.md:40).
- Prepared deletion closure includes hidden/excluded entries, object cap1million/depth1024/approximately256MiB allocation accounting, plan expiry600seconds from completed preparation, bounded detailed results4096/about8MiB and64KiB journal records; preserve unprovable sources and capture substitutions without purge (src/delete/macos.rs:19, src/delete/macos.rs:486, src/delete/macos.rs:658, src/delete/macos.rs:774, src/delete/macos.rs:781, src/delete/macos.rs:798, src/delete/macos.rs:1209).
- Explicit exact-plan confirmation must precede any recovery creation, rename, unlink or chmod/ACL mutation; only newly created quarantine gets permissions changed; pin/capture/identity verification precedes unlink (src/delete.rs:172, src/delete/macos.rs:878, src/delete/macos.rs:909, src/delete/macos.rs:1240, src/delete/macos.rs:1274, src/delete/macos.rs:1330).
- Surface partial/cancelled/error results; no recursive cleanup, copy/delete fallback, automatic journal replay/resume, secure-erasure or power-loss transaction claim (src/main.rs:134, src/delete/macos.rs:1004, src/delete/macos.rs:1095, src/delete/macos.rs:1185, docs/deletion.md:17, docs/deletion.md:19).

### Assumptions

- Audit scope: sanitized publication tree e591e3f1992fef70039dc5ea9b578c84d6aa3c93, whole repository architecture; offline static reads only; architecture mapping is not completed audit coverage or executable certification.
- No material documentation/configuration discrepancy established in inspected consumers: documented recovery-under-scan-root matches actual canonical-root child; documented cache16 is default8workers\*2slots and grows with explicitly permitted worker count (docs/deletion.md:9, SECURITY.md:23, src/delete/macos.rs:886, src/scan.rs:85, src/scan.rs:122).
- Scanning may cross filesystems by default; --one-file-system prevents child traversal when dev differs. Deletion independently requires same dev and APFS volume identity/mount policy (src/scan.rs:87, src/scan.rs:306, src/delete/macos.rs:335).
- Kernel calls can block and cancellation is cooperative; terminal restoration cannot cover SIGKILL/terminal disappearance; retained tree limits are not process RSS limits (SECURITY.md:23, SECURITY.md:25; src/ui/terminal.rs:519).
- Ownership/mode/ACL protected namespaces and trusted OS/macOS APIs are prerequisites; repeated ancestor validation is not atomic current-path containment, pre-pin inode reuse remains limited, and open writers may modify pinned bytes (SECURITY.md:21; src/delete/macos.rs:122, src/delete/macos.rs:1291).
- Packaging provenance/freeze manifests are supplied operator assertions plus static consistency checks, not proof the binary was built from source; consumer validation remains explicitly pending until separately performed (tools/package_release.py:349, tools/package_release.py:358, docs/release-packaging.md:7).
- Third-party source/advisory audit, foreign-account writers, privileged mount races, arbitrary power failure, broad platform/hardware certification remain outside this static architecture pass (SECURITY.md:29, SECURITY.md:31).

## Findings

### No findings

No reportable findings survived the canonical discovery, validation, and reportability gates.

## Reviewed Surfaces

| Surface | Risk Area | Outcome | Notes |
| --- | --- | --- | --- |
| macOS deletion capability, capture and recovery | Permanent filesystem mutation | No issue found | Independent focused auditor fully reviewed src/delete.rs, src/delete/macos.rs, relevant UI binding/cancellation controls, tests/delete.rs and deletion docs. No source-backed bypass identified. Private confirmed plan, fresh full closure, source parent mode/ACL policy, local writable ownership-enforcing APFS, live pin/exclusive private-slot capture/identity verification, bottom-up nonrecursive removal, checked journal/cancellation/partial recovery reviewed. Platform foreign-UID/ACL, mount and power-loss runtime gaps retained as limitations; no execution performed. Supporting working artifacts retained with the private sealed original. |
| Independent baseline security audit | General source security and embedded credentials | No issue found | Independent baseline returned no validated findings. Full-content review scope and counterevidence retained in original worker artifact; credential/private-key review included all 282 current tracked files including documentation, examples, tests and copied notices. Source review is static and does not establish live secret validity or executable certification. Supporting working artifacts retained with the private sealed original. |
| Native metadata parsing and bounded owner scheduler | Untrusted filesystem names/metadata, FFI and resources | No issue found | Full runtime review of src/native.rs, src/scan.rs, src/scan/owner.rs, progress/profile plus related scan/scheduler/owner tests. Checked aligned initialized native storage, checked slice/offset/length/validity/name parsing and stat fallback; metadata-only stat and directory-only O_NOFOLLOW opens; pinned-root component reopen and dev/ino checks; entry/depth caps and two OwnedFd cache slots per owner, descriptor-free jobs; registered live work, parking generation/recheck and final-zero wake; cancelled placeholders remain complete graph with explicit incomplete status; global regular-hardlink accounting and saturating totals. No candidate survived source analysis. |
| CLI, import/export and terminal authority | Untrusted snapshots/configuration/names and UI confirmation | No issue found | Full source plus Rust tests reviewed. Explicit bounded regular-file nofollow reads, strict typed/versioned JSON and ordered rooted snapshot graph, node/name/depth limits, create-new0600 snapshot output. Imports remain descriptive read-only data. safe_text covers control/bidi/backslash/invalid bytes at display and final diagnostic boundaries. Immutable targets, preparation generation discard, cancellation/join, resize-cleared full review and single-use plan confirmation separate observation from mutation. Terminal guard restores handled state; kernel-blocking and SIGKILL limits documented. |
| Packaging, notices and publication provenance | Release payload integrity and sensitive-source disclosure | No issue found | Full tools/package_release.py reviewed; structured entire Cargo.lock/license inventory and source manifest checked. Current tree has282 tracked regular files and no symlink modes. All274 manifest entries match original or explicitly transformed published SHA256; all198 copied notices match inventory hashes/bytes; Cargo manifest/lock hashes match. Helper reads pinned nofollow bounded regular files, consumes immutable regular Git blobs, checks binary/freeze/lock/metadata consistency, creates new output and verifies regular-only manifest-bound archives. Operator config and output paths are trusted ordinary-user authority. This is consistency review, not signature/build equivalence or executable certification. |
| Benchmark and PTY developer helpers | Operator commands, fixture mutation, output retention and terminal inputs | No issue found | Supplementary investigator reports full helper review with no validated finding, including all previously remaining9 files. Explicit argv executes operator-selected tools under ordinary user authority; mutation/deletion fixtures are owned disposable directories; ratatui real-root work requires --read-only and writable paths require OwnedFixture. Passive bounded PTY decoding never forwards child escapes to user terminal. New results do not establish runtime execution. Original full file coverage and limits are retained in managed helper result. Legacy harness raw host/path/output observations and explicit diagnostic transcripts are local operator artifacts requiring separate publication sanitization; no automatic publishing/network sink was established. Supporting working artifacts retained with the private sealed original. |
| Copied notices and other risk classes | Nonimplementation license data; absent remote service | Not applicable | 198 copied third-party license/copyright texts carry no product implementation, authentication, network listener or mutation entrypoint. Classified as notice-only, included in embedded-credential review and hash/byte integrity verification. SQL/web/SSRF/tenant-auth classes do not apply to this ordinary-user local CLI. All282 inventory files classified; no source paths deferred. |

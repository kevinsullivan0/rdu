> Publication-derived copy of the sealed fallback diff review. Original and published hashes are in provenance.json. Internal orchestration/context statements are omitted; security conclusions and limitations are retained. This review covers the exact e591e3f..ea596985 range and is read together with the earlier Standard review. It does not certify an executable or later documentation/evidence.

# Security Review: rdu-publication

## Scope

Exact immutable e591e3f1992fef70039dc5ea9b578c84d6aa3c93..ea5969853f52770e627dfb60333194a182f9edc2 range: restored src/scan.rs, deleted src/scan/owner.rs, source-manifest schema/provenance and README paragraph.3 managed source items plus1 changed doc accounted; minimal supporting source/tests.

- Scan mode: branch_diff
- Target kind: git_diff
- Target ID: target_sha256_fe94382fc612b9579319aeb1a531ff9fb4fa0d3457ff17293d7122095ef35d47
- Revision range: e591e3f1992fef70039dc5ea9b578c84d6aa3c93...ea5969853f52770e627dfb60333194a182f9edc2
- Snapshot digest: codex-security-snapshot/v1:sha256:3906590afdae7e5cec987557e53f089b0bd95c61854490d10e1a82785f05f45c
- Inventory strategy: diff
- Included paths: .
- Excluded paths: none

Limitations and exclusions:
- Later publication evidence and documentation edits are outside this immutable range.
- No runtime testing/dependency/advisory/platform certification by this scan. Existing documented same-UID/root/kernel/inode-reuse/nonatomic-containment limits unchanged.
- Excluded unchanged code outside delta and direct supporting controls: Prior Standard scan af5de7e9-bc10-41c6-813f-7a35f34c3c09 remains authoritative base review. No second broad audit.
- Excluded runtime/dependency/advisory validation: Offline static delta review only; release coordinator owns builds/functional tests. No executable certification, external dependency source/advisory scan, fuzzing or privileged filesystem tests.

### Scan Summary

| Field | Value |
| --- | --- |
| Scan outcome | completed |
| Reportable findings | 0 |
| Severity mix | none |
| Confidence mix | none |
| Coverage | complete |
| Validation mode | offline source-backed static diff review |

Canonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.

## Threat Model

RDU0.3 remains an ordinary-user metadata browser with unchanged CLI/import/export/UI and conditional macOS APFS deletion authority, as mapped in sealed base Standard scan af5de7e9-bc10-41c6-813f-7a35f34c3c09. This diff restores original global queue/cache scanner at ea5969853f52770e627dfb60333194a182f9edc2 and removes the owner scheduler. Boundaries outside exact delta retain base source coverage; this is a bounded delta review, not a second repository audit (src/scan.rs:1, src/scan.rs:103, src/scan.rs:170).

### Assets

- User filesystem object integrity, including unselected/replaced/newly inserted objects; deletion authority is private DeletionSession -\> PreparedPlan -\> consumed ConfirmedPlan rather than imported rows or a phrase alone (src/delete.rs:25, src/delete.rs:172, src/delete.rs:184).
- Metadata tree names/identities/byte totals/completeness remain existing Entry/Directory/Tree API. Root normalized absolute and pinned; restored worker changes internal job ownership and scheduling without adding filesystem content reads or mutation authority (src/scan.rs:34, src/scan.rs:145, src/scan.rs:157, src/native.rs:58, src/native.rs:68).
- Private recovery state and raw-name journal are \<canonical scan root\>/.rdu-recovery-\<128-bit random hex\>/journal.jsonl, and captured slots are object-\<16-digit node-index hex\> in that same directory; journal created 0600, recovery verified current-UID owned, 0700, empty ACL before capture (src/delete/macos.rs:860, src/delete/macos.rs:875, src/delete/macos.rs:904, src/delete/macos.rs:933).
- Explicit snapshots contain filesystem names/metadata and are created at exact CLI output path using create_new and mode0600, after bounded serialization; no existing destination is overwritten (src/cli.rs:221, src/snapshot.rs:246).
- Terminal state, finite process resources, release source/binary provenance and notices (src/ui/terminal.rs:37, src/ui/terminal.rs:83, src/scan.rs:82, tools/package_release.py:323).

### Trust Boundaries

- Filesystem-controlled names/metadata cross libc/kernel FFI into Rust. Native child names must be single components; stat uses AT_SYMLINK_NOFOLLOW; macOS bulk records use aligned initialized buffer, checked byte slices/offsets/validity fields and stat fallback for unsupported/nonregular metadata (src/native.rs:25, src/native.rs:58, src/native.rs:168, src/native.rs:185, src/native.rs:249, src/native.rs:279). OS/libc ABI are trusted per SECURITY.md:3.
- Chosen root/mutable descendants -\> verified directory capabilities. Unchanged native open_at enforces O_DIRECTORY|O_NOFOLLOW. Restored jobs use either bounded shared-cache or single local parent hint, else component-by-component reopen from pinned root; final opened directory dev/inode must match observed expected identity. Metadata-only native enumeration unchanged; scan is observation, not snapshot containment (src/scan.rs:239, src/scan.rs:254, src/scan.rs:264, src/scan.rs:283, src/native.rs:68).
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

- Metadata traversal never follows child symlinks, opens entry contents or invokes a shell; child directory dev/inode must match observed identity (src/native.rs:58, src/native.rs:68, src/scan.rs:286).
- Bound workers1..64 (default8), entry/depth budgets, shared verified-directory cache2\*workers; shared queued jobs own no descriptors, one worker-local continuation may retain a verified parent. prepare_local consumes current scanned fd before shared completion, releasing leaf final owner. pending counts active, queued and local work and is updated under same work mutex before notify_all; zero permits waiter termination. Cancellation consumes registered jobs with incomplete placeholders (src/scan.rs:119, src/scan.rs:135, src/scan.rs:170, src/scan.rs:200, src/scan.rs:405, src/scan.rs:426).
- Regular hardlink bytes charged once globally by(dev,ino), every name remains an entry; saturating post-scan accumulation remains unchanged by scheduler fallback (src/scan.rs:363, src/scan.rs:456, src/scan.rs:463, src/scan.rs:484). Allocation is not reclaimed-space promise (README.md:40).
- Prepared deletion closure includes hidden/excluded entries, object cap1million/depth1024/approximately256MiB allocation accounting, plan expiry600seconds from completed preparation, bounded detailed results4096/about8MiB and64KiB journal records; preserve unprovable sources and capture substitutions without purge (src/delete/macos.rs:19, src/delete/macos.rs:486, src/delete/macos.rs:658, src/delete/macos.rs:774, src/delete/macos.rs:781, src/delete/macos.rs:798, src/delete/macos.rs:1209).
- Explicit exact-plan confirmation must precede any recovery creation, rename, unlink or chmod/ACL mutation; only newly created quarantine gets permissions changed; pin/capture/identity verification precedes unlink (src/delete.rs:172, src/delete/macos.rs:878, src/delete/macos.rs:909, src/delete/macos.rs:1240, src/delete/macos.rs:1274, src/delete/macos.rs:1330).
- Surface partial/cancelled/error results; no recursive cleanup, copy/delete fallback, automatic journal replay/resume, secure-erasure or power-loss transaction claim (src/main.rs:134, src/delete/macos.rs:1004, src/delete/macos.rs:1095, src/delete/macos.rs:1185, docs/deletion.md:17, docs/deletion.md:19).

### Assumptions

- Exact authorized review is e591e3f1992fef70039dc5ea9b578c84d6aa3c93..ea5969853f52770e627dfb60333194a182f9edc2. Whole-repository model carried from prior sealed Standard and reconciled to changed scanner only. No new broad source/advisory review, app execution or executable certification.
- Source-manifest v2 explicitly distinguishes original/published hashes and declares restored scanner/omitted owner module. All273 declared published entry hashes match immutable head; README describes simpler original scanner. Performance choice is coordinator context, not security evidence.
- Scanning crosses filesystems by default; one_file_system prevents child traversal when dev differs, independently of deletion same-dev/APFS mount policy (src/scan.rs:342, src/delete/macos.rs:335).
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
| Restored global scanner and removed owner scheduler | Directory capabilities, fd lifetime, job termination and partial metadata trees | No issue found | Fully reviewed immutable head src/scan.rs including new completion test, exact diff, and deleted src/scan/owner.rs at base. Unchanged native open_at remains directory-only O_NOFOLLOW. Parent hints/cache use verified fds and final directory dev/ino is rechecked; fallback component opens start at pinned root. Shared queued jobs have pinned_parent None; job.pinned_parent is taken before batch storage. Global cache bounded2\*workers; only one local continuation per worker retains parent, dropped on cancelled open; prepare_local releases leaf fd before Work lock. Mutex-protected pending includes executing/queued/local jobs; completion subtracts one/adds exact all-child count, shared children enqueue and notify occur under same mutex; waiter predicate cannot observe premature zero/lost wake. Every admitted child yields Batch even on cancellation/errors, preserving contiguous IDs and incomplete status. No new content-open, mutation or deletion capability introduced (src/scan.rs:103,121,171,196,211,233,242,269,286,311,351,405,410,419,426,428,504). |
| Source provenance schema and README | Publication/source binding and sensitive literals | No issue found | Full structured manifests at base/head parsed and each changed row semantically compared; all273 declared published-entry hashes match immutable head. v2 names original/published hash/byte/source fields explicitly, removes unused owner module, declares original scanner restoration and preserves transformed CLI doc provenance. README single changed paragraph describes original integrated scanner. No executable code or credentials added. Managed source inventory lists3 paths; README fourth changed documentation path was additionally read from exact Git diff. |
| Supporting native, session and regression contracts | Changed scanner consumers and negative controls | No issue found | Read minimum unchanged native open/stat/enumerate and deletion-session scan gates, full scheduler_contracts and owner_fd_fixture sources, plus restored completion unit test. Existing Standard base full-source/secret coverage retained independently, not rerun. Partial/error/cancelled scans still cannot mint deletion session (src/delete/macos.rs:380). Tests assert bounds, tree links/accounting, cancellation and low-fd disposable fixture behavior; no test execution claimed. |

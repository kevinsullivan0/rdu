# Early deletion threat review

This is a design review, not an implementation audit or approval to delete real data. Baseline RDU release: `3108068bbee5e6b79a41a04282ef72a6cfc7e767`, whose security contract is read-only. The proposed release adds live browsing and staged deletion with an explicit final in-app confirmation. Tests may mutate only isolated disposable fixtures. A separate actual Codex Security review is required after implementation freezes.

## Recommendation and blockers

Ship **confirmed quarantine first**. Enable permanent purge only behind the additional protected-slot and recovery gates below. A direct recursive `fstatat`/`unlinkat` walk is not a defensible substitute.

| Blocking issue | Required design consequence |
| --- | --- |
| `unlinkat` removes the entry currently named in a directory; it accepts no expected inode argument. A preceding stat does not bind that later operation. | Never call the final unlink/rmdir in a namespace another actor can replace. Opening the parent FD alone does not make its entries immutable. |
| Moving a subtree under a private directory does not privatize its descendants. A foreign process can retain a directory FD into a still-writable descendant. | Do not recursively unlink names inside the captured subtree. Capture each purge candidate into a protected immediate-parent slot first, or stop at quarantine-only. |
| `RENAME_EXCL` prevents overwriting an existing destination, not substitution of the source. | Compare the captured object with a retained identity handle. On mismatch, preserve it and report a conflict. An unexpected object may already have been relocated; do not describe this as “no unconfirmed mutation.” |
| Mode `0700` is not proof of ACL privacy; ownership can also be disabled on a volume. | Establish and read back owner, mode, extended ACL and mount policy for the quarantine and its trusted parent before transferring user objects. Fail closed if unavailable. |
| `(dev, ino)` saved in a long-lived manifest can become stale or be reused. | Retain suitable object handles across the authority-bearing interval, within an explicit FD/plan budget. Stat generation/timestamps are corroboration, not a universal replacement for a live handle. |
| Rename, journal update and unlink are not one transaction. | Define recoverable partial states and uncertain outcomes before implementing purge. Never auto-purge on restart. |

The unlink and no-overwrite distinctions follow the syscall interfaces, not a limitation of Rust. Apple documents name-based unlink, relative directory descriptors and filesystem-dependent rename flags. [Apple unlink manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/unlink.2.html), [current Apple rename manual source](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/man/man2/rename.2).

## Realistic security boundary

Protect against hostile names, symlinks, replacement entries and other-UID writers that cannot modify the protected quarantine namespace. Preserve unexpected objects rather than extending the confirmed plan. Assume a trusted kernel/local filesystem and ordinary, non-elevated RDU credentials. Refuse root/setuid operation for deletion.

Do **not** promise isolation from an adversarial same-UID process, root, compromised filesystem driver, or an actor already able to change the quarantine's permissions or entries. `0700`, opaque names and an advisory lock do not separate processes sharing the same UID. Normal concurrent editors also mean that confirming an inode is not confirming immutable file contents: an already-open write FD can change bytes after capture. Do not advertise secure erasure or a fixed amount of reclaimed space.

For a small first implementation, restrict purge to tested local APFS volumes, effective ownership enforcement, a trusted user-owned quarantine anchor on that same filesystem, and supported atomic no-overwrite rename. Reject network/FUSE/ownership-disabled volumes, foreign-owned anchors, mount roots, special nodes and uncertain policy. Failure to meet a gate should leave deletion disabled or offer explicitly confirmed quarantine-only—not silently weaken the method. Apple's permission guide treats ACLs and volume ownership settings separately from Unix mode bits. [Apple permission model](https://developer.apple.com/library/archive/documentation/Security/Conceptual/AuthenticationAndAuthorizationGuide/Permissions/Permissions.html).

## Minimal operation contract

1. **Stage in memory.** Bind the selection to a scan generation and immutable object identity, not a live row number or current sort position. Live progress may replace browse snapshots; it must not retarget a staged action. Exports/imports, cached trees and filters provide display data only. An imported item requires a fresh live rescan before it can enter a deletion plan.
2. **Prepare a fresh bounded plan.** Resolve descriptor-relative single components without following symlinks; verify current root/parent/object identities. Record raw name bytes, object type, identity, ownership, selected policy and exact manifest closure. Include hidden/excluded children in the decision explicitly. Fail on incomplete enumeration, resource limits, unsupported identities or crossed mounts. A retained browse FD is not deletion authority: it may refer to a directory that moved outside the displayed path.
3. **Confirm the exact plan.** Show escaped paths, object counts, incomplete/changed status, quarantine location, and whether the action relocates or permanently removes objects. Bind a single-use confirmation capability to the plan digest, scan generation, action and process session. A typed phrase alone is not a capability. Changed selection, root, policy, rescan or plan invalidates it. Expire abandoned plans and release handles. If the product requires dynamic pathname containment, document that concurrent ancestor renames cannot be atomically excluded by repeated stat checks.
4. **Establish quarantine after confirmation.** Open a trusted same-filesystem anchor; create a fresh unpredictable child without reusing existing directories. Verify its identity/owner/mode and remove or replace inherited extended ACLs through its FD, then read back the effective policy. Do not put user objects there before these checks pass. Do not chmod/strip ACLs recursively on user data as an implicit safety fix. The anchor must be outside every selected subtree. Disable purge when ownership is ignored. Only one executor owns an operation session.
5. **Capture without overwrite.** Use `renameatx_np` with tested `RENAME_EXCL` semantics and a fresh opaque destination component. Preserve the source-parent and expected-object capabilities. Verify the destination against the retained object handle after the move. A mismatch becomes a preserved conflict; do not unlink it or automatically move it over a new source occupant. No copy-and-delete fallback for `EXDEV` or unsupported flags. Renamed objects retain their own ACLs; moving them does not sanitize a subtree. [Apple rename source](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/man/man2/rename.2).
6. **Optional protected-slot purge.** For every confirmed manifest object, capture it into a fresh slot directly beneath the verified private quarantine directory, compare its identity with the live confirmed handle, then unlink only that slot. Process directories bottom-up through the same capture/verify path and use nonrecursive `AT_REMOVEDIR`; a newly populated directory must remain with its unexpected entries. Do not enumerate new entries and add them to the plan. A foreign retained FD into a captured directory may add children, causing a safe nonempty failure, but cannot replace the directory's slot in the private parent. This reasoning excludes adversarial same-UID writers and requires the parent privacy gate to remain true.
7. **Report exact outcomes.** Distinguish not attempted, captured, mismatched/preserved, purged, nonempty/preserved, restored, failed and uncertain. Cancel stops admission of new mutations; the syscall already in progress can complete. Do not claim cancellation undoes earlier work or turn partial execution into a success banner.

Holding every confirmed object handle may exceed practical FD limits for large subtrees. That is a release gate, not a reason to drop identity binding. Cap plans, use separately confirmed bounded batches, or keep the operation quarantine-only. Candidate macOS metadata-handle flags such as `O_EVTONLY`/`O_SYMLINK` need disposable-fixture validation for the exact supported types and OS versions; do not assume they work for every node. No regular-file contents need be read.

A final symlink is a separate object, not its target. Permit permanent removal of that link only when its own metadata identity can be pinned and checked without following it; otherwise preserve/quarantine it. Intermediate symlinks never become traversal authority. Mount boundaries need explicit detection: device-number equality alone is not a general proof that no mount transition occurred, and kernel refusal is a final backstop, not the entire plan policy.

## Crash, recovery and cleanup

Create a versioned journal inside protected storage after confirmation and before the first transfer. Persist an intent containing the operation ID, source-parent identity, raw component, expected object identity and opaque destination slot; then record the observed result. Durability requires checking flush results and the supported filesystem ordering of journal and directory changes. `fsync` alone is not an unconditional power-loss guarantee; Apple describes stronger `F_FULLFSYNC` behavior. Do not promise a transaction across filesystems or claim crash testing proves arbitrary power-loss safety. [Apple fsync manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fsync.2.html).

On restart, a journal is a reconciliation hint, never a reusable confirmation token. Reopen through the trusted anchor without following links; validate journal format and live identities. Missing, duplicate, corrupt or contradictory state means preserve and report. Restoration is another no-overwrite operation with an explicit user decision; a re-created original name blocks automatic restoration. A crash or I/O error between mutation and journal completion must not trigger blind retries or cleanup.

Remove only known RDU-created journal files and known empty quarantine directories whose identities are validated beneath a trusted parent. Never call recursive cleanup on a quarantine, even if its name has RDU's prefix. Unexpected files keep the directory and a visible recovery record. If the quarantine's parent can be renamed or populated by an excluded writer, leave the directory rather than applying a final name-based rmdir in that mutable parent.

## API and test gates

Keep the executor separate from Ratatui/live scanning. Its only mutating entry point should require an unforgeable, single-use confirmed-plan value; UI handlers and imports cannot synthesize it. Central `--read-only` enforcement must prevent quarantine, rename, unlink, permission/ACL changes and recovery cleanup regardless of key bindings or background events. Benchmark commands must supply this guard. A separate explicit export destination is not deletion authority.

Use deterministic interleaving hooks in disposable fixtures, not timing-only race loops:

| Fixture/interleaving | Required result |
| --- | --- |
| Replace a source entry after validation, before capture | No destination overwrite; moved mismatch preserved; no unlink. |
| Replace an entry immediately before a proposed final unlink | Final unlink exists only in a proven private slot; direct mutable-parent variant is rejected. |
| Retain a writable descendant dirfd before subtree capture | Unexpected child/replacement survives; no recursive deletion through that FD's namespace. |
| Inheritable ACL grants another UID search/write or delete-child | Quarantine privacy setup/readback succeeds before capture or mutation is refused. Mode-only check must fail the test. |
| Ownership-disabled volume, foreign owner, intermediate symlink, mount point or unsupported rename flag | Fail closed with no copy/delete fallback. Privileged mount tests remain a separate authorized suite. |
| Confirmed final symlink points outside the selected tree | Outside target is unchanged; only the link may be captured/purged under the supported identity policy. |
| Same-UID injector mutates quarantine | Document unsupported isolation; demonstrate that ordinary mode bits cannot justify the stronger claim. |
| Existing destination, re-created source name, unexpected quarantine entry | Preserve all occupants; require explicit recovery decision. |
| Crash/error/cancel at every journal/capture/verify/purge boundary | Reconciliation is idempotent, uncertainty visible, no automatic purge or overwrite. |
| Import, filter, live reorder/rescan, replayed/expired confirmation or `--read-only` | No mutating capability is produced; no filesystem effect. |

Other-UID/ACL cases may need a controlled helper or separate account; if unavailable, record the gap and keep the associated guarantee disabled. This review performed only read-only source/document research and wrote this document: no implementation, deletion, permission change, build or runtime test occurred.

Primary contracts checked: installed macOS SDK `rename(2)`, `unlink(2)`, `chmod(2)`, `fsync(2)`, and `sys/acl.h`, `sys/fcntl.h`, `sys/stat.h`, `sys/mount.h`; Apple's public manuals and permission guide linked above. Current XNU source corroborates that unlink authorization/removal consumes a resolved directory/name operation, not an application-supplied expected-inode token: [Apple VFS source](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/vfs/vfs_syscalls.c). Newer SDK flags are not proof of support on every deployment filesystem. No Apple manual or source implementation is copied into this design.

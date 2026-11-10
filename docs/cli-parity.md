# Practical GDU command-line compatibility

RDU keeps its existing JSON `--summary` and `-t/--threads` contracts. GDU's `-t` means top files; RDU exposes that behavior as `--top N`. This document specifies the supported subset; RDU does not claim complete GDU compatibility.

## Implemented options

| Capability | RDU flags and behavior |
| --- | --- |
| Scan exclusions | `-i/--ignore-dirs PATHS` accepts comma-separated exact directory paths, relative to the explicit scan root or absolute. Repeatable `-I/--ignore-dirs-pattern RE` matches full directory paths with regex-lite regular expressions. `-X/--ignore-from FILE` combines one regex per nonblank, noncomment line; UTF-8, regular files only with final symlinks rejected, 64 KiB maximum, at most 256 patterns/64 KiB combined. These are regexes, not globs. |
| Hidden directories | `-H/--no-hidden` prunes directory names beginning with `.`; hidden files are retained. No default exclusions are silently added. |
| File extensions | `-T/--type EXTS` and `-E/--exclude-type EXTS` accept comma-separated extensions, case insensitive, with optional leading dots. Exclusion wins. An empty extension denotes extensionless names. Directories remain traversable. Non-directory metadata entries are filtered without reading contents. |
| Text summaries | `-s/--summarize` prints a root total and escaped path. `--summary` continues to emit JSON with both byte metrics, counts, errors and elapsed time. |
| Ordering and counts | `--sort size\|name\|count`, `--reverse/--reverse-sort`, `-C/--show-item-count`. Directory item count means all retained descendant entries, excluding the directory itself; ordinary files have zero descendants. |
| Output scope | `--depth N` controls text display depth, with zero showing only the total. It never truncates scanning. `--max-depth N` remains a scan budget and reports partial results when reached. `--top N` recursively ranks regular files using a bounded heap, then prints the root total. |
| Units | Default text output stays raw bytes. `-b/--bytes/--no-prefix`, `--human-readable` (IEC), `--si`, `-k/--show-in-kib`, or `--units B\|KiB\|MiB\|GiB\|human\|si`. JSON always uses byte integers. |
| Worker aliases | `-m/--max-cores` aliases the bounded directory-worker count; `--sequential` selects one worker. This is not GDU's Go runtime CPU cap. |
| Existing flag aliases | `--show-apparent-size` aliases `--apparent-size`; `--no-cross` aliases `--one-file-system`; `-v` aliases `-V/--version`. |
| Terminal controls | `-n/--non-interactive`, `--interactive` (requires terminal stdin/stdout), `-p/--no-progress`, `-q/--quiet`, `-c/--no-color`, `-u/--no-unicode/--ascii`. Quiet suppresses ordinary text; explicitly requested JSON remains available. |
| Mutation authority | `--read-only/--no-delete` disables mutation controls. Imported snapshots are always offline/read-only. Filters change scan scope; folder deletion must review the deletion service's fresh full manifest, which can include scan-excluded content. |
| Explicit defaults | `--config-file FILE` loads strict version-1 JSON, at most 64 KiB, from a regular file without following a final symlink. No implicit files are loaded and no config is written. Command-line values override config regardless of option position. |
| Snapshots | `-o/--output-file FILE` exports the retained tree and selects noninteractive operation. `-f/--input-file FILE` opens an offline snapshot. Export creates a new mode-0600 file and refuses overwrite. It can be combined with import to re-export a snapshot, but import cannot be combined with a live scan path or scan filters. |

Exact paths preserve raw filename bytes. Regex matching uses a lossy UTF-8 view of a full path because regex-lite operates on strings; use exact exclusions for paths whose raw names cannot be represented in UTF-8. RDU's hardlink deduplication, allocated/apparent accounting, no-follow traversal, partial-result exit 2 and output-failure exit 1 remain in force. Scan filters run before retained-entry admission and do not spend the retained-entry budget on excluded entries. Unreadable metadata still reports errors.

## Explicit JSON configuration

Select one file with `--config-file settings.json` or `--config-file=settings.json`. A minimal file is `{"version": 1}` and reproduces the ordinary defaults. For example:

```json
{
  "version": 1,
  "threads": 8,
  "no_hidden": true,
  "ignore_dirs": [".git", "target"],
  "sort": "size",
  "show_count": true,
  "units": "human",
  "read_only": true
}
```

Accepted optional keys are `threads`, `backend`, `one_file_system`, `max_entries`, `max_depth`, `no_hidden`, `ignore_dirs`, `ignore_patterns`, `include_types`, `exclude_types`, `apparent`, `sort`, `reverse`, `show_count`, `no_color`, `ascii`, `no_progress`, `read_only`, `units` and `depth`. They use the corresponding CLI value types and limits. Arrays contain individual paths, regular expressions or extensions; exact paths remain relative to the scan root, rather than the config file. Unknown/duplicate keys, unsupported versions, invalid scalar defaults and action keys such as roots, input/output files, summary selection or commands are errors. Optional `null` fields are equivalent to absent fields.

Precedence is built-in defaults, then the explicit config, then explicit CLI options. The first CLI occurrence of an exact-ignore, include-type or exclude-type flag replaces that configured list; later occurrences append. `-I` and `-X` share the same pattern list, with the first of either replacing configured patterns. `--ignore-dirs=` clears configured exact exclusions. Config boolean defaults can be enabled by their CLI flags; existing flags do not acquire inverse forms. An explicit CLI `--read-only` always wins, including when the file says `"read_only": false`.

Value-taking long options accept both `--flag value` and `--flag=value`. Boolean flags do not accept `=true`/`=false`. Values beginning with `--` remain data when supplied to a value-taking option; arguments after a standalone `--` are positional paths. `--top` cannot be combined with `--summary` or `--summarize`.

## RDU snapshot version 1

The format is a single JSON object with `schema: "rdu-snapshot"`, `version: 1`, a raw-byte `root` array, `files`, `errors`, `complete`, `elapsed_nanos` and a flat `dirs` array. Each directory stores its raw-byte name, nullable parent index, device/inode identity, allocated/apparent totals, local errors and entries. Each entry stores its raw-byte name, kind (`file`, `directory`, `symlink`, `other`), device/inode identity, both byte metrics and nullable child index. Parent indices precede children; every directory except the root must have exactly one matching incoming entry. Original names and integer totals survive round trips without UTF-8 conversion. Stored identities are descriptive data and never grant filesystem authority.

Import accepts regular files only and rejects final symlinks. The document limit is 64 MiB, with at most one million directory/entry nodes, 16 MiB of combined raw names, 64 KiB per name and directory depth 1024. JSON parser nesting limits remain enabled. Graph cycles, out-of-range indices, orphaned children, duplicate sibling names, invalid name components, mismatched identities, negative/overflowing integers and inconsistent file counts are rejected. Export applies the same size/node/name limits before creating its destination. A later I/O failure may leave a partial export; RDU reports it without unlinking a path another process may have replaced.

This schema is RDU-specific. GDU/ncdu JSON interoperability, compression and database replay are not implemented. An incomplete snapshot returns exit 2 even when its recorded error count is zero. Saved data is displayed as observed metadata; reopening it never rescans or enables deletion.

## Explicit omissions

The next lightweight release does not implement automatic or YAML configuration, config writes, gitignore semantics, multiple/file roots, date/mtime filters, file-target symlink following, annex/exclusive extent accounting, disk selection, content/archive browsing, arbitrary shell/open hooks, persistent databases, a live web server, or runtime HTTP profiling. Environment block-size variables are not interpreted. UI-specific keys and deletion capabilities are tracked separately from this command-line subset. No optional static HTML export is advertised.

#!/usr/bin/env python3
"""Local, offline RDU notice generation and deterministic release packaging (Python 3.11+).

Never builds, installs, publishes, executes the product, or overwrites an output directory.
Final packaging requires an explicit operator-supplied source/binary freeze record.
"""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import struct
import subprocess
import sys
import zlib
import tarfile
import tomllib
from urllib.parse import unquote, urlsplit

LIMIT = 32 * 1024 * 1024
EXCLUDED = ('.git', '.superpowers', '.agents', '.codex', '.aws', 'target', 'dist', 'bench/bin', 'bench/data')
PRIVATE_NAMES = {'task-source.txt', 'task-steering.txt', 'architecture-steering.txt', 'steering-reader-excerpts.json', '.env', '.netrc', 'credentials'}
NOTICE = re.compile(r'^(licen[cs]es?|copying|copyright|notice|unlicense)([._-]|$)', re.I)
MACHINE = (b'\x7fELF', b'\xcf\xfa\xed\xfe', b'\xfe\xed\xfa\xcf', b'\xce\xfa\xed\xfe', b'\xfe\xed\xfa\xce', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca', b'MZ')


def fail(message):
    raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()


def command(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd)


def relative(value):
    path = PurePosixPath(value)
    if path.is_absolute() or not path.parts or any(p in ('..', '.') for p in path.parts) or '\\' in value or any(ord(c) < 32 for c in value):
        fail(f'unsafe relative path: {value!r}')
    return path


def safe_read(root, name, limit=LIMIT):
    """Open every descendant component through a pinned no-follow directory fd."""
    parts = relative(str(name)).parts
    fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        for index, part in enumerate(parts):
            flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK
            if index + 1 < len(parts):
                flags |= os.O_DIRECTORY
            next_fd = os.open(part, flags, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > limit:
            fail(f'notice/input is not a bounded regular file: {name}')
        with os.fdopen(fd, 'rb', closefd=False) as stream:
            data = stream.read(limit + 1)
        if len(data) > limit:
            fail(f'input grew beyond limit: {name}')
        return data
    finally:
        os.close(fd)


def write_new(root, name, data, mode=0o644):
    path = root.joinpath(*relative(name).parts)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    with os.fdopen(fd, 'wb') as output:
        output.write(data)
    os.chmod(path, mode)


def new_directory(path):
    path = path.absolute()
    if path.is_symlink() or path.exists():
        fail(f'refusing to replace output: {path}')
    path.mkdir(mode=0o755)
    return path.resolve()


def git_blob(repo, object_id):
    return command('git', 'cat-file', 'blob', object_id, cwd=repo)


def git_source(repo, ref):
    commit = command('git', 'rev-parse', '--verify', ref + '^{commit}', cwd=repo).decode().strip()
    tree = command('git', 'rev-parse', commit + '^{tree}', cwd=repo).decode().strip()
    stamp = int(command('git', 'show', '-s', '--format=%ct', commit, cwd=repo))
    files = {}
    blobs = {}
    excluded = []
    for record in command('git', 'ls-tree', '-rzl', '--full-tree', commit, cwd=repo).split(b'\0'):
        if not record:
            continue
        header, raw_name = record.split(b'\t', 1)
        mode, kind, object_id, size = header.decode().split()
        name = raw_name.decode('utf-8')
        relative(name)
        if any(name == p or name.startswith(p + '/') for p in EXCLUDED) or PurePosixPath(name).name in PRIVATE_NAMES:
            excluded.append(name)
            continue
        if kind != 'blob' or mode not in ('100644', '100755'):
            fail(f'source contains link/submodule/special entry: {name}')
        if int(size) > LIMIT or sum(len(v[0]) for v in files.values()) + int(size) > 512 * 1024 * 1024:
            fail('source payload exceeds packaging size bounds')
        if object_id not in blobs:
            blobs[object_id] = git_blob(repo, object_id)
        data = blobs[object_id]
        if len(data) > LIMIT or any(data.startswith(magic) for magic in MACHINE):
            fail(f'source contains oversized data or a machine executable: {name}')
        files[name] = (data, 0o755 if mode == '100755' else 0o644, object_id)
    return commit, tree, stamp, files, excluded


def graph(metadata):
    packages = {p['id']: p for p in metadata['packages']}
    def identity(value):
        p = packages[value]
        return f"{p['name']}@{p['version']}" + ('' if p.get('source') else ' (workspace)')
    return sorted([{'package': identity(n['id']), 'features': sorted(n['features']),
                    'dependencies': sorted([{'package': identity(d['pkg']), 'kinds': d['dep_kinds']} for d in n['deps']], key=lambda d: d['package'])}
                   for n in metadata['resolve']['nodes']], key=lambda n: n['package'])


def inventory(args):
    commit = command('git', 'rev-parse', '--verify', args.source + '^{commit}', cwd=args.repo).decode().strip()
    lock_bytes = command('git', 'show', commit + ':Cargo.lock', cwd=args.repo)
    manifest_bytes = command('git', 'show', commit + ':Cargo.toml', cwd=args.repo)
    locked = [p for p in tomllib.loads(lock_bytes.decode())['package'] if p.get('source')]
    metadata_bytes = safe_read(args.metadata.parent.resolve(), args.metadata.name)
    metadata = json.loads(metadata_bytes)
    selected = {(p['name'], p['version']): p for p in metadata['packages'] if p.get('source')}
    project = tomllib.loads(manifest_bytes.decode())['package']
    workspace = [p for p in metadata['packages'] if p['id'] in metadata['workspace_members']]
    if len(workspace) != 1 or (workspace[0]['name'], workspace[0]['version']) != (project['name'], project['version']):
        fail('metadata workspace does not match frozen Cargo package')
    declared = tomllib.loads(manifest_bytes.decode())['dependencies']
    direct = {d['name']: d for d in workspace[0]['dependencies'] if d['kind'] is None and d['target'] is None}
    if set(declared) != set(direct):
        fail('metadata direct dependencies differ from frozen Cargo manifest')
    for name, requirement in declared.items():
        settings = requirement if isinstance(requirement, dict) else {}
        if sorted(settings.get('features', [])) != sorted(direct[name]['features']) or settings.get('default-features', True) != direct[name]['uses_default_features']:
            fail('metadata direct feature selection differs from frozen Cargo manifest')
    registry = args.registry.resolve(strict=True)
    cached = {}
    for package in locked:
        if package['source'] != 'registry+https://github.com/rust-lang/crates.io-index':
            fail('only the current crates.io lock graph is supported')
        matches = list(registry.glob('*/' + package['name'] + '-' + package['version']))
        if len(matches) != 1:
            fail(f'expected one cached locked package: {package["name"]}@{package["version"]}')
        prefix = matches[0].relative_to(registry).as_posix()
        cargo_bytes = safe_read(registry, prefix + '/Cargo.toml')
        cargo = tomllib.loads(cargo_bytes.decode())['package']
        archive_bytes = safe_read(registry.parent / 'cache', str(PurePosixPath(prefix).parent / (package['name'] + '-' + package['version'] + '.crate')))
        if (cargo['name'], cargo['version']) != (package['name'], package['version']) or digest(archive_bytes) != package['checksum']:
            fail('cached package identity/archive checksum differs from Cargo.lock')
        checksums = {'files': {}}
        wanted = {'Cargo.toml', 'README.md', 'AUTHORS', 'src/lib.rs', cargo.get('license-file', cargo.get('license_file', ''))}
        wanted.discard('')
        with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode='r:gz') as crate:
            for member in crate:
                parts = relative(member.name).parts
                if parts[0] != package['name'] + '-' + package['version']:
                    fail('crate archive has an unexpected root')
                name = '/'.join(parts[1:])
                if name in wanted or (len(parts) == 2 and NOTICE.match(name)):
                    if not member.isfile() or member.size > LIMIT:
                        fail('crate notice is not a bounded regular file')
                    data = crate.extractfile(member).read(LIMIT + 1)
                    checksums['files'][name] = digest(data)
        if checksums['files'].get('Cargo.toml') != digest(cargo_bytes):
            fail('cached manifest differs from checksum-verified crate archive')
        cached[(package['name'], package['version'])] = (package, prefix, cargo, cargo_bytes, checksums)
    if not set(selected).issubset(cached):
        fail('metadata contains packages absent from the frozen lock')
    output = new_directory(args.output)
    records = []
    for key, (package, prefix, cargo, cargo_bytes, checksums) in sorted(cached.items()):
        root = registry / prefix
        notices = [p.name for p in root.iterdir() if NOTICE.match(p.name)]
        declared = cargo.get('license-file', cargo.get('license_file'))
        if declared:
            relative(declared)
            notices.append(declared)
        if key[0] == 'r-efi' and not notices:
            notices = ['AUTHORS']  # README identifies AUTHORS as its complete license/author record.
        donor_key = key
        exception = None
        if key[0] == 'r-efi' and notices == ['AUTHORS']:
            declaration = safe_read(registry, prefix + '/README.md')
            if checksums['files'].get('README.md') != digest(declaration) or b'AUTHORS' not in declaration:
                fail('r-efi AUTHORS license declaration differs from verified crate')
            exception = {'reason': 'README identifies AUTHORS as the complete license/author record', 'declaration_source': 'README.md', 'declaration_sha256': digest(declaration)}
        if not notices and key[0] in ('winapi-i686-pc-windows-gnu', 'winapi-x86_64-pc-windows-gnu'):
            donor_key = ('winapi', '0.3.9')
            donor = cached[donor_key]
            if cargo.get('repository') != donor[2].get('repository') or cargo.get('license') != donor[2].get('license'):
                fail('winapi notice donor must have identical upstream repository/license declaration')
            notices = ['LICENSE-APACHE', 'LICENSE-MIT']
            exception = {'reason': 'target crate omits standalone project notices', 'donor': 'winapi@0.3.9', 'repository': cargo['repository']}
        if not notices or not cargo.get('license'):
            fail(f'missing authoritative local license text: {key}')
        donor = cached[donor_key]
        files = []
        for name in sorted(set(notices)):
            data = safe_read(registry, donor[1] + '/' + name)
            data.decode('utf-8')
            if donor[4]['files'].get(name) != digest(data):
                fail(f'notice differs from cached crate file checksum: {key} {name}')
            destination = f'{key[0]}-{key[1]}/{name}'
            write_new(output, destination, data)
            files.append({'path': destination, 'sha256': digest(data), 'bytes': len(data), 'source_package': '@'.join(donor_key), 'source_path': name})
        if donor_key != key:
            data = safe_read(registry, prefix + '/src/lib.rs')
            if checksums['files'].get('src/lib.rs') != digest(data):
                fail('target package notice header differs from verified crate archive')
            header = b'\n'.join(line for line in data.splitlines() if line.startswith(b'//')) + b'\n'
            destination = f'{key[0]}-{key[1]}/NOTICE-PACKAGE-HEADER.txt'
            write_new(output, destination, header)
            files.append({'path': destination, 'sha256': digest(header), 'bytes': len(header), 'source_package': '@'.join(key), 'source_path': 'src/lib.rs', 'source_sha256': digest(data), 'extraction': 'comment lines preserving original package copyright/license header'})
        records.append({'name': key[0], 'version': key[1], 'license': cargo['license'], 'source': package['source'], 'package_checksum': package['checksum'], 'manifest_sha256': digest(cargo_bytes), 'in_supplied_metadata': key in selected, 'notice_exception': exception, 'files': files})
    sysroot = Path(command('rustc', '+' + args.toolchain, '--print', 'sysroot').decode().strip()).resolve(strict=True)
    std_files = ['COPYRIGHT-library.html'] + sorted('licenses/' + p.name for p in (sysroot / 'share/doc/rust/licenses').iterdir())
    standard = []
    for name in std_files:
        data = safe_read(sysroot, 'share/doc/rust/' + name)
        data.decode('utf-8')
        destination = 'rust-standard-library/' + name
        write_new(output, destination, data)
        standard.append({'path': destination, 'sha256': digest(data), 'bytes': len(data), 'toolchain_source': 'share/doc/rust/' + name})
    normalized_graph = graph(metadata)
    value = {'format': 'rdu-license-inventory-v1', 'source_commit_examined': commit, 'cargo_toml_sha256': digest(manifest_bytes), 'cargo_lock_sha256': digest(lock_bytes), 'cargo_metadata_sha256': digest(metadata_bytes), 'graph_sha256': digest(encoded(normalized_graph)), 'resolved_graph': normalized_graph, 'packages': records, 'standard_library': {'rustc_vv': command('rustc', '+' + args.toolchain, '-Vv').decode(), 'cargo_version': command('cargo', '+' + args.toolchain, '-V').decode().strip(), 'files': standard}}
    write_new(output, 'DEPENDENCIES.json', encoded(value))
    print(json.dumps({'output': str(output), 'locked_packages': len(records), 'supplied_metadata_packages': len(selected), 'notice_files': sum(len(p['files']) for p in records) + len(standard), 'manifest_sha256': digest(encoded(value))}, indent=2))


def check_links(files, report):
    pending = ['README.md', 'THIRD_PARTY.md', 'SECURITY.md', 'docs/cli-parity.md', 'docs/deletion.md', 'docs/ui.md', report]
    if 'docs/release-packaging.md' in files:
        pending.append('docs/release-packaging.md')
    seen = set()
    while pending:
        name = pending.pop()
        if name in seen:
            continue
        seen.add(name)
        if name not in files:
            fail(f'missing entry document/report: {name}')
        for target in re.findall(r'!?\[[^\]]*\]\(([^)]+)\)', files[name][0].decode('utf-8')):
            target = target.strip('<>').split(' "', 1)[0]
            parsed = urlsplit(target)
            if parsed.scheme or not parsed.path:
                continue
            destination = os.path.normpath(str(PurePosixPath(name).parent / unquote(parsed.path)))
            relative(destination)
            if destination not in files and not any(p.startswith(destination + '/') for p in files):
                fail(f'broken bundled documentation link: {name} -> {target}')
            if destination in files and destination.endswith('.md'):
                pending.append(destination)
    return sorted(seen)


def archive(path, root, files, timestamp):
    with path.open('xb') as raw, gzip.GzipFile(filename='', fileobj=raw, mode='wb', mtime=0) as zipped, tarfile.open(fileobj=zipped, mode='w', format=tarfile.PAX_FORMAT) as output:
        for name, (data, mode, _) in sorted(files.items()):
            entry = tarfile.TarInfo(root + '/' + name)
            entry.size, entry.mode, entry.mtime = len(data), mode, timestamp
            entry.uid = entry.gid = 0
            entry.uname = entry.gname = ''
            output.addfile(entry, io.BytesIO(data))


def verify_archive(path):
    actual = {}
    manifest = None
    root = None
    total = 0
    with tarfile.open(path, 'r:gz') as archive_file:
        for member in archive_file:
            parts = relative(member.name).parts
            if len(parts) < 2 or not member.isfile() or member.mode not in (0o644, 0o755) or member.size > LIMIT:
                fail('archive contains special entry, missing root or oversized file')
            root = root or parts[0]
            if parts[0] != root:
                fail('archive has multiple roots')
            name = '/'.join(parts[1:])
            if name in actual:
                fail('duplicate archive entry')
            data = archive_file.extractfile(member).read(LIMIT + 1)
            total += len(data)
            if total > 512 * 1024 * 1024 or len(data) != member.size:
                fail('archive payload exceeds size budget or is truncated')
            actual[name] = {'sha256': digest(data), 'bytes': len(data), 'mode': member.mode}
            if name == 'PACKAGE-MANIFEST.json':
                manifest = json.loads(data)
    if not manifest or actual.pop('PACKAGE-MANIFEST.json', None) is None or actual != manifest['files']:
        fail('archive differs from its complete payload manifest')
    return {'archive': path.name, 'sha256': digest(path.read_bytes()), 'bytes': path.stat().st_size, 'files_verified': len(actual), 'integrity': 'passed', 'consumer_validation': 'pending'}


def package(args):
    if not args.preview and not args.freeze:
        fail('final artifacts require an explicit final freeze record')
    if args.preview and not args.output.resolve().is_relative_to(Path('/tmp').resolve()):
        fail('development previews must remain under /tmp')
    commit, tree, stamp, source, excluded = git_source(args.repo, args.source)
    if source.get('tools/package_release.py', (b'',))[0] != Path(__file__).read_bytes():
        fail('run the helper version committed in the selected source tree')
    cargo = tomllib.loads(source['Cargo.toml'][0].decode())['package']
    inventory = json.loads(source['licenses/DEPENDENCIES.json'][0])
    locked = {(p['name'], p['version'], p['source'], p['checksum']) for p in tomllib.loads(source['Cargo.lock'][0].decode())['package'] if p.get('source')}
    covered = {(p['name'], p['version'], p['source'], p['package_checksum']) for p in inventory.get('packages', [])} if isinstance(inventory, dict) else set()
    if locked != covered:
        fail('committed inventory does not cover every locked dependency')
    if inventory.get('format') != 'rdu-license-inventory-v1' or inventory['cargo_lock_sha256'] != digest(source['Cargo.lock'][0]) or inventory['cargo_toml_sha256'] != digest(source['Cargo.toml'][0]):
        fail('commit regenerated notices matching the final Cargo files before packaging')
    metadata_bytes = safe_read(args.metadata.parent.resolve(), args.metadata.name)
    if inventory['graph_sha256'] != digest(encoded(graph(json.loads(metadata_bytes)))):
        fail('supplied dependency graph differs from committed notice inventory')
    for notice in [f for p in inventory['packages'] for f in p['files']] + inventory['standard_library']['files']:
        if digest(source['licenses/' + notice['path']][0]) != notice['sha256']:
            fail('committed notice differs from inventory')
    binary = safe_read(args.binary.parent.resolve(), args.binary.name)
    if digest(binary) != args.expected_sha256 or len(binary) < 32 or struct.unpack('<IIII', binary[:16])[:2] != (0xfeedfacf, 0x01000007) or struct.unpack('<I', binary[12:16])[0] != 2:
        fail('binary hash or Intel macOS executable header differs from requested freeze')
    binary_source = command('git', 'rev-parse', '--verify', args.binary_source + '^{commit}', cwd=args.repo).decode().strip()
    state = 'development-preview'
    freeze = None
    if not args.preview:
        if not args.freeze:
            fail('final artifacts require an explicit final freeze record')
        freeze = json.loads(safe_read(args.freeze.parent.resolve(), args.freeze.name))
        expected = {'cargo_version': inventory['standard_library']['cargo_version'], 'release_status': 'final', 'source_commit': commit, 'source_tree': tree, 'binary_source_commit': commit, 'binary_sha256': digest(binary), 'binary_bytes': len(binary), 'target': 'x86_64-apple-darwin', 'cargo_lock_sha256': digest(source['Cargo.lock'][0]), 'cargo_metadata_sha256': digest(metadata_bytes), 'rustc_vv': inventory['standard_library']['rustc_vv']}
        if binary_source != commit or any(freeze.get(k) != v for k, v in expected.items()) or not isinstance(freeze.get('build_command'), list) or not freeze['build_command'] or not all(isinstance(v, str) for v in freeze['build_command']):
            fail('final freeze does not bind this exact source, binary, graph and toolchain')
        state = 'final-frozen-package'
    elif args.freeze:
        fail('preview mode cannot consume a final freeze record')
    links = check_links(source, args.report)
    output = new_directory(args.output)
    version = cargo['version'] + ('-dev-' + commit[:12] if args.preview else '')
    common = {'format': 'rdu-package-v1', 'packager': {'python': sys.version.split()[0], 'zlib': zlib.ZLIB_VERSION, 'helper_sha256': digest(Path(__file__).read_bytes())}, 'project': {'name': cargo['name'], 'version': cargo['version'], 'license': cargo['license']}, 'source_payload_sha256': digest(encoded({p: {'sha256': digest(d), 'mode': m} for p, (d, m, _) in source.items()})), 'release_status': state, 'source_commit': commit, 'source_tree': tree, 'source_exclusions': excluded, 'binary_source_commit': binary_source, 'binary_sha256': digest(binary), 'binary_bytes': len(binary), 'target': 'x86_64-apple-darwin', 'cargo_lock_sha256': digest(source['Cargo.lock'][0]), 'cargo_metadata_sha256': digest(metadata_bytes), 'license_inventory_sha256': digest(source['licenses/DEPENDENCIES.json'][0]), 'toolchain': inventory['standard_library']['rustc_vv'], 'freeze': freeze, 'documentation_links_checked': links, 'consumer_validation': 'pending'}
    receipts = []
    for kind in ('source', 'macos-x86_64'):
        name = f'rdu-{version}-{kind}'
        files = dict(source)
        if kind != 'source':
            files['rdu'] = (binary, 0o755, None)
            files['RUNNING.md'] = ((f'# RDU {cargo["version"]}\n\nStatus: {state}.\n\nRun `./rdu --read-only PATH` or `./rdu --help`. See [README](README.md), [CLI](docs/cli-parity.md), [deletion policy](docs/deletion.md), and [report]({args.report}).\n\nThe snapshot/cleanup controls are documented; permanent deletion needs separate exact typed confirmation. No install is required. This folder also contains the exact committed source payload, dependency notices and source provenance.\n\nA development preview does not certify its binary was built from this source. Consumer checks and final source rebuild are pending until separately recorded.\n').encode(), 0o644, None)
        value = dict(common, kind=kind, files={p: {'sha256': digest(d), 'bytes': len(d), 'mode': m} for p, (d, m, _) in files.items()}, git_blobs={p: oid for p, (_, _, oid) in source.items()})
        files['PACKAGE-MANIFEST.json'] = (encoded(value), 0o644, None)
        folder = new_directory(output / name)
        for p, (data, mode, _) in sorted(files.items()):
            write_new(folder, p, data, mode)
        archive_path = output / (name + '.tar.gz')
        archive(archive_path, name, files, stamp)
        receipts.append(verify_archive(archive_path))
    release = dict(common, archives=receipts)
    write_new(output, 'RELEASE.json', encoded(release))
    checksums = ''.join(f'{r["sha256"]}  {r["archive"]}\n' for r in receipts)
    checksums += f'{digest(encoded(release))}  RELEASE.json\n{digest(binary)}  rdu-{version}-macos-x86_64/rdu\n'
    write_new(output, 'SHA256SUMS', checksums.encode())
    print(json.dumps({'output': str(output), 'status': state, 'archives': receipts, 'consumer_validation': 'pending'}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    notices = sub.add_parser('inventory')
    notices.add_argument('--repo', type=Path, required=True)
    notices.add_argument('--source', required=True)
    notices.add_argument('--metadata', type=Path, required=True)
    notices.add_argument('--registry', type=Path, required=True)
    notices.add_argument('--toolchain', default='stable')
    notices.add_argument('--output', type=Path, required=True)
    bundle = sub.add_parser('package')
    for flag in ('repo', 'metadata', 'binary', 'freeze', 'output'):
        bundle.add_argument('--' + flag, type=Path, required=flag != 'freeze')
    for flag in ('source', 'binary-source', 'expected-sha256', 'report'):
        bundle.add_argument('--' + flag, required=True)
    bundle.add_argument('--preview', action='store_true')
    check = sub.add_parser('verify')
    check.add_argument('archive', type=Path)
    args = parser.parse_args()
    try:
        if args.action == 'inventory':
            inventory(args)
        elif args.action == 'package':
            package(args)
        else:
            print(json.dumps(verify_archive(args.archive), indent=2))
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError, tarfile.TarError) as error:
        parser.exit(1, f'packaging error: {ascii(str(error))}\n')


if __name__ == '__main__':
    main()

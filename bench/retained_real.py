"""Bounded independent metadata model for full retained real-tree output.

Names and complete models stay in memory. Only path-free totals, identity,
coverage and metadata fingerprints may be serialized by the caller.
"""
import hashlib
import json
import os
from pathlib import Path
import stat
import time

from retained_output import parse_dua_tree, parse_rdu_tree, validate_rows

MAX_OUTPUT = 32 * 1024 * 1024
MAX_NODES = 200000
MAX_DEPTH = 511


def check_component(name):
    raw = os.fsencode(name)
    try:
        text = raw.decode('utf8', errors='strict')
    except UnicodeError as error:
        raise ValueError('Real retained UTF8 filename coverage') from error
    if not text or text in ('.', '..') or '/' in text or '\\' in text or text.startswith(' '):
        raise ValueError('Real retained basename/indentation coverage')
    if any(ord(character) < 32 or 127 <= ord(character) <= 159
           or ord(character) in (0xad, 0x61c, 0xfeff)
           or any(lo <= ord(character) <= hi for lo, hi in ((0x200b, 0x200f), (0x2028, 0x202e),
                                                          (0x2060, 0x206f), (0xfff9, 0xfffb)))
           for character in text):
        raise ValueError('Real retained control-character coverage')
    return text


def build_model(root, deadline):
    if time.monotonic() >= deadline:
        raise TimeoutError('Final retained metadata-model deadline already exhausted')
    root = Path(root)
    root_metadata = root.lstat()
    if not stat.S_ISDIR(root_metadata.st_mode):
        raise ValueError('Retained input must be a genuine directory')
    root_device = root_metadata.st_dev
    nodes = {}
    pending = [(root, '', 0)]
    totals = dict(allocated=0, apparent=0, files=0, directories=0,
                  hardlink_duplicates=0, errors=0, symlinks=0,
                  directory_allocated=0, directory_apparent=0)
    seen = set()
    max_depth = 0
    while pending:
        if time.monotonic() >= deadline:
            raise TimeoutError('Final retained metadata-model deadline')
        path, relative, depth = pending.pop()
        if depth > MAX_DEPTH or len(nodes) >= MAX_NODES:
            raise ValueError('Real retained depth/node cap')
        metadata = path.lstat()
        directory = stat.S_ISDIR(metadata.st_mode)
        regular = stat.S_ISREG(metadata.st_mode)
        symlink = stat.S_ISLNK(metadata.st_mode)
        if metadata.st_dev != root_device:
            raise ValueError('Real retained cross-device input')
        kind = 'directory' if directory else 'symlink' if symlink else 'file'
        identity = (metadata.st_dev, metadata.st_ino)
        key = identity if regular and metadata.st_nlink > 1 else None
        node = {'kind': kind, 'allocated': metadata.st_blocks * 512,
                'apparent': metadata.st_size, 'hardlink': key, 'identity': identity,
                'mode': metadata.st_mode, 'mtime_ns': metadata.st_mtime_ns}
        if relative in nodes:
            raise ValueError('Duplicate metadata model path')
        nodes[relative] = node
        max_depth = max(max_depth, depth)
        totals['directories' if directory else 'files'] += 1
        totals['symlinks'] += int(symlink)
        duplicate = key is not None and key in seen
        if key is not None:
            seen.add(key)
        if duplicate:
            totals['hardlink_duplicates'] += 1
        else:
            totals['allocated'] += node['allocated']; totals['apparent'] += node['apparent']
        if directory:
            totals['directory_allocated'] += node['allocated']; totals['directory_apparent'] += node['apparent']
            with os.scandir(path) as entries:
                for entry in entries:
                    if time.monotonic() >= deadline:
                        raise TimeoutError('Final retained directory-model deadline')
                    component = check_component(entry.name)
                    if len(nodes) + len(pending) >= MAX_NODES:
                        raise ValueError('Real retained pending node cap')
                    child = relative + '/' + component if relative else component
                    pending.append((Path(entry.path), child, depth + 1))
    after = root.lstat()
    if (after.st_dev, after.st_ino) != (root_metadata.st_dev, root_metadata.st_ino):
        raise ValueError('Real retained root identity changed')
    model = {'nodes': nodes, 'totals': totals}
    fingerprint = hashlib.sha256(json.dumps(model, sort_keys=True, ensure_ascii=True).encode()).hexdigest()
    return model, {'totals': totals, 'metadata_fingerprint': fingerprint,
                   'root_identity': [root_metadata.st_dev, root_metadata.st_ino],
                   'nodes': len(nodes), 'edges': len(nodes) - 1, 'max_depth': max_depth,
                   'max_nodes': MAX_NODES, 'max_depth_limit': MAX_DEPTH,
                   'filename_coverage': 'All UTF8, no escaping/indentation ambiguity; names retained in memory only',
                   'output_bound_bytes': MAX_OUTPUT}


def validate_sample(sample, variant, model, root, sentinel):
    sample['matches_oracle'] = False
    try:
        stdout = sample['stdout'].encode('utf8')
        if sample['stdout_bytes'] > MAX_OUTPUT or len(stdout) != sample['stdout_bytes']:
            raise ValueError('Real retained output/UTF8 byte coverage cap')
        if sample['returncode'] != 0 or sample['stderr_bytes'] or sample.get('timed_out'):
            raise ValueError('Retained process exit/error/timeout contract')
        tool = 'rdu' if variant['kind'] == 'retained-rdu' else 'dua'
        parsed = parse_rdu_tree(stdout, MAX_OUTPUT) if tool == 'rdu' else parse_dua_tree(stdout, root, sentinel, MAX_OUTPUT)
        sample['retained_validation'] = validate_rows(parsed, model, 'allocated', tool)
        sample['matches_oracle'] = True
    except (ValueError, KeyError, TypeError) as error:
        # Never retain parser text containing a real filename/path.
        sample['retained_failure'] = type(error).__name__ + ': defining process/full-node/edge/subtotal/order/coverage contract failed'
    finally:
        sample.pop('stdout', None); sample.pop('stderr', None)
        sample['output_retention'] = 'Real path text discarded after strict in-memory full retained-tree validation; original byte/line counts/hashes only'
    return sample

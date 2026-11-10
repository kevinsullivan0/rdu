"""Root-first GDU parsing and aggregate-only retention of path-bearing output."""
import hashlib
import re


def parse_gdu_root(output):
    # --depth 1 selects the full analyzer and prints root before child rows.
    # Never sum child rows, choose the largest row or scan past an error line.
    match = re.match(r'^[ \t]*(\d+)[ \t]+[^\r\n]+', output)
    return int(match[1]) if match else None


def redact_gdu_output(sample):
    for stream in ('stdout', 'stderr'):
        text = sample.pop(stream, '')
        raw = text.encode('utf-8')
        sample.setdefault(stream + '_bytes', len(raw))
        sample.setdefault(stream + '_lines', len(raw.splitlines()))
        sample.setdefault(stream + '_sha256', hashlib.sha256(raw).hexdigest())
        sample[stream + '_redacted'] = True
    sample['output_retention'] = 'root numeric total plus original-byte hashes/counts; no GDU path listings or diagnostic text retained'

#!/usr/bin/env python3
"""Check an explicitly selected local RDU binary using owned temporary fixtures."""
import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import resource
import subprocess
import tempfile

from harness import oracle


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("refusing to replace verification evidence")
    binary = args.binary.resolve(strict=True)
    result = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "binary_bytes": binary.stat().st_size, "checks": []}
    with tempfile.TemporaryDirectory(prefix="rdu-release-check-", dir="/tmp") as temporary:
        base = Path(temporary)
        root = base / "tree"
        root.mkdir()
        outside = base / "outside"
        outside.mkdir()
        (outside / "excluded").write_bytes(b"outside the scan")
        for branch in range(24):
            parent = root / f"branch-{branch}"
            parent.mkdir()
            for leaf in range(4):
                (parent / f"file-{leaf}").write_bytes(b"owned fixture" * (leaf + 1))
                nested = parent / f"sub-{leaf}"
                for _ in range(2):
                    nested.mkdir()
                    (nested / "contents").write_bytes(b"nested owned fixture")
                    nested = nested / "deeper"
        os.link(root / "branch-0/file-0", root / "hardlink")
        (root / "loop").symlink_to(root, target_is_directory=True)
        (root / "outside-link").symlink_to(outside, target_is_directory=True)
        (root / "dangling").symlink_to(base / "missing")
        (root / "evil\x1b]52;c;PWN\x07\n").touch()
        os.mkfifo(root / "fifo")
        expected = oracle(root)
        assert expected["errors"] == 0
        keys = ("allocated", "apparent", "files", "directories", "errors")

        def scan(backend, threads, soft_limit=None):
            def limits():
                current, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
                resource.setrlimit(resource.RLIMIT_NOFILE, (min(current, soft_limit), hard))
            child = subprocess.run(
                [str(binary), "--summary", "--backend", backend, "--threads", str(threads), str(root)],
                capture_output=True, timeout=30,
                preexec_fn=limits if soft_limit is not None else None)
            assert child.returncode == 0, (backend, threads, soft_limit, child.returncode)
            totals = json.loads(child.stdout)
            assert all(totals[k] == expected[k] for k in keys), (backend, threads, totals)
            result["checks"].append({"check": "metadata_oracle", "backend": backend,
                                     "threads": threads, "soft_fd_limit": soft_limit,
                                     "totals": {k: totals[k] for k in keys}, "passed": True})

        for backend in ("auto", "bulk", "portable"):
            scan(backend, 8)
        for backend in ("auto", "portable"):
            for threads, limit in ((1, 16), (8, 48), (64, 280)):
                scan(backend, threads, limit)
        for label, flags in (("help", ["--help"]), ("version", ["--version"]),
                             ("summary", ["--summary", str(root)]),
                             ("table", ["-n", str(root)])):
            for mode in ("broken_pipe", "read_only"):
                if mode == "broken_pipe":
                    reader, descriptor = os.pipe()
                    os.close(reader)
                else:
                    descriptor = os.open(root / "branch-0/file-0", os.O_RDONLY)
                try:
                    child = subprocess.run([str(binary), *flags], stdout=descriptor,
                                           stderr=subprocess.PIPE, timeout=30)
                finally:
                    os.close(descriptor)
                assert child.returncode == 1, (label, mode, child.returncode)
                expected_errno = errno.EPIPE if mode == "broken_pipe" else errno.EBADF
                assert child.stderr.startswith(b"rdu: ") and b"panicked" not in child.stderr
                assert f"os error {expected_errno}".encode() in child.stderr, (label, mode, child.stderr)
                result["checks"].append({"check": "stdout_failure", "command": label,
                                         "mode": mode, "returncode": child.returncode,
                                         "passed": True})
        partial = subprocess.run([str(binary), "--summary", "--max-entries", "2", str(root)],
                                 capture_output=True, timeout=30)
        assert partial.returncode == 2 and json.loads(partial.stdout)["errors"] > 0
        result["checks"].append({"check": "partial_result_exit", "returncode": 2, "passed": True})
        assert oracle(root) == expected
        result["fixture_stable"] = True
    result["passed"] = True
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"passed": True, "checks": len(result["checks"]),
                      "binary_sha256": result["binary_sha256"]}))


if __name__ == "__main__":
    main()

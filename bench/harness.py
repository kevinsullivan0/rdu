#!/usr/bin/env python3
"""Bounded fixtures, independent lstat oracle, and serialized warm benchmarks."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import re
import stat
import statistics
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
DATASETS = ("many-small", "wide", "deep", "mixed")


def dataset_paths(names, existing):
    paths = {name: DATA / name for name in names}
    for label, supplied in existing:
        if label in DATASETS or label in paths:
            raise ValueError(f"duplicate dataset label: {label}")
        path = Path(supplied).expanduser().resolve(strict=True)
        if not path.is_dir():
            raise ValueError(f"existing dataset is not a directory: {path}")
        paths[label] = path
    return paths


def oracle(root):
    """Count every name, but charge each hardlinked regular inode once."""
    totals = dict(allocated=0, apparent=0, files=0, directories=0,
                  hardlink_duplicates=0, errors=0, symlinks=0,
                  directory_allocated=0, directory_apparent=0)
    pending = [Path(root)]
    seen = set()
    while pending:
        path = pending.pop()
        try:
            metadata = path.lstat()
        except OSError:
            totals["errors"] += 1
            continue
        directory = stat.S_ISDIR(metadata.st_mode)
        totals["directories" if directory else "files"] += 1
        totals["symlinks"] += int(stat.S_ISLNK(metadata.st_mode))
        duplicate = False
        if stat.S_ISREG(metadata.st_mode) and metadata.st_nlink > 1:
            key = (metadata.st_dev, metadata.st_ino)
            duplicate = key in seen
            seen.add(key)
        if duplicate:
            totals["hardlink_duplicates"] += 1
        else:
            totals["allocated"] += metadata.st_blocks * 512
            totals["apparent"] += metadata.st_size
        if directory:
            totals["directory_allocated"] += metadata.st_blocks * 512
            totals["directory_apparent"] += metadata.st_size
            try:
                with os.scandir(path) as entries:
                    pending.extend(Path(entry.path) for entry in entries)
            except OSError:
                totals["errors"] += 1
    return totals


def generate():
    DATA.mkdir(exist_ok=True)
    manifest_path = DATA / "manifest.json"
    if manifest_path.exists():
        return json.loads(manifest_path.read_text())
    # Refuse to overwrite partial or user-provided trees.
    if any(DATA.iterdir()):
        raise RuntimeError("data exists without manifest; inspect it before regenerating")
    started = time.perf_counter()
    for name in DATASETS:
        (DATA / name).mkdir()
    small = DATA / "many-small"
    for group in range(200):
        directory = small / f"d{group:03}"
        directory.mkdir()
        for index in range(500):
            (directory / f"f{index:04}").write_bytes(b"x" * 23 if index % 5 == 0 else b"")
    wide = DATA / "wide"
    for index in range(20000):
        # Large single readdir; empty files bound physical data allocation.
        (wide / f"entry-{index:06}").touch()
    current = DATA / "deep"
    for depth in range(240):
        current = current / "d"
        current.mkdir()
        for index in range(4):
            (current / f"f{index}").write_bytes(b"depth")
    mixed = DATA / "mixed"
    for group in range(32):
        directory = mixed / f"d{group:02}"
        directory.mkdir()
        for index in range(8):
            (directory / f"payload-{index}").write_bytes(bytes([index + 1]) * 32768)
        original = directory / "payload-0"
        os.link(original, directory / "hard")
        os.link(original, mixed / f"hard-root-{group}")
        os.symlink("payload-0", directory / "file-link")
        os.symlink("..", directory / "dir-link")
        os.symlink("absent", directory / "dangling")
    for index in range(4):
        with (mixed / f"sparse-{index}").open("wb") as stream:
            stream.truncate(16 * 1024 * 1024)
            stream.seek(index * 4096)
            stream.write(b"sparse")
    manifest = {"schema": 1, "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "generation_seconds": time.perf_counter() - started,
                "description": {"many-small": "100000 files, 200 directories; 20% have 23 bytes",
                                "wide": "20000 empty files in one directory",
                                "deep": "240 levels; four five-byte files at each level",
                                "mixed": "32 directories, 256 32KiB files, 64 extra hardlinks, 96 symlinks, four sparse 16MiB files"},
                "oracles": {name: oracle(DATA / name) for name in DATASETS}}
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def capture(command):
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    return {"command": command, "returncode": result.returncode, "output": result.stdout.strip()}


def environment():
    info = {"platform": platform.platform(), "machine": platform.machine(),
            "python": sys.version, "logical_cpus": os.cpu_count(),
            "dataset_path": str(DATA), "cache_policy": "warm; no OS cache eviction",
            "parallel_benchmark_processes": 1,
            "relevant_environment": {key: os.environ.get(key) for key in
                                     ("GOMAXPROCS", "GOGC", "RAYON_NUM_THREADS")}}
    commands = [["uname", "-a"], ["df", "-h", str(HERE)], ["mount"]]
    if sys.platform == "darwin":
        commands.extend([["sysctl", "-n", key] for key in
                         ("machdep.cpu.brand_string", "hw.memsize", "hw.physicalcpu", "hw.logicalcpu")])
    info["observations"] = [capture(command) for command in commands]
    return info


def measure(command, *, env=None, cwd=None, timeout=None):
    # wait4 measures this one child, unlike cumulative RUSAGE_CHILDREN max RSS.
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        started = time.perf_counter()
        child = subprocess.Popen(command, stdout=stdout, stderr=stderr,
                                 env={**os.environ, "LC_ALL": "C", "BLOCK_SIZE": "1", "BLOCKSIZE": "1"} if env is None else env, cwd=cwd)
        timed_out = False
        if timeout is None:
            _, status, usage = os.wait4(child.pid, 0)
        else:
            import signal
            def alarm_handler(signum, frame):
                raise TimeoutError("owned benchmark child exceeded timeout")
            prior_handler = signal.signal(signal.SIGALRM, alarm_handler)
            prior_timer = signal.setitimer(signal.ITIMER_REAL, timeout)
            try:
                _, status, usage = os.wait4(child.pid, 0)
            except TimeoutError:
                timed_out = True
                try:
                    child.kill()
                except ProcessLookupError:
                    pass
                _, status, usage = os.wait4(child.pid, 0)
            finally:
                signal.setitimer(signal.ITIMER_REAL, 0)
                signal.signal(signal.SIGALRM, prior_handler)
                if prior_timer[0]:
                    signal.setitimer(signal.ITIMER_REAL, *prior_timer)
        elapsed = time.perf_counter() - started
        child.returncode = os.waitstatus_to_exitcode(status)
        stdout.seek(0)
        stderr.seek(0)
        stdout_bytes = stdout.read()
        stderr_bytes = stderr.read()
        return {"command": command, "returncode": child.returncode, "wall_seconds": elapsed,
                "peak_rss_bytes": usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
                "user_seconds": usage.ru_utime, "system_seconds": usage.ru_stime,
                "minor_page_faults": usage.ru_minflt, "major_page_faults": usage.ru_majflt,
                "block_input_operations": usage.ru_inblock, "block_output_operations": usage.ru_oublock,
                "voluntary_context_switches": usage.ru_nvcsw,
                "involuntary_context_switches": usage.ru_nivcsw,
                "stdout_bytes": len(stdout_bytes), "stdout_lines": len(stdout_bytes.splitlines()),
                "stdout_sha256": hashlib.sha256(stdout_bytes).hexdigest(),
                "stderr_bytes": len(stderr_bytes), "stderr_lines": len(stderr_bytes.splitlines()),
                "stderr_sha256": hashlib.sha256(stderr_bytes).hexdigest(),
                "stdout": stdout_bytes.decode(errors="replace"),
                "stderr": stderr_bytes.decode(errors="replace"),
                **({"timed_out": timed_out} if timeout is not None else {})}


def binary_info(path, version_args):
    path = Path(path).resolve()
    return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "version": capture([str(path), *version_args])}


def run(args):
    manifest = generate()
    paths = dataset_paths(args.datasets or list(DATASETS), args.existing)
    names = list(paths)
    expected = {name: oracle(path) for name, path in paths.items()}
    variants = []
    binaries = []
    config = str(HERE / "empty-gdu.yaml")
    for label, path, use_default in [(label, path, False) for label, path in args.gdu] + [(label, path, True) for label, path in args.gdu_default]:
        binaries.append(binary_info(path, ["--version"]))
        command = [str(Path(path).resolve()), "-nps", "--no-prefix", "--config-file", config]
        if args.gdu_cores and not use_default:
            command.extend(["--max-cores", str(args.gdu_cores)])
        variants.append((label, "gdu", command))
    if args.rdu:
        binaries.append(binary_info(args.rdu, ["--version"]))
        for backend in args.backends:
            for threads in args.threads:
                label = f"rdu-{backend}-{threads}t"
                variants.append((label, "rdu", [str(Path(args.rdu).resolve()), "--summary", "--backend", backend, "--threads", str(threads)]))
    if not variants:
        raise RuntimeError("provide --gdu LABEL PATH and/or --rdu PATH")
    cases = [(name, mode, label, kind, command) for name in names
             for mode in args.modes for label, kind, command in variants]
    result = {"schema": 1, "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "dataset_paths": {name: str(path) for name, path in paths.items()},
              "existing_datasets_policy": "metadata-only oracle and scans; no generation, modification or content collection in existing trees",
              "environment": environment(), "manifest": manifest, "oracles": expected,
              "binaries": binaries, "seed": args.seed, "runs_per_case": args.runs,
              "warmups_per_case": args.warmups, "samples": [], "summaries": []}
    rng = random.Random(args.seed)
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    for phase, rounds in (("warmup", args.warmups), ("measured", args.runs)):
        for iteration in range(rounds):
            order = cases.copy()
            rng.shuffle(order)
            for name, mode, label, kind, base in order:
                command = base.copy()
                if mode == "apparent":
                    command.append("-a" if kind == "gdu" else "--apparent-size")
                command.append(str(paths[name]))
                sample = measure(command)
                sample.update(dataset=name, mode=mode, variant=label, phase=phase, iteration=iteration)
                if sample["returncode"] != 0:
                    result["samples"].append(sample)
                    output.write_text(json.dumps(result, indent=2) + "\n")
                    raise RuntimeError(f"{label} exited {sample['returncode']}: {sample['stderr']}")
                if kind == "rdu":
                    totals = json.loads(sample["stdout"])
                    sample["totals"] = totals
                    sample["matches_oracle"] = all(totals[key] == expected[name][key] for key in
                                                   ("allocated", "apparent", "files", "directories", "errors"))
                    sample["reported_bytes"] = totals[mode]
                    sample["normalized_reported_bytes"] = totals[mode]
                    sample["expected_bytes"] = expected[name][mode]
                else:
                    match = re.match(r"^\s*(\d+)\s", sample["stdout"])
                    if not match:
                        raise RuntimeError(f"cannot parse GDU summary: {sample['stdout']!r}")
                    sample["reported_bytes"] = int(match[1])
                    # Current upstream GDU apparent size omits directory-own st_size.
                    # Preserve raw values; normalize only by independently observed metadata.
                    directory_bytes = expected[name]["directory_apparent"] if mode == "apparent" else 0
                    sample["expected_bytes"] = expected[name][mode] - directory_bytes
                    sample["normalized_reported_bytes"] = sample["reported_bytes"] + directory_bytes
                    sample["matches_oracle"] = sample["reported_bytes"] == sample["expected_bytes"]
                result["samples"].append(sample)
                if kind == "rdu" and not sample["matches_oracle"]:
                    output.write_text(json.dumps(result, indent=2) + "\n")
                    raise RuntimeError(f"{label} disagrees with oracle on {name}; see {output}")
            output.write_text(json.dumps(result, indent=2) + "\n")
    for name, mode, label, _, _ in cases:
        samples = [s for s in result["samples"] if s["dataset"] == name and s["mode"] == mode
                   and s["variant"] == label and s["phase"] == "measured"]
        times = [s["wall_seconds"] for s in samples]
        result["summaries"].append({"dataset": name, "mode": mode, "variant": label,
                                    "n": len(times), "median_seconds": statistics.median(times),
                                    "min_seconds": min(times), "max_seconds": max(times),
                                    "pstdev_seconds": statistics.pstdev(times),
                                    "median_peak_rss_bytes": statistics.median(s["peak_rss_bytes"] for s in samples),
                                    "max_peak_rss_bytes": max(s["peak_rss_bytes"] for s in samples),
                                    "matches_oracle": all(s["matches_oracle"] for s in samples)})
    output.write_text(json.dumps(result, indent=2) + "\n")
    for summary in result["summaries"]:
        print(f"{summary['dataset']:12} {summary['mode']:9} {summary['variant']:24} "
              f"median {summary['median_seconds']:.6f}s RSS {summary['median_peak_rss_bytes']/1048576:.1f}MiB "
              f"correct={summary['matches_oracle']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)
    subparsers.add_parser("generate")
    scan = subparsers.add_parser("oracle")
    scan.add_argument("path", type=Path)
    benchmark = subparsers.add_parser("run")
    benchmark.add_argument("--gdu", nargs=2, action="append", default=[], metavar=("LABEL", "PATH"))
    benchmark.add_argument("--gdu-default", nargs=2, action="append", default=[], metavar=("LABEL", "PATH"),
                           help="additional GDU variant that preserves upstream CPU defaults regardless of --gdu-cores")
    benchmark.add_argument("--gdu-cores", type=int, help="optional explicit GDU CPU cap; otherwise upstream default")
    benchmark.add_argument("--rdu")
    benchmark.add_argument("--backends", nargs="+", default=["auto"], choices=["auto", "portable", "bulk"])
    benchmark.add_argument("--threads", nargs="+", type=int, default=[8])
    benchmark.add_argument("--datasets", nargs="+", choices=DATASETS)
    benchmark.add_argument("--existing", nargs=2, action="append", default=[], metavar=("LABEL", "PATH"),
                           help="also scan an existing directory using metadata only; never generate or modify data there")
    benchmark.add_argument("--modes", nargs="+", default=["allocated", "apparent"], choices=["allocated", "apparent"])
    benchmark.add_argument("--runs", type=int, default=7)
    benchmark.add_argument("--warmups", type=int, default=1)
    benchmark.add_argument("--seed", type=int, default=20261109)
    benchmark.add_argument("--output", default=str(HERE / "results" / "comparison.json"))
    args = parser.parse_args()
    if args.action == "generate":
        print(json.dumps(generate(), indent=2))
    elif args.action == "oracle":
        print(json.dumps(oracle(args.path), indent=2))
    else:
        if args.runs < 7 or args.warmups < 1 or any(n < 1 for n in args.threads) or (args.gdu_cores is not None and args.gdu_cores < 1):
            parser.error("use >=7 measured runs, >=1 warmup, and positive threads")
        run(args)


if __name__ == "__main__":
    main()

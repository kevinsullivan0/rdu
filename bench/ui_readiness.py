#!/usr/bin/env python3
"""Measure process launch to a validated, complete RDU tree frame in a real PTY.

Uses only q after readiness. Retains scalar timing and aggregate metadata, never
terminal transcripts or filename inventories. Acquire the parent's slot first.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import random
import re
import select
import statistics
import struct
import subprocess
import termios
import time

FRAME_START = b"\x1b[H"
FRAME_END = b"\x1b[0m\x1b[J"
ANSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
KEYS = ("allocated", "apparent", "files", "directories", "errors")


def human_size(size):
    if size < 1024:
        return f"{size} B"
    value, unit = float(size), 0
    units = ("B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB")
    while value >= 1024 and unit < len(units) - 1:
        value /= 1024
        unit += 1
    return f"{value:.1f} {units[unit]}"


def completed_rdu_frame(data, expected, metric="allocated"):
    start = data.find(FRAME_START)
    if start < 0:
        return None
    end = data.find(FRAME_END, start)
    if end < 0:
        return None
    lines = ANSI.sub(b"", data[start:end]).decode("utf-8", "strict").split("\r\n")
    if len(lines) < 6:
        raise ValueError("completed frame is missing browser rows")
    header = f"rdu  {metric}  sort:size  total {human_size(expected[metric])}"
    if lines[0] != header:
        raise ValueError("completed frame total or metric differs from independent oracle")
    status = re.fullmatch(r"\d+ of \d+ entries \| (\d+) files, (\d+) dirs \| errors: (\d+) \(here \d+\) \| [\d.]+s", lines[-3])
    if status is None or tuple(map(int, status.groups())) != tuple(expected[key] for key in ("files", "directories", "errors")):
        raise ValueError("completed frame file, directory, or error counts differ from independent oracle")
    if not lines[1].startswith("Path: ") or not lines[-2].startswith("Filter: ") or not lines[-1].startswith("j/k select  Enter open"):
        raise ValueError("completed frame is missing the full browser layout")
    return {"metric": metric, "validated": True}


def completed_gdu_frame(screen, expected, root):
    """Accept only a complete tcell synchronized draw of the expected root."""
    footer_pattern = re.compile(r"\s*Total disk usage: (\d+(?:\.\d+)? (?:[KMGTPE]iB|B)) Apparent size: (\d+(?:\.\d+)? (?:[KMGTPE]iB|B)) Items: (\d+) Sorting by: size desc\s*")
    modal_pattern = re.compile(r"(?:\b(?:scanning|finalizing|stopping scan)(?:\.{3}|…)|calculating disk usage\.{3}|preview, Tab to resume)", re.IGNORECASE)
    while screen.frames:
        draw, lines = screen.frames.popleft()
        if len(lines) < 3 or not lines[0].strip().startswith("gdu "):
            continue
        if lines[1].strip() != f"--- {root} ---":
            continue
        if any(modal_pattern.search(line) for line in lines):
            continue
        footer = footer_pattern.fullmatch(lines[-1])
        if footer is None:
            continue
        expected_items = expected["files"] + expected["directories"] - 1
        allocated = expected["allocated"] - expected.get("root_allocated", 0)
        if footer[1] != human_size(allocated) or int(footer[3]) != expected_items:
            raise ValueError("completed GDU footer allocated total or item count differs from independent oracle")
        return {"metric": "allocated", "validated": True, "items": expected_items,
                "allocated_display_validated": True, "apparent_field_present": True,
                "apparent_display_validated": False, "completed_draws": draw,
                "screen_sha256": hashlib.sha256("\n".join(lines).encode()).hexdigest()}
    return None


def terminal_modes(attributes):
    normalized = list(attributes)
    # Darwin synthesizes transient PENDIN status upon re-entering canonical mode.
    normalized[3] &= ~getattr(termios, "PENDIN", 0)
    return normalized


def pty_environment():
    environment = os.environ.copy()
    for key in ("BLOCK_SIZE", "BLOCKSIZE", "LINES", "COLUMNS"):
        environment.pop(key, None)
    environment.update(TERM="xterm-256color", LC_ALL="C")
    return environment


def owned_controlling_terminal():
    # tcell opens /dev/tty, so merely redirecting fd 0 is insufficient. This
    # creates a new session whose controlling terminal is this owned PTY only.
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    # Do not inherit an unrelated foreground group from openpty's initial state.
    # Reads by a background group can stop on SIGTTIN even after a successful
    # screen paint. Both measured tools receive this same owned foreground PTY.
    os.tcsetpgrp(0, os.getpgrp())


def termios_record(attributes):
    return {"iflag":attributes[0], "oflag":attributes[1], "cflag":attributes[2],
            "lflag":attributes[3], "ispeed":attributes[4], "ospeed":attributes[5],
            "cc_hex":[bytes([value]).hex() if isinstance(value,int) else value.hex() for value in attributes[6]]}


class ReadinessFailure(ValueError):
    def __init__(self, message, diagnostics):
        super().__init__(message)
        self.diagnostics = diagnostics
    def __str__(self):
        return self.args[0] + "; aggregate diagnostics=" + json.dumps(self.diagnostics,sort_keys=True)


def child_exit_drained(returncode, readable, chunk):
    return returncode is not None and (not readable or chunk == b"")


def ready_once(command, expected, metric="allocated", timeout=30, record_time=True,
               kind="rdu", root=None, diagnostic_transcript=None, diagnostic_checkpoint=None,
               controlling_terminal=None):
    from vt_screen import VTScreen
    if kind not in {"rdu", "gdu"}:
        raise ValueError("unsupported readiness tool")
    if kind == "gdu" and root is None:
        raise ValueError("GDU readiness requires the expected root")
    if controlling_terminal is None:
        controlling_terminal = kind == "gdu"
    master, slave = pty.openpty()
    before = terminal_modes(termios.tcgetattr(slave))
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 160, 0, 0))
    process = None
    transcript = bytearray()
    screen = VTScreen()
    stream_hash = hashlib.sha256()
    started = time.perf_counter() if record_time else None
    deadline = time.monotonic() + timeout
    elapsed = None
    validation = None
    failure = None
    events = []
    snapshots = {}
    def capture(label):
        states = {}
        for name,descriptor in [("slave",slave),("master",master)]:
            try:
                states[name] = termios_record(termios.tcgetattr(descriptor))
            except (OSError,termios.error) as error:
                states[name] = {"unavailable_errno":error.args[0]}
        snapshots[label] = states
        return states
    def checkpoint():
        if diagnostic_checkpoint is not None:
            try:
                Path(diagnostic_checkpoint).write_text(json.dumps({"tool":kind,"events":events,"termios":snapshots,
                    "complete_frame_validated":validation is not None,"primary_error":None if failure is None else failure.args[0]},indent=2)+"\n")
            except OSError:
                pass  # Diagnostic persistence must never replace the primary error.
    capture("before")
    def event(name):
        observation = {"order":len(events), "event":name, "pty_bytes_received":len(transcript),
                       "utf8_decoded_bytes":screen.bytes_received-len(screen.decoder.getstate()[0]),
                       "characters_consumed":screen.characters_consumed, "parser_state":screen.state}
        if process is not None:
            try:
                observation["child_is_foreground"] = os.tcgetpgrp(slave) == os.getpgid(process.pid)
                observation["child_is_session_leader"] = os.getsid(process.pid) == process.pid
            except OSError as error:
                observation["foreground_or_session_query_errno"] = error.errno
        events.append(observation)
        checkpoint()
    def child_poll():
        if process.returncode is not None:
            return process.returncode
        pid,status = os.waitpid(process.pid, os.WNOHANG | os.WUNTRACED)
        if pid and os.WIFSTOPPED(status):
            event("child-stopped")
            events[-1]["stop_signal"] = os.WSTOPSIG(status)
            raise ValueError("owned PTY child stopped on signal " + str(os.WSTOPSIG(status)))
        if pid:
            process.returncode = os.waitstatus_to_exitcode(status)
        return process.returncode
    event("pty-created")
    try:
        process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                                   env=pty_environment(), preexec_fn=owned_controlling_terminal if controlling_terminal else None)
        capture("after_spawn")
        event("child-spawned")
        while time.monotonic() < deadline:
            if select.select([master], [], [], min(0.05, max(0, deadline - time.monotonic())))[0]:
                chunk = os.read(master, 128 * 1024)
                arrived = time.perf_counter() if record_time else None
                transcript.extend(chunk)
                stream_hash.update(chunk)
                if len(transcript) > 2 * 1024 * 1024:
                    raise ValueError("readiness output exceeded the bounded capture limit")
                screen.feed(chunk)
                if len(events) == 2:
                    event("first-output")
                validation = (completed_rdu_frame(transcript, expected, metric) if kind == "rdu"
                              else completed_gdu_frame(screen, expected, str(root)))
                if validation:
                    elapsed = arrived - started if record_time else None
                    capture("at_ready")
                    event("validated-ready")
                    if termios.tcgetattr(slave)[3] & (termios.ICANON | termios.ECHO):
                        raise ValueError("validated screen did not leave canonical input and echo modes")
                    break
            if child_poll() is not None:
                raise ValueError("process exited before a validated complete tree frame")
        else:
            raise ValueError("timed out before a validated complete tree frame")
        os.write(master, b"q")
        event("single-q-written")
        exit_deadline = time.monotonic() + 5
        eof = False
        while True:
            readable = select.select([master], [], [], 0.05)[0]
            if readable:
                try:
                    chunk = os.read(master, 128 * 1024)
                except OSError as error:
                    if error.errno != 5:
                        raise
                    chunk = b""  # Unix PTY master EOF can also be reported as EIO.
                if not chunk:
                    eof = True
                    event("master-eof")
                transcript.extend(chunk)
                stream_hash.update(chunk)
                screen.feed(chunk)
                if len(transcript) > 4 * 1024 * 1024:
                    raise ValueError("terminal exit output exceeded the bounded capture limit")
            if child_exit_drained(child_poll(), readable, b"" if eof else chunk if readable else None):
                event("child-exited")
                break
            if time.monotonic() >= exit_deadline:
                raise ValueError("browser did not exit after its single q key")
        after_exit = capture("after_exit")
        if process.returncode != 0:
            raise ValueError(f"browser exit code {process.returncode}")
        if "unavailable_errno" in after_exit["slave"]:
            restored = None
            mode_observation = "unavailable_after_exit"
        else:
            restored = terminal_modes(termios.tcgetattr(slave)) == before
            mode_observation = "verified"
            if not restored:
                raise ValueError("browser did not restore terminal modes")
        if not screen.cursor_visible or not screen.saw_alt_enter or not screen.saw_alt_leave or screen.alt_screen:
            raise ValueError("browser did not restore cursor and alternate screen")
        return {"ready_seconds": elapsed, "complete_frame_validated": True,
                "terminal_restored": restored, "terminal_mode_observation":mode_observation,
                "cursor_and_screen_restored": True, "endpoint_valid":True, "returncode": process.returncode,
                "tool": kind, "validation": validation, "pty_bytes_received": len(transcript),
                "pty_stream_sha256": stream_hash.hexdigest(), "synchronized_draws": screen.draw_count,
                "events":events, "termios":snapshots}
    except Exception as error:
        # Never include terminal text or filenames in retained diagnostics.
        diagnostics = {"tool": kind, "pty_bytes_received": len(transcript),
                       "pty_stream_sha256": stream_hash.hexdigest(),
                       "synchronized_draws": screen.draw_count,
                       "alt_screen_entered": screen.saw_alt_enter,
                       "parser_state": screen.state, "complete_frame_validated": validation is not None,
                       "events":events, "termios":snapshots}
        failure = ReadinessFailure(str(error), diagnostics)
        checkpoint()
        raise failure from error
    finally:
        capture("before_cleanup")
        if diagnostic_transcript is not None:
            try:
                # Explicit owned-fixture authorization only; never set by CLI.
                Path(diagnostic_transcript).write_bytes(transcript)
            except OSError:
                pass
        try:
            if process is not None and process.poll() is None:
                process.kill()
                process.wait()
                event("owned-child-killed-after-failure")
        except OSError as cleanup_error:
            if failure is not None:
                failure.diagnostics["cleanup_errno"] = cleanup_error.errno
        capture("after_cleanup")
        if failure is not None and process is not None:
            failure.diagnostics["cleanup_returncode"] = process.returncode
        checkpoint()
        for descriptor in (master,slave):
            try:
                os.close(descriptor)
            except OSError:
                pass


def validate_summary(command, expected):
    completed = subprocess.run([command[0], "--summary", *command[1:]],
                               capture_output=True, timeout=30, check=False)
    if completed.returncode != 0:
        raise ValueError("exact --summary check failed")
    parsed = json.loads(completed.stdout)
    totals = parsed.get("totals", parsed)
    if any(totals[key] != expected[key] for key in KEYS):
        raise ValueError("exact --summary totals differ from independent oracle")
    return {key: totals[key] for key in KEYS}


def readiness_sample_eligible(sample, endpoint_contract="ui01_terminal_restoration"):
    if not sample.get("complete_frame_validated") or sample.get("returncode") != 0:
        return False
    if endpoint_contract == "ui01_terminal_restoration":
        return sample.get("terminal_restored") is True
    if endpoint_contract == "ready_frame_and_single_q_exit":
        mode_observation = sample.get("terminal_mode_observation")
        observed = mode_observation == "verified" and sample.get("terminal_restored") is True
        explicitly_unavailable = mode_observation == "unavailable_after_exit" and sample.get("terminal_restored") is None
        return bool(sample.get("endpoint_valid") and sample.get("cursor_and_screen_restored")
                    and (observed or explicitly_unavailable))
    return False


def summarize_ready_samples(result, datasets, variants, baseline, seed, endpoint_contract="ui01_terminal_restoration"):
    from paired_stats import paired_effect
    measured = [sample for sample in result["samples"] if sample["phase"] == "measured"]
    permitted = (result.get("valid", False) and result.get("datasets_stable", False)
                 and result.get("binaries_unchanged", False)
                 and all(readiness_sample_eligible(sample, endpoint_contract) for sample in measured))
    result["endpoint_contract"] = endpoint_contract
    result["inference_permitted"] = bool(permitted)
    if not permitted:
        result["inference_suppressed_reason"] = "changed dataset/binary or failed readiness validation; descriptive samples retained"
    for name in datasets:
        samples = [sample for sample in measured if sample["dataset"] == name]
        baseline_values = {sample["iteration"]: sample["ready_seconds"] for sample in samples if sample["variant"] == baseline}
        for variant in variants:
            values = {sample["iteration"]: sample["ready_seconds"] for sample in samples if sample["variant"] == variant["label"]}
            if not values:
                continue
            result["summaries"].append({"dataset": name, "variant": variant["label"], "n": len(values),
                "median_seconds": statistics.median(values.values()), "min_seconds": min(values.values()),
                "max_seconds": max(values.values()), "pstdev_seconds": statistics.pstdev(values.values())})
            if permitted and variant["label"] != baseline:
                if values.keys() != baseline_values.keys():
                    result["inference_permitted"] = False
                    result["paired_effects"].clear()
                    result["inference_suppressed_reason"] = "incomplete paired rounds; descriptive samples retained"
                    permitted = False
                    continue
                effect = paired_effect([baseline_values[i] for i in sorted(values)], [values[i] for i in sorted(values)], seed=seed)
                result["paired_effects"].append({"dataset": name, "variant": variant["label"], **effect})


def main():
    import sys
    bench = Path(__file__).resolve().parent
    if not (bench / "harness.py").is_file():
        bench = bench.parent
    sys.path.insert(0, str(bench))
    from harness import oracle
    from paired_stats import paired_effect
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--slot", required=True)
    parser.add_argument("--lock-file", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=15)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--seed", type=int, default=20261109)
    parser.add_argument("--check-only", action="store_true", help="functional probe; do not record readiness times")
    args = parser.parse_args()
    if args.rounds < 15 or args.warmups < 1:
        parser.error("use at least 15 rounds and one warmup")
    if args.output.exists():
        parser.error("refusing to overwrite an existing run bundle")
    config = json.loads(args.config.read_text())
    variants = config["variants"]
    endpoint_contract = config.get("endpoint_contract", "ui01_terminal_restoration")
    if endpoint_contract not in {"ui01_terminal_restoration", "ready_frame_and_single_q_exit"}:
        parser.error("unknown readiness eligibility contract")
    if any(variant.get("kind") == "gdu" for variant in variants) and endpoint_contract != "ready_frame_and_single_q_exit":
        parser.error("GDU comparison requires the explicit ready-frame-and-single-q endpoint contract")
    if any(variant.get("kind", "rdu") not in {"rdu", "gdu"} for variant in variants):
        parser.error("only RDU and source-validated GDU variants are accepted")
    if not args.check_only and any(variant.get("kind") == "gdu" for variant in variants) and not config.get("gdu_detector_independently_validated"):
        parser.error("paired GDU timings require an independent validation checkpoint")
    labels = [v["label"] for v in variants]
    if len(labels) != len(set(labels)) or config["baseline"] not in labels:
        parser.error("variant labels must be unique and baseline must exist")
    paths = {name: Path(path).expanduser().resolve(strict=True) for name, path in config["datasets"].items()}
    with args.lock_file.open("a+") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            parser.error("another benchmark or profile holds the measurement lock")
        expected = {name: {**oracle(path), "root_allocated": path.lstat().st_blocks * 512} for name, path in paths.items()}
        if any(variant.get("kind") == "gdu" for variant in variants) and any(totals["root_allocated"] for totals in expected.values()):
            parser.error("this GDU comparison requires zero own allocated blocks at every root")
        if args.check_only and len(paths) * len(variants) > 2:
            parser.error("initial functional validation is bounded to two PTY smoke checks")
        if any(totals["errors"] for totals in expected.values()):
            parser.error("readiness benchmark requires complete independent metadata oracles")
        frozen = {v["label"]: {"command": v["command"], "binary_sha256": hashlib.sha256(Path(v["command"][0]).read_bytes()).hexdigest()}
                  for v in variants}
        result = {"schema": 1, "slot": args.slot, "question": config.get("question"),
                  "endpoint": "process launch to received complete first tree frame, validated against independent oracle",
                  "cache_policy": "warm; no cache eviction", "pty": {"rows": 40, "columns": 160},
                  "baseline": config["baseline"], "endpoint_contract":endpoint_contract, "variants": frozen, "oracles_before": expected,
                  "seed": args.seed, "samples": [], "summaries": [], "paired_effects": [],
                  "check_only": args.check_only,
                  "pty_environment": {"TERM": "xterm-256color", "LC_ALL": "C", "BLOCK_SIZE": None, "BLOCKSIZE": None},
                  "gdu_apparent_display": "field syntax only; allocated and Items independently validated",
                  "gdu_detector_independently_validated": config.get("gdu_detector_independently_validated", False),
                  "gdu_correctness_evidence": config.get("gdu_correctness_evidence")}
        def command(v, name):
            return [token.replace("{path}", str(paths[name])) for token in v["command"]]
        args.output.parent.mkdir(parents=True, exist_ok=True)
        def save():
            temporary = args.output.with_suffix(args.output.suffix + ".partial")
            temporary.write_text(json.dumps(result, indent=2) + "\n")
            temporary.replace(args.output)
        result.update(valid=False, status="running")
        save()
        try:
            result["summary_checks_before"] = {name: {v["label"]: validate_summary(command(v, name), expected[name]) for v in variants if v.get("kind", "rdu") == "rdu"} for name in paths}
        except Exception as error:
            result.update(status="failed-before-timing", error=str(error))
            save()
            raise
        rng = random.Random(args.seed)
        phases = [("check", 1)] if args.check_only else [("warmup", args.warmups), ("measured", args.rounds)]
        for phase, rounds in phases:
            for iteration in range(rounds):
                names = list(paths)
                rng.shuffle(names)
                for name in names:
                    order = variants.copy()
                    rng.shuffle(order)
                    for variant in order:
                        identity = dict(dataset=name, variant=variant["label"], phase=phase, iteration=iteration)
                        try:
                            sample = ready_once(command(variant, name), expected[name], variant.get("metric", "allocated"), record_time=not args.check_only, kind=variant.get("kind", "rdu"), root=paths[name], controlling_terminal=endpoint_contract == "ready_frame_and_single_q_exit")
                        except Exception as error:
                            result["samples"].append({**identity, "error": str(error), "complete_frame_validated": False})
                            result.update(status="failed", error=str(error))
                            save()
                            raise
                        sample.update(identity)
                        result["samples"].append(sample)
                        save()
        result["oracles_after"] = {name: {**oracle(path), "root_allocated": path.lstat().st_blocks * 512} for name, path in paths.items()}
        result["datasets_stable"] = expected == result["oracles_after"]
        result["binaries_unchanged"] = all(hashlib.sha256(Path(v["command"][0]).read_bytes()).hexdigest() == frozen[v["label"]]["binary_sha256"] for v in variants)
        try:
            result["summary_checks_after"] = {name: {v["label"]: validate_summary(command(v, name), expected[name]) for v in variants if v.get("kind", "rdu") == "rdu"} for name in paths}
        except Exception as error:
            result.update(status="failed-after-timing", error=str(error))
            save()
            raise
        result["valid"] = result["datasets_stable"] and result["binaries_unchanged"]
        if not args.check_only:
            summarize_ready_samples(result, paths, variants, config["baseline"], args.seed, endpoint_contract)
        result["status"] = "complete" if result["valid"] else "invalid"
        save()
        print(json.dumps({"output": str(args.output), "valid": result["valid"], "samples": len(result["samples"]), "check_only": args.check_only,
                  "pty_environment": {"TERM": "xterm-256color", "LC_ALL": "C", "BLOCK_SIZE": None, "BLOCKSIZE": None},
                  "gdu_apparent_display": "field syntax only; allocated and Items independently validated",
                  "gdu_detector_independently_validated": config.get("gdu_detector_independently_validated", False),
                  "gdu_correctness_evidence": config.get("gdu_correctness_evidence")}))
        if not result["valid"]:
            raise SystemExit("dataset or binary changed; comparison invalid")


if __name__ == "__main__":
    main()

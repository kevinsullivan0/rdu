#!/usr/bin/env python3
"""Real Ratatui acceptance in disposable fixtures, including explicit deletion.

Usage: python3 tests/ui_pty.py target/debug/examples/ui_acceptance --driver
       python3 tests/ui_pty.py target/debug/rdu
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unicodedata


class Screen:
    """Small incremental VT reader for Crossterm's cursor-addressed text output."""
    def __init__(self, rows=30, columns=110):
        self.pending = ""
        self.row = self.col = 0
        self.resize(rows, columns)

    def resize(self, rows, columns):
        old=getattr(self,"cells",[])
        self.rows, self.columns = rows, columns
        self.cells = [[old[r][c] if r<len(old) and c<len(old[r]) else " " for c in range(columns)] for r in range(rows)]
        self.row = min(getattr(self,"row",0),rows-1)
        self.col = min(getattr(self,"col",0),columns-1)

    @property
    def text(self):
        return "\n".join("".join(row) for row in self.cells)

    def feed(self, data):
        self.pending += data
        while self.pending:
            if self.pending.startswith("\x1b["):
                match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", self.pending)
                if not match:
                    return
                args, _, code = match.groups()
                self.pending = self.pending[match.end():]
                values = [int(v) if v else 0 for v in args.lstrip("?").split(";")]
                n = values[0] or 1
                if code in "Hf":
                    self.row = min(self.rows - 1, max(0, n - 1))
                    self.col = min(self.columns - 1, max(0, (values[1] if len(values) > 1 else 1) - 1))
                elif code == "G": self.col = min(self.columns - 1, max(0, n - 1))
                elif code == "A": self.row = max(0, self.row - n)
                elif code == "B": self.row = min(self.rows - 1, self.row + n)
                elif code == "C": self.col = min(self.columns - 1, self.col + n)
                elif code == "D": self.col = max(0, self.col - n)
                elif code == "J" and values[0] in (2, 3):
                    self.cells = [[" "] * self.columns for _ in range(self.rows)]
                elif code == "K":
                    start, end = (0, self.columns) if values[0] == 2 else ((0, self.col + 1) if values[0] == 1 else (self.col, self.columns))
                    self.cells[self.row][start:end] = [" "] * (end - start)
                continue
            if self.pending.startswith("\x1b]"):
                end = self.pending.find("\x07")
                if end < 0: return
                self.pending = self.pending[end + 1:]
                continue
            if self.pending[0] == "\x1b":
                if len(self.pending) < 2: return
                self.pending = self.pending[2:]
                continue
            char, self.pending = self.pending[0], self.pending[1:]
            if char == "\r": self.col = 0
            elif char == "\n": self.row = min(self.rows - 1, self.row + 1)
            elif char == "\b": self.col = max(0, self.col - 1)
            elif char >= " ":
                if unicodedata.combining(char): continue
                if self.col < self.columns:
                    self.cells[self.row][self.col] = char
                self.col += 2 if unicodedata.east_asian_width(char) in ("W", "F") else 1
                if self.col >= self.columns:
                    self.col = self.columns - 1


class Session:
    def __init__(self, binary, root, driver=False, writable=False, no_color=False, extra_args=()):
        import codecs
        self.master, self.slave = pty.openpty()
        self.before = termios.tcgetattr(self.slave)
        self.screen = Screen()
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.resize(30, 110)
        if driver:
            args = [binary] + (["--fixture-writable"] if writable else []) + [str(root)]
        else:
            args = [binary] + ([] if writable else ["--read-only"]) + list(extra_args) + [str(root)]
        env = os.environ.copy()
        if no_color: env["NO_COLOR"] = "1"
        self.started = time.monotonic()
        self.proc = subprocess.Popen(args, stdin=self.slave, stdout=self.slave, stderr=self.slave, env=env)
        self.output = b""
        self.loading_at = self.ready_at = None
        self.saw_live_change = False
        self.entries_seen = set()

    def resize(self, rows, columns):
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        self.screen.resize(rows, columns)
        self.resize_offset=len(self.output) if hasattr(self,"output") else None

    def pump(self, timeout=.05):
        if select.select([self.master], [], [], timeout)[0]:
            try: chunk = os.read(self.master, 1024 * 256)
            except OSError: chunk = b""
            self.output += chunk
            self.screen.feed(self.decoder.decode(chunk))
            text = self.screen.text
            if "SCANNING" in text and self.loading_at is None:
                self.loading_at = time.monotonic() - self.started
            if "Scan complete" in text and "Space Stage" in text and self.ready_at is None:
                self.ready_at = time.monotonic() - self.started
            match = re.search(r"Entries observed (\d+)", text)
            if match: self.entries_seen.add(int(match.group(1)))
            return bool(chunk)
        return False

    def wait(self, predicate, timeout=8, fresh=False):
        start = len(self.output)
        end = time.monotonic() + timeout
        if predicate == "Scan complete":
            predicate = lambda text: "Scan complete" in text and "Space Stage" in text
        else:
            predicate = (lambda text, value=predicate: value in text) if isinstance(predicate, str) else predicate
        while time.monotonic() < end:
            self.pump()
            resize_ready=True
            if self.resize_offset is not None:
                changed=self.output[self.resize_offset:]
                clear=changed.find(b"\x1b[2J")
                resize_ready=clear>=0 and changed.rfind(b"\x1b[?25l")>clear
                if resize_ready:self.resize_offset=None
            frame_ready=self.output.endswith(b"\x1b[?25l") or b"\x1b[?1049l" in self.output[-256:]
            if resize_ready and frame_ready and (not fresh or len(self.output) > start) and predicate(self.screen.text): return
            if self.proc.poll() is not None: break
        raise AssertionError(f"expectation failed; exit={self.proc.poll()}; screen:\n{self.screen.text}\nraw:{self.output[-500:]!r}")

    def send(self, data, predicate=None, timeout=8):
        os.write(self.master, data)
        if predicate is not None: self.wait(predicate, timeout=timeout, fresh=True)

    def selected(self, name):
        return lambda text: any(">" in row and name in row for row in text.splitlines())

    def close(self, key=b"q", sig=None, codes=(0,)):
        if key is not None: os.write(self.master, key)
        if sig is not None: self.proc.send_signal(sig)
        end = time.monotonic() + 10
        while self.proc.poll() is None and time.monotonic() < end: self.pump()
        self.proc.wait(timeout=1)
        while self.pump(0): pass
        assert self.proc.returncode in codes, (self.proc.returncode, self.screen.text)
        after = termios.tcgetattr(self.slave)
        before = list(self.before)
        before[3] &= ~getattr(termios, "PENDIN", 0)
        after[3] &= ~getattr(termios, "PENDIN", 0)
        assert after == before, f"termios not restored: {before!r} / {after!r}"
        assert b"\x1b[?25h" in self.output, "cursor not restored"
        assert b"\x1b[?1049l" in self.output, "alternate screen not restored"
        assert b"\x1b]52;c;PWN" not in self.output, "hostile name became an OSC"

    def cleanup(self):
        if self.proc.poll() is None: self.proc.kill(); self.proc.wait()
        os.close(self.master); os.close(self.slave)


def manifest(root):
    """Snapshot fixture bytes/names, never follow links."""
    result = {}
    for base, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            path = Path(base) / name
            result[str(path.relative_to(root))] = (path.lstat().st_ino, path.lstat().st_mode,
                os.readlink(path) if path.is_symlink() else (path.read_bytes() if path.is_file() else None))
    return result


def main():
    binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/rdu").resolve())
    driver = "--driver" in sys.argv
    evidence = {"driver": driver, "checks": []}
    with tempfile.TemporaryDirectory(prefix="rdu-pty-", dir="/tmp") as temporary:
        root = Path(temporary).resolve()
        (root / "alpha").mkdir(); (root / "alpha/nested").write_bytes(b"A" * 65536)
        (root / "zebra").write_bytes(b"Z" * 4096)
        (root / "zulu").mkdir(); (root / "zulu/inside").touch()
        hostile = os.fsencode(root) + b"/evil\x1b]52;c;PWN\x07\n"
        fd = os.open(hostile, os.O_CREAT | os.O_WRONLY, 0o600); os.close(fd)
        before = manifest(root)
        session = Session(binary, root, driver=driver, no_color=True)
        try:
            session.wait("Scan complete")
            assert "READ ONLY" in session.screen.text
            assert "evil\\x1b]52;c;PWN\\x07\\x0a" in session.screen.text
            assert not termios.tcgetattr(session.slave)[3] & (termios.ICANON | termios.ECHO)
            session.send(b"j", session.selected("zebra")); session.send(b"k", session.selected("alpha/"))
            session.send(b"\x1b[B", session.selected("zebra")); session.send(b"\x1b[A", session.selected("alpha/"))
            session.send(b"\r", "Path: " + str(root / "alpha"))
            session.send(b"h", session.selected("alpha/"))
            session.send(b"n", "sort:name"); session.send(b"s", "sort:size")
            session.send(b"a", "apparent"); session.send(b"a", "allocated")
            session.send(b"/zulu", "Filter: zulu_"); session.send(b"\r", "Filter: zulu ")
            session.send(b"\r", "Path: " + str(root / "zulu"))
            session.send(b"n", "sort:name"); session.send(b"h", session.selected("zulu/"))
            session.send(b"?", "Navigation & safety")
            session.resize(12, 60); session.wait("Navigation & safety")
            session.send(b"\x1b[F", "Other keys return")
            session.send(b"?", "Path:")
            session.send(b"i", "Selection")
            session.send(b"\x1b[F", "reclaimed space")
            session.send(b"\x1b", "Path:")
            session.resize(30, 110); session.wait("Scan complete")
            session.send(b"b", "Staging basket"); session.send(b"\x1b", "Path:")
            session.resize(8, 35); session.wait(session.selected("zulu/"))
            session.resize(30, 110); session.wait("Scan complete")
            session.send(b" ", "staging disabled")
            session.send(b"r", "Scan complete")
            session.close()
            evidence["loading_seconds"] = session.loading_at
            evidence["ready_seconds"] = session.ready_at
            assert manifest(root) == before, "read-only UI modified fixture"
        finally: session.cleanup()
        evidence["checks"].append("navigation/filter/sort/parent identity/help/basket/resize/hostile names/NO_COLOR/read-only/refresh")
        for key, sig in [(b"\x03", None), (b"\x04", None), (b"\x1b", None), (None, signal.SIGTERM), (None, signal.SIGHUP)]:
            session = Session(binary, root, driver=driver)
            try:
                session.wait("Scan complete"); session.close(key=key, sig=sig)
            finally: session.cleanup()
        evidence["checks"].append("termios/cursor/alternate-screen restoration: q/Ctrl-C/Ctrl-D/Escape/SIGTERM/SIGHUP")
        # This separate fixture is the only writable UI test. Every mutation is
        # preceded by a complete plan review plus the exact in-app typed phrase.
        (root / "delete-me").mkdir(); (root / "delete-me/inside").write_bytes(b"delete fixture only")
        before = manifest(root)
        session = Session(binary, root, driver=driver, writable=True)
        try:
            session.wait("Scan complete")
            session.send(b"/delete-me", "Filter: delete-me_"); session.send(b"\r", "Filter: delete-me ")
            session.send(b" ", "Staged in memory")
            assert manifest(root) == before, "staging changed filesystem"
            session.send(b"D", "PERMANENT DELETION")
            session.send(b"m", "Fresh immutable manifest")
            session.send(b"\x1b", "PERMANENT DELETION")
            session.send(b"\x1b", "prepared plan discarded")
            assert manifest(root) == before, "cancelled review changed filesystem"
            session.send(b"D", "PERMANENT DELETION")
            # Scroll every screen of the complete path/policy review.
            for _ in range(12): session.send(b"\x1b[6~"); session.pump(.03)
            session.wait("Full review displayed")
            session.resize(8, 35); session.wait("Enlarge terminal")
            session.send(b"DELETE 1\r")
            # Keep the terminal short until the FIFO cancellation is consumed;
            # expanding first would make subsequent queued typing large-screen input.
            session.send(b"\x1b", lambda text: "Path:" in text and "Enlarge terminal" not in text)
            session.resize(30, 110); session.wait("prepared plan discarded")
            assert manifest(root) == before, "short terminal allowed confirmation"
            session.send(b"D", "PERMANENT DELETION")
            for _ in range(12): session.send(b"\x1b[6~"); session.pump(.03)
            session.wait("Full review displayed")
            session.send(b"DELETE 1\r", "OPERATION FINISHED", timeout=15)
            session.wait("Total 2")
            session.wait("Purged 2")
            assert not (root / "delete-me").exists(), "confirmed fixture directory survived successful purge"
            assert (root / "alpha/nested").read_bytes() == b"A" * 65536
            session.send(b"r", "Scan complete")
            session.close()
        finally: session.cleanup()
        evidence["checks"].append("observational stage/fresh manifest/review cancel/resize blocks confirmation/typed fixture deletion/report/rescan")
        if not driver:
            folder=root/"filtered-folder";folder.mkdir()
            (folder/"visible.bin").write_bytes(b"visible fixture")
            (folder/"omitted").mkdir();(folder/"omitted/hidden.bin").write_bytes(b"excluded fixture")
            before=manifest(root)
            session=Session(binary,root,writable=True,extra_args=("--ignore-dirs","filtered-folder/omitted"))
            try:
                session.wait("Scan complete");session.wait("FILTERED SCAN")
                session.send(b"/filtered-folder","Filter: filtered-folder_");session.send(b"\r","Filter: filtered-folder ")
                session.send(b" ","Staged in memory");assert manifest(root)==before
                session.send(b"D","4 manifest objects")
                session.wait("including scan-excluded items")
                session.send(b"m","Fresh immutable manifest")
                found=False
                for index in range(4):
                    session.wait(f"object {index+1} of 4")
                    found |= "hidden.bin" in session.screen.text
                    if index<3:session.send(b"n",f"object {index+2} of 4")
                assert found,"fresh manifest did not expose the excluded descendant"
                session.send(b"\x1b","PERMANENT DELETION")
                session.send(b"\x1b","prepared plan discarded")
                assert manifest(root)==before,"filtered staging or cancelled review changed files"
                session.close()
            finally:session.cleanup()
            evidence["checks"].append("filtered folder: full fresh closure/count/scope warning/excluded descendant inspection; cancelled without writes")
        # A larger owned fixture keeps traversal running long enough to observe
        # actual changing counters and exercise stop-and-keep-partial.
        large = root / "large"; large.mkdir()
        for i in range(300):
            directory = large / str(i); directory.mkdir()
            for j in range(80): (directory / str(j)).touch()
        session = Session(binary, large, driver=driver,extra_args=("--backend","portable","--sequential"))
        try:
            session.wait(lambda text: bool(re.search(r"Entries observed [1-9]", text)), timeout=10)
            assert session.entries_seen, "no actual nonzero observations"
            session.send(b"x", "PARTIAL", timeout=10)
            assert "READ ONLY" in session.screen.text
            session.send(b" ", "staging disabled")
            session.close(codes=(2,))
            evidence["observed_entry_samples"] = sorted(session.entries_seen)
        finally: session.cleanup()
        evidence["checks"].append("actual live counters/stop/keep partial/read-only partial/worker shutdown")
        before=manifest(root)
        session=Session(binary,root,driver=driver,writable=True)
        try:
            session.wait("Scan complete")
            session.send(b"/large","Filter: large_");session.send(b"\r","Filter: large ")
            session.send(b" ","Staged in memory")
            session.send(b"D","CHECKING")
            session.send(b"r","Scan complete")
            assert manifest(root)==before,"refresh during preparation changed fixture"
            session.send(b"/large","Filter: large_");session.send(b"\r","Filter: large ")
            session.send(b" ","Staged in memory")
            session.send(b"D","CHECKING")
            quit_start=time.monotonic();session.close()
            evidence["prepare_quit_seconds"]=time.monotonic()-quit_start
            assert evidence["prepare_quit_seconds"]<2,"quitting preparation did not cancel promptly"
            assert manifest(root)==before,"quit during preparation changed fixture"
        finally:session.cleanup()
        evidence["checks"].append("single cancellable preparation: refresh and prompt quit while CHECKING; fixture unchanged")
        session=Session(binary,large,driver=driver)
        try:
            session.wait("SCANNING");session.close()
        finally:session.cleanup()
        session=Session(binary,root/"missing-root",driver=driver)
        try:
            session.wait("No such file");session.close(key=None,codes=(1,))
        finally:session.cleanup()
        evidence["checks"].append("quit during loading cancels/joins scan; scan error restores terminal")
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__": main()

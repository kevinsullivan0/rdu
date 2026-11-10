#!/usr/bin/env python3
"""Serialize a root-authorized profiling command with paired.py runs."""
import fcntl
from pathlib import Path
import subprocess
import sys

path = Path(__file__).resolve().parent / 'data/measurement.lock'
path.parent.mkdir(exist_ok=True)
with path.open('a+') as lock:
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit('another benchmark/profile holds the measurement lock')
    if len(sys.argv) < 2:
        raise SystemExit('usage: with_measurement_lock.py COMMAND [ARG ...]')
    raise SystemExit(subprocess.call(sys.argv[1:]))

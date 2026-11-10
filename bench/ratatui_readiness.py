#!/usr/bin/env python3
"""Source-bound quantitative current-Ratatui PTY adapter, one explicitly granted child.

No transcripts/screens/names are retained. No terminal replies are generated.
Use --owned-capability first; real-root cases always run --read-only.
"""
import argparse
import errno
import fcntl
import hashlib
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
import tomllib
from vt_screen import VTScreen

KEYS = ('allocated', 'apparent', 'files', 'directories', 'errors')
FRAME_FILES = ('src/ui.rs', 'src/ui/terminal.rs', 'src/ui/render.rs', 'src/ui/format.rs', 'src/ui/view.rs')
MAX_OUTPUT = 16 * 1024 * 1024


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


class Invalid(ValueError):
    """Constant diagnostic code; never include a path-bearing frame or OS message."""


class SampleFailure(Invalid):
    def __init__(self, code, receipt):
        super().__init__(code)
        self.receipt = receipt


def human_size(size):
    if size < 1024:
        return f'{size} B'
    value, unit = float(size), 0
    units = ('B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB', 'EiB')
    while value >= 1024 and unit < 6:
        value /= 1024
        unit += 1
    return f'{value:.1f} {units[unit]}'


class RatatuiScreen(VTScreen):
    """Frozen renderer: complete diff ends SGR0 then execute!(Hide), which flushes.

    Current Ratatui does not emit synchronized-output frames. Setup Hide alone is
    not a draw. Split/coalesced PTY reads do not determine frame boundaries.
    """
    def __init__(self):
        super().__init__(40, 160)
        self.reset_end = None
        self.queries = 0

    def _csi(self, raw, final):
        if final in 'hl' and raw == '?2026':
            raise Invalid('renderer_protocol_changed_sync_output')
        if final in 'cn':
            self.queries += 1
        previous = self.reset_end
        super()._csi(raw, final)
        if raw == '0' and final == 'm':
            self.reset_end = self.characters_consumed
        elif raw == '?25' and final == 'l' and self.alt_screen and previous == self.characters_consumed - 6:
            self.draw_count += 1
            self.frames.append((self.draw_count, tuple(self.lines())))
            if len(self.frames) > 128:
                raise Invalid('completed_frame_queue_bound')

    def _osc_done(self):
        if self.sequence.startswith('52;'):
            raise Invalid('unexpected_clipboard_control')
        super()._osc_done()

    def finish(self):
        self.decoder.decode(b'', final=True)
        if self.state != 'ground' or self.in_sync:
            raise Invalid('incomplete_terminal_protocol_at_exit')


def browser_frame(lines, expected, writable=False):
    if len(lines) != 40:
        raise Invalid('wrong_frame_geometry')
    if 'Scan complete' not in lines[2]:
        return None
    mode = lines[0].strip()
    if mode != ('rdu LIVE · STAGING' if writable else 'rdu READ ONLY'):
        raise Invalid('browser_mode_or_partial_state')
    if not lines[1].startswith(' Path: '):
        raise Invalid('missing_browser_path_row')
    stats = re.fullmatch(r'Here ([0-9]+(?:\.[0-9])? (?:B|KiB|MiB|GiB|TiB|PiB|EiB))   Files ([0-9]+)   Dirs ([0-9]+)   Errors ([0-9]+)   Scan [0-9]+\.[0-9]{2}s', lines[3].strip(' │'))
    if not stats:
        raise Invalid('complete_statistics_shape')
    if stats[1] != human_size(expected['allocated']) or tuple(map(int, stats.groups()[1:])) != tuple(expected[k] for k in ('files', 'directories', 'errors')):
        raise Invalid('complete_formatted_totals_or_counts_mismatch')
    if not all(value in lines[5] for value in ('Files', 'allocated', 'sort:size')) or not all(value in lines[6] for value in ('Name', 'Size')):
        raise Invalid('missing_complete_browser_table')
    footer = re.fullmatch(r'([0-9]+) / ([0-9]+) · Filter: *', lines[38].rstrip())
    if not footer or not all(value in lines[39] for value in ('Select', 'Enter Open', 'Space Stage', 'q Quit')):
        raise Invalid('missing_complete_navigation_footer')
    selected, visible = map(int, footer.groups())
    if visible < 2 or not 1 <= selected <= visible:
        raise Invalid('selection_capability_requires_two_root_entries')
    # Path text is memory-only; even its token is omitted from the returned receipt.
    return {'selected': selected, 'visible': visible, '_path_token': sha(lines[1].encode())}


def loading_frame(lines):
    if not re.fullmatch(r'rdu  SCANNING  [0-9]+\.[0-9]s *', lines[0]):
        return None
    if not lines[1].startswith('Path: ') or 'Live metadata progress' not in lines[3]:
        raise Invalid('incomplete_loading_layout')
    text = '\n'.join(lines[4:12])
    values = {}
    patterns = {'entries': r'Entries observed ([0-9]+)', 'files': r'Files observed ([0-9]+)',
                'completed': r'Directories completed ([0-9]+) / discovered ([0-9]+)',
                'active': r'Active workers ([0-9]+)', 'errors': r'Errors ([0-9]+)'}
    phase = re.search(r'Phase (Scanning|Accounting|Complete|Cancelled)\b', text)
    if not phase:
        raise Invalid('missing_loading_phase')
    for key, pattern in patterns.items():
        match = re.search(pattern, text)
        if not match:
            raise Invalid('missing_loading_counter')
        values[key] = tuple(map(int, match.groups())) if key == 'completed' else int(match[1])
    if values['errors']:
        raise Invalid('invalid_loading_work_or_errors')
    return dict(values, phase=phase[1])


class Milestones:
    def __init__(self, expected, writable=False):
        if set(KEYS) - expected.keys() or any(type(expected[k]) is not int or expected[k] < 0 for k in KEYS) or expected['errors'] or expected['directories'] < 1:
            raise Invalid('invalid_independent_oracle')
        self.expected, self.writable = expected, writable
        self.loading = self.positive = self.ready = self.j_at = self.k_at = None
        self.active_samples = []
        self.last_work = None
        self.browser = None
        self.sent_key = None
        self.sent_at = None
        self.sent_cutoff = None
        self.response_j = self.response_k = None

    def observe(self, serial, lines, arrived):
        load = loading_frame(lines)
        if load:
            if self.ready is not None:
                raise Invalid('unexpected_return_to_loading')
            if self.loading is None:
                self.loading = arrived
            work = (load['entries'], load['files'], *load['completed'])
            if self.last_work is not None and any(a < b for a, b in zip(work, self.last_work)):
                raise Invalid('nonmonotonic_live_work')
            self.last_work = work
            if load['entries'] > 0:
                if self.positive is None:
                    self.positive = arrived
                if load['phase'] == 'Scanning' and load['active'] > 0 and (not self.active_samples or load['entries'] > self.active_samples[-1]['entries']):
                    self.active_samples.append({'seconds': arrived, 'entries': load['entries'], 'files': load['files']})
            return None
        state = browser_frame(lines, self.expected, self.writable)
        if state is None:
            return None
        if self.ready is None:
            if state['selected'] != 1:
                raise Invalid('initial_selection_not_first')
            self.ready, self.browser = arrived, state
            return b'j'
        if state['_path_token'] != self.browser['_path_token'] or state['visible'] != self.browser['visible']:
            raise Invalid('selection_changed_root_or_visible_scope')
        if self.sent_cutoff is None or serial <= self.sent_cutoff:
            return None  # Draws already decoded before the key write cannot prove response.
        target = 2 if self.sent_key == b'j' else 1
        if state['selected'] != target:
            if state['selected'] == (1 if target == 2 else 2):
                return None
            raise Invalid('unexpected_selection_response')
        if self.sent_key == b'j':
            self.j_at, self.response_j = arrived, arrived - self.sent_at
            return b'k'
        if self.sent_key == b'k':
            self.k_at, self.response_k = arrived, arrived - self.sent_at
            return b'q'
        return None

    def sent(self, key, at, cutoff):
        if key not in (b'j', b'k', b'q'):
            raise Invalid('unapproved_input_key')
        self.sent_key, self.sent_at, self.sent_cutoff = key, at, cutoff

    def result(self):
        if self.ready is None or self.response_j is None or self.response_k is None:
            raise Invalid('usable_frame_not_retrospectively_validated')
        observable = len(self.active_samples) >= 3
        return {'first_complete_loading_seconds': self.loading, 'first_positive_counter_seconds': self.positive,
                'live_progress_three_updates_observable': observable,
                'live_progress_observation': 'observed' if observable else 'too_short_or_unobserved',
                'active_progress_samples': self.active_samples,
                'first_usable_browser_seconds': self.ready, 'harmless_j_response_seconds': self.response_j,
                'harmless_k_response_seconds': self.response_k, 'navigation_round_trip_validated': True,
                'formatted_total': human_size(self.expected['allocated']),
                'final_counts': {k: self.expected[k] for k in ('files', 'directories', 'errors')},
                'hidden_exact_bytes_verified_by_screen': False}


def normalize_termios(value):
    value = list(value)
    value[3] &= ~getattr(termios, 'PENDIN', 0)
    return value


def controlling_terminal():
    # Proof is inside the owning session. Parent cross-session tty/PID queries
    # are neither necessary nor portable on Darwin.
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    os.tcsetpgrp(0, os.getpgrp())
    if os.getsid(0) != os.getpid() or os.getpgrp() != os.getpid() or os.tcgetpgrp(0) != os.getpgrp():
        raise OSError(errno.EPERM, 'owned controlling terminal proof failed')


def attributes(fd):
    try:
        return {'available': True, 'value': normalize_termios(termios.tcgetattr(fd))}
    except (OSError, termios.error) as error:
        return {'available': False, 'errno': error.args[0]}


def restoration(before, after):
    observed = {}
    for name in ('master', 'slave'):
        left, right = before[name], after[name]
        item = {'before_available': left['available'], 'after_available': right['available']}
        if left['available'] and right['available']:
            item['equal'] = left['value'] == right['value']
        else:
            item['equal'] = None
            item['before_errno'] = left.get('errno')
            item['after_errno'] = right.get('errno')
        observed[name] = item
    equal = [item['equal'] for item in observed.values() if item['equal'] is not None]
    return {'observations': observed, 'termios_valid_fd_observed': bool(equal),
            'restored': all(equal) if equal else None}


def controlled_environment():
    value = os.environ.copy()
    for key in ('BLOCK_SIZE', 'BLOCKSIZE', 'LINES', 'COLUMNS', 'NO_COLOR', 'TERM_PROGRAM', 'TERM_PROGRAM_VERSION'):
        value.pop(key, None)
    value.update(TERM='xterm-256color', LC_ALL='C')
    return value


def validate_binding(receipt, repo, contract, registry):
    if contract.get('format') != 'rdu-ratatui-frame-contract-v1' or contract.get('geometry') != {'rows': 40, 'columns': 160} or contract.get('term') != 'xterm-256color' or contract.get('locale') != 'C':
        raise Invalid('unsupported_frame_contract')
    binary = Path(receipt['binary']) if isinstance(receipt['binary'], str) else Path(receipt['binary']['path'])
    expected_sha = receipt.get('binary_sha256', receipt['binary'].get('sha256') if isinstance(receipt['binary'], dict) else None)
    expected_size = receipt.get('binary_bytes', receipt['binary'].get('bytes') if isinstance(receipt['binary'], dict) else None)
    data = binary.read_bytes()
    if sha(data) != expected_sha or len(data) != expected_size:
        raise Invalid('changed_binary_binding')
    commit, hashes = receipt['source_commit'], receipt['runtime_and_cargo_sha256']
    if not re.fullmatch('[0-9a-f]{40}', commit):
        raise Invalid('source_commit_not_immutable')
    names = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', commit, '--', 'src', 'Cargo.toml', 'Cargo.lock'], cwd=repo).decode().splitlines()
    if set(names) != set(hashes):
        raise Invalid('incomplete_runtime_source_binding')
    for name, expected in hashes.items():
        if sha(subprocess.check_output(['git', 'show', f'{commit}:{name}'], cwd=repo)) != expected:
            raise Invalid('source_hash_binding_mismatch')
    renderer = {name: hashes[name] for name in FRAME_FILES}
    if sha(canonical(renderer)) not in contract['approved_renderer_map_sha256']:
        raise Invalid('variant_renderer_requires_contract_review')
    lock = tomllib.loads(subprocess.check_output(['git', 'show', f'{commit}:Cargo.lock'], cwd=repo).decode())
    locked = {(p['name'], p['version']): p.get('checksum') for p in lock['package']}
    for package in contract['dependency_packages']:
        if locked.get((package['name'], package['version'])) != package['checksum']:
            raise Invalid('dependency_frame_package_changed')
    for item in contract['dependency_sources']:
        path = registry / item['package_directory'] / item['path']
        if sha(path.read_bytes()) != item['sha256']:
            raise Invalid('dependency_frame_source_changed')
    framework = {name: sha((Path(__file__).parent / name).read_bytes()) for name in ('ratatui_readiness.py', 'vt_screen.py')}
    return binary, {'source_commit': commit, 'runtime_and_cargo_sha256': hashes,
                    'binary_sha256': expected_sha, 'binary_bytes': expected_size,
                    'renderer_map_sha256': sha(canonical(renderer)), 'framework_sha256': framework,
                    'frame_contract_sha256': sha(canonical(contract))}


def validate_external_gates(gates, binding):
    result = {}
    for name in ('exact_cli', 'retained_tree'):
        item = gates.get(name)
        if not isinstance(item, dict) or item.get('passed') is not True or item.get('source_commit') != binding['source_commit'] or item.get('binary_sha256') != binding['binary_sha256']:
            raise Invalid('separate_defining_gates_not_bound')
        if sha(Path(item['receipt']).read_bytes()) != item.get('sha256'):
            raise Invalid('separate_gate_receipt_changed')
        result[name] = {key: item[key] for key in ('sha256', 'source_commit', 'binary_sha256', 'passed')}
    return result


class OwnedFixture:
    def __init__(self, entries):
        if not 2 <= entries <= 100000:
            raise Invalid('owned_fixture_size_out_of_bounds')
        self.directory = tempfile.TemporaryDirectory(prefix='rdu-ui-owned-', dir='/tmp')
        self.path = Path(self.directory.name).resolve()
        for name in ('alpha', 'beta'):
            (self.path / name).mkdir()
        for index in range(entries):
            branch = 'alpha' if index % 2 else 'beta'
            name = 'evil\x1b]52;c;fixture\x07' if index == 0 else f'item-{index:06}'
            (self.path / branch / name).touch()

    def signature(self):
        rows = []
        for parent, dirs, files in os.walk(self.path, followlinks=False):
            for name in ['.'] + sorted(dirs + files):
                path = Path(parent) / name
                value = path.lstat()
                rows.append((str(path.relative_to(self.path)), value.st_dev, value.st_ino, value.st_mode, value.st_size, value.st_mtime_ns))
        return sha(canonical(sorted(rows)))

    def close(self):
        self.directory.cleanup()


def measure_once(binary, root, expected, binding, *, writable_fixture=None, timeout=10):
    """Exactly one product child, no implicit CLI/competitor validation children."""
    writable = writable_fixture is not None
    if writable and (not isinstance(writable_fixture, OwnedFixture) or root != writable_fixture.path):
        raise Invalid('writable_real_root_forbidden')
    if not 0 < timeout <= 10:
        raise Invalid('session_deadline_out_of_bounds')
    command = [str(binary)] + ([] if writable else ['--read-only']) + ['--backend', 'bulk', '--threads', '8', str(root)]
    screen, milestones = RatatuiScreen(), Milestones(expected, writable)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 160, 0, 0))
    before = {'master': attributes(master), 'slave': attributes(slave)}
    process = None
    usage = status = None
    output_hash, output_bytes = hashlib.sha256(), 0
    q_at = exit_at = None
    code = 'unexpected_failure'
    terminal = {'termios_valid_fd_observed': False, 'restored': None}
    phase = 'before_spawn'
    inputs = []
    failure_errno = None
    started = time.perf_counter()

    def reap():
        nonlocal status, usage, exit_at, phase
        if status is not None:
            return
        phase = 'wait4_poll'
        pid, value, resource = os.wait4(process.pid, os.WNOHANG | os.WUNTRACED)
        if pid and os.WIFSTOPPED(value):
            raise Invalid('owned_child_stopped')
        if pid:
            status, usage, exit_at = value, resource, time.perf_counter() - started
            process.returncode = os.waitstatus_to_exitcode(value)

    def pump(wait):
        nonlocal output_bytes, phase
        phase = 'select_terminal_output'
        if not select.select([master], [], [], wait)[0]:
            return False
        try:
            phase = 'read_terminal_output'
            data = os.read(master, 65536)
        except OSError as error:
            if error.errno != errno.EIO:
                raise
            data = b''
        if not data:
            return False
        output_bytes += len(data)
        if output_bytes > MAX_OUTPUT:
            raise Invalid('output_bound_exceeded')
        output_hash.update(data)
        arrived = time.perf_counter() - started
        phase = 'decode_terminal_output'
        screen.feed(data)
        while screen.frames:
            serial, lines = screen.frames.popleft()
            if q_at is not None:
                continue
            phase = 'validate_completed_frame'
            key = milestones.observe(serial, lines, arrived)
            if key is not None:
                if key == b'j':
                    phase = 'ready_termios_observation'
                    ready = {'master': attributes(master), 'slave': attributes(slave)}
                    flags = [item['value'][3] for item in ready.values() if item['available']]
                    terminal['raw_at_ready'] = bool(flags) and all(not value & (termios.ICANON | termios.ECHO) for value in flags)
                    if not terminal['raw_at_ready']:
                        raise Invalid('browser_raw_mode_unobserved_or_mismatched')
                now = time.perf_counter() - started
                phase = 'write_' + key.decode()
                os.write(master, key)
                inputs.append(key.decode())
                milestones.sent(key, now, screen.draw_count)
                if key == b'q':
                    set_q(now)
        return True

    def set_q(value):
        nonlocal q_at
        q_at = value

    try:
        phase = 'spawn_with_child_owned_session_proof'
        process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                                   env=controlled_environment(), preexec_fn=controlling_terminal)
        terminal['child_preexec_session_foreground_proof'] = True
        terminal['parent_cross_session_queries'] = 'not_required_or_performed'
        phase = 'read_frames'
        while status is None and time.perf_counter() - started < timeout:
            pump(.01)
            reap()
        if status is None:
            raise Invalid('session_timeout')
        while select.select([master], [], [], 0)[0]:
            if not pump(0):
                break
        screen.finish()
        if process.returncode != 0 or q_at is None:
            raise Invalid('nonzero_or_early_process_exit')
        phase = 'after_exit_termios_observation'
        after = {'master': attributes(master), 'slave': attributes(slave)}
        terminal.update(restoration(before, after))
        if terminal['restored'] is None:
            raise Invalid('terminal_restoration_observation_unavailable')
        terminal.update(cursor_visible=screen.cursor_visible, alt_enter_seen=screen.saw_alt_enter,
                        alt_leave_seen=screen.saw_alt_leave, alt_screen_active=screen.alt_screen)
        if not all((terminal['restored'], screen.cursor_visible, screen.saw_alt_enter, screen.saw_alt_leave, not screen.alt_screen)):
            raise Invalid('terminal_restoration_mismatch')
        result = milestones.result()
        result.update(endpoint_valid=True, wall_to_exit_seconds=exit_at, ready_to_q_seconds=q_at - milestones.ready,
                      q_to_exit_seconds=exit_at - q_at)
    except Exception as error:
        failure_errno = error.errno if isinstance(error, OSError) else error.args[0] if isinstance(error, termios.error) else None
        code = str(error) if isinstance(error, Invalid) else 'unexpected_' + type(error).__name__
        result = {'endpoint_valid': False, 'failure_code': code, 'failure_phase': phase, 'os_errno': failure_errno}
    finally:
        if process is not None and status is None:
            try:
                process.kill()
            except ProcessLookupError:
                pass
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
            exit_at = time.perf_counter() - started
        for fd in (master, slave):
            os.close(fd)
    result.update(binding=binding, read_only=not writable, session_timeout_seconds=timeout, pty_rows=40, pty_columns=160,
                  generated_terminal_replies=0, received_bytes=output_bytes, received_sha256=output_hash.hexdigest(),
                  completed_draws=screen.draw_count, terminal_queries_seen=screen.queries, terminal=terminal,
                  inputs_written=inputs,
                  returncode=process.returncode if process else None)
    if usage is not None:
        result['resources'] = {'user_seconds': usage.ru_utime, 'system_seconds': usage.ru_stime,
                               'max_rss_bytes': usage.ru_maxrss if sys.platform == 'darwin' else usage.ru_maxrss * 1024,
                               'minor_faults': usage.ru_minflt, 'major_faults': usage.ru_majflt,
                               'block_input': usage.ru_inblock, 'block_output': usage.ru_oublock,
                               'voluntary_switches': usage.ru_nvcsw, 'involuntary_switches': usage.ru_nivcsw}
    if sha(Path(binary).read_bytes()) != binding['binary_sha256']:
        result.update(endpoint_valid=False, failure_code='binary_changed_during_sample')
    if not result['endpoint_valid']:
        raise SampleFailure(result['failure_code'], result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--owned-capability', action='store_true')
    mode.add_argument('--case', type=Path)
    parser.add_argument('--variant-receipt', type=Path, required=True)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--registry-source', type=Path, required=True)
    parser.add_argument('--frame-contract', type=Path, default=Path(__file__).with_name('ratatui_frame_contract.json'))
    parser.add_argument('--owned-writable', action='store_true')
    parser.add_argument('--owned-entries', type=int, default=16)
    parser.add_argument('--timeout', type=float, default=8)
    parser.add_argument('--grant', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--measurement-lock', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('refusing to replace evidence')
    fixture = None
    fixture_before = None
    lock = None
    result = {'format': 'rdu-ratatui-sample-v1', 'execution_grant': args.grant,
              'comparative_tool_ui_eligible': False, 'child_calls': 0,
              'measurement_eligibility': 'capability_nonquiet' if args.owned_capability else 'requires_separate_quiet_stage',
              'capability_only': args.owned_capability}
    try:
        lock = args.measurement_lock.open('a+')
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        lock.seek(0); lock.truncate(); lock.write(json.dumps({'pid': os.getpid(), 'slot': args.grant, 'adapter': 'current-ratatui'})); lock.flush()
        result['measurement_lock_held'] = True
        receipt = json.loads(args.variant_receipt.read_text())
        contract = json.loads(args.frame_contract.read_text())
        binary, binding = validate_binding(receipt, args.repo, contract, args.registry_source)
        if args.owned_capability:
            from harness import oracle
            fixture = OwnedFixture(args.owned_entries)
            root, expected = fixture.path, oracle(fixture.path)
            fixture_before = fixture.signature()
            label = 'owned-writable' if args.owned_writable else 'owned-read-only'
        else:
            if args.owned_writable:
                raise Invalid('writable_case_must_be_adapter_owned')
            case = json.loads(args.case.read_text())
            if case.get('format') != 'rdu-ratatui-case-v1' or not re.fullmatch('[a-z][a-z0-9-]{0,31}', case.get('dataset', '')):
                raise Invalid('invalid_case_schema')
            root, expected, label = Path(case['root']), case['expected'], case['dataset']
            if not root.is_absolute():
                raise Invalid('case_root_must_be_absolute')
            result['separate_defining_gates'] = validate_external_gates(case.get('separate_defining_gates', {}), binding)
            result['case_sha256'] = sha(args.case.read_bytes())
        result.update(dataset=label, child_calls=1)
        result['sample'] = measure_once(binary, root, expected, binding,
                                       writable_fixture=fixture if args.owned_writable else None, timeout=args.timeout)
        if fixture is not None and oracle(root) != expected:
            raise Invalid('owned_fixture_changed')
    except SampleFailure as error:
        result.update(status='failed', failure_code=str(error), sample=error.receipt, retry_permitted=False)
    except Exception as error:
        result.update(status='failed', failure_code=str(error) if isinstance(error, Invalid) else 'unexpected_' + type(error).__name__, retry_permitted=False)
    else:
        result['status'] = 'passed'
    finally:
        if fixture is not None:
            result['owned_fixture_unchanged'] = fixture.signature() == fixture_before
            if not result['owned_fixture_unchanged']:
                result.update(status='failed', failure_code='owned_fixture_changed', retry_permitted=False)
            fixture.close()
        if lock is not None:
            fcntl.flock(lock, fcntl.LOCK_UN); lock.close()
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'status': result['status'], 'child_calls': result['child_calls'], 'failure_code': result.get('failure_code')}))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())

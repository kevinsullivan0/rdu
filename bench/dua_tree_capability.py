#!/usr/bin/env python3
"""At most two stock dua owned PTYs, passive decoder only, no speed endpoint."""
import fcntl,hashlib,json,os,pty,re,select,struct,subprocess,tempfile,termios,threading,time
from pathlib import Path
from harness import oracle
from next_controls import isolated_env
from ui_readiness import owned_controlling_terminal,terminal_modes,termios_record
from vt_screen import VTScreen
ROOT=Path(__file__).resolve().parent.parent
HIDE=b'\x1b[?25l';SHOW=b'\x1b[?25h'

def completed_frame(lines,expected,canonical_root,names):
    footer=lines[-1].strip()
    match=re.fullmatch(r'Sort mode: size, large first\s+Total disk usage:\s*(\d+)\s+b\s+Processed (\d+) entries in (\d+\.\d{2}s)\s*',footer)
    if match is None:return None
    if int(match[1])!=expected['allocated'] or int(match[2])!=expected['files']+expected['directories']-1:raise ValueError('Completed footer differs from independent accounting')
    if not any(canonical_root in line for line in lines[:-1]):raise ValueError('Completed browser root differs')
    rows=[next((i for i,line in enumerate(lines[:-1]) if name in line),None) for name in names]
    if any(i is None for i in rows) or len(set(rows))!=len(names):raise ValueError('Completed browser is missing distinct expected visible rows')
    return {'validated':True,'allocated':int(match[1]),'processed_entries':int(match[2]),'visible_rows':len(names),'current_frame_sha256':hashlib.sha256('\n'.join(lines).encode()).hexdigest()}

def child(command,expected,root,names,env,cwd,once):
    master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',40,160,0,0));before=terminal_modes(termios.tcgetattr(slave));screen=VTScreen();record={'mode':'once' if once else 'interactive','command':command,'hard_child_cap_seconds':8,'timing_endpoint_recorded':False,'terminal_replies_sent':0,'input_events':[],'termios_before':termios_record(before),'complete_frame':None,'navigation_validated':False,'stream_bytes':0,'stream_retained':False,'decoder_source_sha256':hashlib.sha256((ROOT/'bench/vt_screen.py').read_bytes()).hexdigest()}
    process=None;watchdog=None;tail=bytearray();digest=hashlib.sha256();failure=None;deadline=None;stage='initial';saw_hide=saw_show=False;raw_observed=False;callback_fired=[]
    try:
        process=subprocess.Popen(command,stdin=slave,stdout=slave,stderr=slave,cwd=cwd,env=env,preexec_fn=owned_controlling_terminal,close_fds=True)
        def expire():
            if process.poll() is None:
                callback_fired.append(True)
                try:process.kill()
                except ProcessLookupError:pass
        watchdog=threading.Timer(8,expire);watchdog.start();deadline=time.monotonic()+8;os.set_blocking(master,False)
        while True:
            if time.monotonic()>=deadline:
                expire();record['timeout']=True;break
            try:
                state=termios.tcgetattr(slave);raw_observed|=not bool(state[3]&termios.ICANON) and not bool(state[3]&termios.ECHO)
            except (OSError,termios.error):pass
            readable=select.select([master],[],[],min(.05,max(0,deadline-time.monotonic())))[0]
            chunk=b''
            if readable:
                try:chunk=os.read(master,65536)
                except BlockingIOError:chunk=b''
                except OSError as error:
                    if error.errno==5:chunk=b''
                    else:raise
            if chunk:
                digest.update(chunk);record['stream_bytes']+=len(chunk)
                if record['stream_bytes']>1024*1024:raise ValueError('PTY output cap exceeded')
                for byte in chunk:
                    one=bytes([byte]);screen.feed(one);tail.append(byte)
                    if len(tail)>32:del tail[:-32]
                    if tail.endswith(HIDE):
                        saw_hide=True
                        # Source-defined post-draw hide: inspect current decoded cells, never accumulated stripped text.
                        if screen.state=='ground' and stage=='initial':
                            found=completed_frame(screen.lines(),expected,str(root.resolve()),names)
                            if found:
                                record['complete_frame']=found
                                if not once:
                                    record['termios_before_navigation']=termios_record(terminal_modes(termios.tcgetattr(slave)))
                                    os.write(master,b'\r');record['input_events'].append('enter-largest-directory');stage='navigation'
                        elif screen.state=='ground' and stage=='navigation':
                            lines=screen.lines();joined='\n'.join(lines);target=str((root/'largest_dir').resolve())
                            if target in joined and 'inside_big' in joined and 'inside_empty' in joined and 'small_a' not in joined and 'small_b' not in joined:
                                record['navigation_validated']=True;record['navigated_frame_sha256']=hashlib.sha256(joined.encode()).hexdigest();os.write(master,b'q');record['input_events'].append('q');stage='quitting'
                    if tail.endswith(SHOW):saw_show=True
            # Parent deliberately retains slave for valid post-exit termios. No EOF assertion is substituted.
            if process.poll() is not None and not chunk:
                break
        if process.poll() is None:process.kill()
        record['returncode']=process.wait(timeout=1)
    except Exception as error:
        failure=type(error).__name__+': '+str(error);record['coverage_or_setup_error']=failure
    finally:
        if watchdog:watchdog.cancel();watchdog.join(timeout=1)
        if process is not None:
            if process.poll() is None:
                process.kill();process.wait(timeout=1)
            record.setdefault('returncode',process.returncode)
        record['watchdog_fired']=bool(callback_fired);record['stream_sha256']=digest.hexdigest();record['normal_raw_mode_observed']=raw_observed;record['cursor_hide_seen']=saw_hide;record['cursor_show_seen']=saw_show;record['alternate_enter_seen']=screen.saw_alt_enter;record['alternate_leave_seen']=screen.saw_alt_leave;record['alternate_active_after']=screen.alt_screen;record['cursor_visible_after']=screen.cursor_visible
        try:
            after=terminal_modes(termios.tcgetattr(slave));record['termios_after']=termios_record(after);record['termios_restored_by_child']=before==after
            if before!=after:termios.tcsetattr(slave,termios.TCSANOW,before);record['parent_restored_owned_pty']=True
        except (OSError,termios.error) as error:record['termios_after_unavailable_errno']=error.args[0];record['termios_restored_by_child']=False
        os.close(master);os.close(slave);record['owned_pty_closed']=True;record['owned_child_reaped']=process is None or process.poll() is not None
    safe=record.get('returncode')==0 and record['termios_restored_by_child'] and not record['alternate_active_after'] and record['cursor_visible_after'] and not failure and not record.get('timeout') and not record['watchdog_fired']
    if once:
        record['source_mode_obligations']='No raw/alternate-screen requirement; preserve prior modes and restore cursor if hidden'
        record['safe_expected_absence']=bool(safe and record['complete_frame'] is None)
        record['eligible_completed_once_endpoint']=bool(safe and record['complete_frame'])
    else:
        record['source_mode_obligations']='Raw mode and alternate screen entered, completed frame and directory navigation, ordinary q, child restores modes/cursor/alternate'
        record['eligible_usable_browser']=bool(safe and raw_observed and screen.saw_alt_enter and screen.saw_alt_leave and saw_show and record['complete_frame'] and record['navigation_validated'] and record['input_events']==['enter-largest-directory','q'])
    record['safe_to_continue_branch']=bool(safe)
    return record

def main():
    output=ROOT/'evidence/next/dua-tree-cap-01.json'
    if output.exists():raise SystemExit('Two-child capability already attempted; no automatic retry')
    binary=ROOT/'bench/bin/dua-v3.0.0';wanted='42fffd8345ba65cdb33eedc682f09ea1130736adccf72b94fa6446a1c99d89c0';assert hashlib.sha256(binary.read_bytes()).hexdigest()==wanted
    lock=(ROOT/'bench/data/measurement.lock').open('a');fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    result={'slot':'DUA-TREE-CAP-01','scope':'At most two owned-fixture functional PTY children; no timing ratios, real roots, new emulation or terminal replies','binary_sha256':wanted,'helper_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'decoder_sha256':hashlib.sha256((ROOT/'bench/vt_screen.py').read_bytes()).hexdigest(),'children':[],'fixture_recipe':{'directories_including_root':2,'files':4,'payload_sizes':[8193,0,1,2]},'config_policy':'Proven isolated Mac HOME path; gitignore/cleanup heuristics/notifications disabled for hermetic owned capability; bytes format explicit; not a default-feature performance comparison','source_references':['https://github.com/Byron/dua-cli/blob/v3.0.0/src/main.rs','https://github.com/Byron/dua-cli/blob/v3.0.0/src/interactive/app/eventloop.rs','https://github.com/Byron/dua-cli/blob/v3.0.0/src/interactive/widgets/footer.rs']}
    def save():output.write_text(json.dumps(result,indent=2)+'\n')
    try:
        with tempfile.TemporaryDirectory(prefix='rdu-dua-tree-cap-',dir='/tmp') as temporary:
            owned=Path(temporary);root=owned/'tree';root.mkdir();directory=root/'largest_dir';directory.mkdir();(directory/'inside_big').write_bytes(b'x'*8193);(directory/'inside_empty').touch();(root/'small_a').write_bytes(b'a');(root/'small_b').write_bytes(b'bb')
            before=oracle(root);result['oracle_before']=before;assert root.stat().st_blocks==0 and before['errors']==0
            home=owned/'config';home.mkdir();env=isolated_env(home);env.update(TERM='xterm-256color',LC_ALL='C',LC_MESSAGES='C',LANG='C',LANGUAGE='C');env.pop('BLOCK_SIZE',None);env.pop('BLOCKSIZE',None);env.pop('COLUMNS',None);env.pop('LINES',None)
            config=Path(env['HOME'])/'Library/Application Support/dua-cli/config.toml';config.parent.mkdir(parents=True);config.write_text('format = "bytes"\ngitignore = false\ncleanup_heuristics = false\n[notifications]\nscan_finished = false\ndelete_finished = false\n')
            command=[str(binary),'interactive','--threads','8','--format','bytes'];names=['largest_dir','small_a','small_b']
            for once in (True,False):
                args=command+(['--once'] if once else [])+[str(root)]
                record=child(args,before,root,names,env,home,once);result['children'].append(record);save()
                after=oracle(root)
                if after!=before:result['stop_reason']='Owned fixture oracle changed';break
                if not record['safe_to_continue_branch']:result['stop_reason']='Unsafe/unrestored/unsupported/failed child; no further child';break
            result['oracle_after']=oracle(root);result['fixture_stable']=result['oracle_after']==before
        result['owned_fixture_config_cleanup']=True
    finally:
        result['child_count']=len(result['children']);result['binary_unchanged']=hashlib.sha256(binary.read_bytes()).hexdigest()==wanted;result['decoder_unchanged']=hashlib.sha256((ROOT/'bench/vt_screen.py').read_bytes()).hexdigest()==result['decoder_sha256'];save();output.chmod(0o444);fcntl.flock(lock,fcntl.LOCK_UN);lock.close()
    print(json.dumps({'children':result['child_count'],'results':[{'mode':x['mode'],'exit':x.get('returncode'),'once':x.get('eligible_completed_once_endpoint'),'normal':x.get('eligible_usable_browser'),'safe_absence':x.get('safe_expected_absence'),'error':x.get('coverage_or_setup_error')} for x in result['children']],'fixture_stable':result.get('fixture_stable'),'stop_reason':result.get('stop_reason')}))
if __name__=='__main__':main()

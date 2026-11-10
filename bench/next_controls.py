#!/usr/bin/env python3
"""Exact scalar parsers and owned comparator correctness gate, without timing."""
import argparse,datetime,fcntl,hashlib,json,os,re,stat,subprocess,tempfile
from pathlib import Path
from harness import oracle
ROOT=Path(__file__).resolve().parent.parent

def parse_diskus(output):
    if not re.fullmatch(rb'[ \t]*[0-9]+[ \t]*\n?',output):
        raise ValueError('Diskus must emit exactly one integer-byte line')
    return int(output.strip())

def parse_dua(output,root):
    try:text=output.decode('utf8',errors='strict')
    except UnicodeError as error:raise ValueError('invalid UTF-8') from error
    match=re.fullmatch(r'[ \t]*([0-9]+)[ \t]+b[ \t]+([^\r\n\x1b]+)\n?',text)
    if not match or match.group(2)!=str(root):
        raise ValueError('dua must emit exactly one bytes-unit row for the explicit root')
    return int(match.group(1))

def parse_dua_root_and_sentinel(output,root,sentinel):
    lines=output.splitlines(keepends=True)
    if len(lines)!=2:raise ValueError('dua two-input mode must emit exactly two rows, no total')
    total=parse_dua(lines[0],root)
    if parse_dua(lines[1],sentinel)!=0:raise ValueError('sentinel must report zero bytes')
    return total

def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def isolated_env(home):
    env={k:v for k,v in os.environ.items() if not k.startswith(('DUA_','DISKUS_'))}
    for key in ('HOME','XDG_CONFIG_HOME','XDG_CACHE_HOME','XDG_DATA_HOME'):
        path=home/key.lower();path.mkdir(parents=True,exist_ok=True);env[key]=str(path)
    env.update(LC_ALL='C',TERM='dumb',NO_COLOR='1')
    return env

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    if args.output.exists():raise SystemExit('Refusing to overwrite correctness evidence')
    lockpath=ROOT/'bench/data/measurement.lock';lockpath.parent.mkdir(exist_ok=True)
    lock=lockpath.open('a');fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    tools={'diskus':ROOT/'bench/bin/diskus-v0.9.0','dua':ROOT/'bench/bin/dua-v3.0.0'}
    expected_sha={'diskus':'d7bedf2da4c8349eb69fd9bf27379882255413edfba7216bac647252e6602ebe','dua':'42fffd8345ba65cdb33eedc682f09ea1130736adccf72b94fa6446a1c99d89c0'}
    result={'started_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'phase':'correctness only; no child wall/CPU/RSS timings collected','slot':'NEXT-CORRECT-01','euid':os.geteuid(),'shared_measurement_lock':True,'harness_sha256':sha(__file__),'binaries_before':{k:sha(v) for k,v in tools.items()},'samples':[],'work_differences':[]}
    assert result['binaries_before']==expected_sha
    args.output.parent.mkdir(parents=True,exist_ok=True)
    def save():args.output.write_text(json.dumps(result,indent=2)+'\n')
    save()
    try:
      with tempfile.TemporaryDirectory(prefix='rdu-next-controls-') as temp:
        owned=Path(temp);fixtures={name:owned/name for name in ('flat','mixed','nested','denied')}
        for p in fixtures.values():p.mkdir()
        for n in range(24):(fixtures['flat']/f'f{n:03}').write_bytes(b'x'*n)
        for name in ('empty','with spaces','line\nbreak','tab\tname','-dash','unicode-λ'):(fixtures['flat']/name).touch()
        mixed=fixtures['mixed'];(mixed/'payload').write_bytes(b'z'*8193)
        os.link(mixed/'payload',mixed/'hard1');os.link(mixed/'payload',mixed/'hard2')
        (mixed/'empty-directory').mkdir();(mixed/'link-to-dir').symlink_to('empty-directory',target_is_directory=True)
        (mixed/'dangling').symlink_to('absent');(mixed/'parent').symlink_to('..',target_is_directory=True)
        with (mixed/'sparse').open('wb') as f:f.truncate(1024*1024);f.seek(1024*1024-1);f.write(b'x')
        current=fixtures['nested']
        for depth in range(8):
            current=current/f'd{depth}';current.mkdir();(current/'payload').write_bytes(b'n'*(depth+1))
        (fixtures['denied']/'inaccessible').mkdir();(fixtures['denied']/'inaccessible'/'payload').write_bytes(b'denied')
        rootlink=owned/'root-link';rootlink.symlink_to(fixtures['mixed'],target_is_directory=True)
        allpaths={**{k:v for k,v in fixtures.items() if k!='denied'},'root-symlink':rootlink,'missing':owned/'missing'}
        result['oracles_before']={k:oracle(v) for k,v in allpaths.items()}
        result['fixture_policy']='Owned synthetic paths only; raw stderr/stdout may include disposable fixture names, no real user data. Fixture contents discarded at end.'
        homes=[owned/'config-a',owned/'config-b']
        for p in homes:p.mkdir()
        # Two empty independent config environments; poisoned-file resolver validation is separate.
        def sample(tool,name,path,mode='allocated',profile=0,extra=None,declared='clean-total'):
            cmd=[str(tools[tool])]
            if tool=='diskus':cmd+=['--threads','8','--directories','included']
            else:cmd+=['aggregate','--threads','8','--format','bytes','--no-sort']
            if mode=='apparent':cmd+=['--apparent-size']
            if extra:cmd+=extra
            cmd+=[str(path)]
            entry={'tool':tool,'case':name,'mode':mode,'config_profile':profile,'command':cmd,'declared_contract':declared,'oracle_before':oracle(path)}
            try:
                p=subprocess.run(cmd,cwd=homes[profile],env=isolated_env(homes[profile]),stdin=subprocess.DEVNULL,capture_output=True,timeout=15)
                entry.update(returncode=p.returncode,stdout=p.stdout.decode('utf8','replace'),stderr=p.stderr.decode('utf8','replace'),stdout_bytes=len(p.stdout),stderr_bytes=len(p.stderr))
                try:entry['reported_bytes']=(parse_diskus(p.stdout) if tool=='diskus' else parse_dua(p.stdout,path))
                except ValueError as error:entry['parse_error']=str(error)
                expected=entry['oracle_before'][mode]
                entry.update(expected_bytes=expected,total_matches=entry.get('reported_bytes')==expected,clean_success=p.returncode==0 and not p.stderr)
                entry['eligible_clean_total']=bool(entry['total_matches'] and entry['clean_success'] and entry['oracle_before']['errors']==0)
            except subprocess.TimeoutExpired:entry.update(timeout=True,eligible_clean_total=False)
            entry['oracle_after']=oracle(path);entry['input_stable']=entry['oracle_before']==entry['oracle_after']
            entry['eligible_clean_total']=entry['eligible_clean_total'] and entry['input_stable']
            result['samples'].append(entry);save()
        for name in ('flat','mixed','nested'):
            for mode in ('allocated','apparent'):
                for tool in tools:
                    for profile in (0,1):sample(tool,name,fixtures[name],mode,profile)
        for tool in tools:
            for name in ('missing','root-symlink'):sample(tool,name,allpaths[name],declared='edge/error-contract; no automatic timing eligibility')
        denied=fixtures['denied']/'inaccessible';denied.chmod(0)
        try:
            for tool in tools:sample(tool,'permission-denied',fixtures['denied'],declared='partial traversal; examine exit/stderr independently of bytes')
        finally:denied.chmod(0o700)
        # Explicitly demonstrate that omitting directory inclusion in apparent Diskus differs.
        for name in ('flat','mixed','nested'):
            cmd=[str(tools['diskus']),'--threads','8','--apparent-size',str(fixtures[name])]
            p=subprocess.run(cmd,cwd=homes[0],env=isolated_env(homes[0]),stdin=subprocess.DEVNULL,capture_output=True,timeout=15)
            reported=parse_diskus(p.stdout);o=oracle(fixtures[name])
            result['work_differences'].append({'tool':'diskus','case':name,'command':cmd,'returncode':p.returncode,'stderr':p.stderr.decode(),'reported_bytes':reported,'all_entry_apparent':o['apparent'],'directory_apparent':o['directory_apparent'],'equals_apparent_minus_directories':reported==o['apparent']-o['directory_apparent'],'is_clean_matching_contract':reported==o['apparent']})
        result['oracles_after']={k:oracle(v) for k,v in allpaths.items()}
        result['all_fixture_oracles_stable']=result['oracles_before']==result['oracles_after']
        result['config_verdict']='Two independent empty temporary HOME/XDG environments produce recorded results; this alone is not proof of the macOS resolver. Source-derived path and poisoned-config detection require a separate bounded gate before timing.'
        result['cleanup']='Owned TemporaryDirectory removed after restoring owned permission bits; no real paths modified'
    finally:
        result['binaries_after']={k:sha(v) for k,v in tools.items()}
        result['binaries_unchanged']=result['binaries_before']==result['binaries_after'];save();args.output.chmod(0o444)
        fcntl.flock(lock,fcntl.LOCK_UN);lock.close()
    print(json.dumps({'samples':len(result['samples']),'clean_eligible':sum(s['eligible_clean_total'] for s in result['samples']),'stable':result['all_fixture_oracles_stable'],'binaries_unchanged':result['binaries_unchanged']}))

if __name__=='__main__':main()

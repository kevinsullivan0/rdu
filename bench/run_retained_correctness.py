#!/usr/bin/env python3
"""Run only the granted twelve-child owned sorted-retained-output functional gate."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
import time

from harness import oracle
from paired_sparse import isolated_environment
from retained_output import parse_dua_tree, parse_rdu_tree, validate_rows

ROOT=Path(__file__).resolve().parent.parent
OUTPUT=ROOT/'evidence/next/retained-sorted-full-output-correctness-02.json'
PROPOSAL=ROOT/'evidence/next/retained-sorted-full-output-correctness-02-proposal.json'
SPARSE_BYTES=262144


def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def model(root):
    nodes={};pending=[root]
    while pending:
        path=pending.pop();m=path.lstat();name=str(path.relative_to(root)) if path!=root else ''
        kind='directory' if stat.S_ISDIR(m.st_mode) else 'symlink' if stat.S_ISLNK(m.st_mode) else 'file'
        nodes[name]={'kind':kind,'allocated':m.st_blocks*512,'apparent':m.st_size,
                     'hardlink':(m.st_dev,m.st_ino) if stat.S_ISREG(m.st_mode) and m.st_nlink>1 else None,
                     'identity':(m.st_dev,m.st_ino),'mode':m.st_mode,'mtime_ns':m.st_mtime_ns}
        if kind=='directory':
            with os.scandir(path) as entries:pending.extend(Path(e.path) for e in entries)
    totals=oracle(root)
    if totals['errors']:raise ValueError('Owned oracle error')
    return {'nodes':nodes,'totals':totals}


def fingerprint(value):return hashlib.sha256(json.dumps(value,sort_keys=True).encode()).hexdigest()


def exceeded_budget_fields(budget):
    fields=[]
    if budget['errors']:fields.append('oracle_errors')
    if budget['files']+budget['directories']>128:fields.append('observed_names')
    if budget['apparent']>2097152:fields.append('apparent_bytes')
    if budget['allocated']>1048576:fields.append('allocated_bytes')
    return fields


def make_sparse(path):
    with path.open('wb') as stream:stream.truncate(SPARSE_BYTES)


def generate(owned):
    fixtures={name:owned/name for name in ('composite','nested','denied','hostile-output')}
    for p in fixtures.values():p.mkdir()
    p=fixtures['composite'];(p/'payload').write_bytes(b'x'*8193)
    os.link(p/'payload',p/'hard-a');os.link(p/'payload',p/'hard-b')
    (p/'zero').touch();(p/'two words').touch();(p/'unicode-λ').touch();(p/'empty').mkdir()
    (p/'dir-link').symlink_to('empty',target_is_directory=True)
    (p/'parent-link').symlink_to('..',target_is_directory=True);(p/'dangling').symlink_to('absent')
    make_sparse(p/'sparse')
    current=fixtures['nested']
    for depth in range(8):
        (current/'payload').write_bytes(b'n'*(depth+1));(current/'empty').mkdir()
        current=current/'branch';current.mkdir()
    (current/'payload').write_bytes(b'end')
    blocked=fixtures['denied']/'blocked';blocked.mkdir();(blocked/'payload').write_bytes(b'denied')
    # Newline is a valid Mac filename; no invalidUTF8 filesystem creation is attempted.
    (fixtures['hostile-output']/'line\nbreak').write_bytes(b'h')
    fixtures['missing']=owned/'missing'
    sentinel=owned/'zero-sentinel';sentinel.touch()
    return fixtures,sentinel,blocked


def main():
    if OUTPUT.exists():raise SystemExit('Existing functional result; no retry')
    proposal=json.loads(PROPOSAL.read_text())
    assert proposal['max_child_calls']==12 and len(proposal['cases'])==12
    tools=proposal['tools']
    if any(sha(v['path'])!=v['sha256'] for v in tools.values()):raise SystemExit('Pinned binary mismatch')
    checks=[{'path':str(PROPOSAL),'sha256':sha(PROPOSAL)}]
    for name in ('run_retained_correctness.py','retained_output.py','test_retained_output.py','harness.py','paired_sparse.py'):
        p=ROOT/'bench'/name;checks.append({'path':str(p),'sha256':sha(p)})
    result={'slot':'RETAINED-SORTED-FULL-OUTPUT-CORRECT-02','grant':'Root reviewed revised02 and explicitly granted twelve-child owned functional gate after pure tests/code commit; no real roots or timing claims',
            'proposal':proposal,'source_checks':checks,'status':'preflight','samples':[],'euid':os.geteuid(),
            'invalid_utf8_filesystem_generation':'Not attempted; valid newline negative plus raw-byte parser unit negative',
            'performance_inference':False,'builds_active':'A/B builds/tests may run; child watchdog scalars are not performance data',
            'binaries_before':{k:sha(v['path']) for k,v in tools.items()}}
    def save():
        temporary=OUTPUT.with_suffix('.partial');temporary.write_text(json.dumps(result,indent=2)+'\n');temporary.replace(OUTPUT)
    lock=(ROOT/'bench/data/measurement.lock').open('a+')
    fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB);lock.seek(0);lock.truncate();lock.write(json.dumps({'pid':os.getpid(),'slot':result['slot']}));lock.flush()
    deadline=time.monotonic()+150
    try:
        with tempfile.TemporaryDirectory(prefix='rdu-retained-owned-',dir='/tmp') as temporary:
            owned=Path(temporary).resolve(strict=True);fixtures,sentinel,blocked=generate(owned)
            blocked_mode=stat.S_IMODE(blocked.lstat().st_mode)
            config=owned/'isolated-config';config.mkdir();env=isolated_environment(config)
            before={name:model(path) for name,path in fixtures.items() if name!='missing'}
            budget=oracle(owned)
            result['owned_budget_before']=budget
            result['budget_exceeded_fields_before']=exceeded_budget_fields(budget);save()
            if result['budget_exceeded_fields_before']:
                raise ValueError('Owned fixture budget exceeded: '+','.join(result['budget_exceeded_fields_before']))
            result['fixture_oracles_before']={name:{'totals':m['totals'],'root_identity':m['nodes']['']['identity'],'metadata_fingerprint':fingerprint(m)} for name,m in before.items()}
            result['sentinel_before']=oracle(sentinel);save()
            try:
                for index,case in enumerate(proposal['cases']):
                    if len(result['samples'])>=12:raise ValueError('Child budget exhausted')
                    name=case['fixture'];path=fixtures[name];tool=case['tool'];metric=case['mode']
                    if name=='denied' and os.geteuid()==0:
                        result['permission_negative_unavailable']='euid0 bypasses discretionary permission denial';continue
                    denied=name=='denied'
                    if denied:blocked.chmod(0)
                    try:
                        snapshot=oracle(path);snapshot_model=model(path) if name not in ('missing','denied') else None
                        command=[tools[tool]['path'] if token in ('FROZEN_RDU','FROZEN_DUA') else str(path) if token=='ROOT' else str(sentinel) if token=='OWNED_ZERO_SENTINEL' else token
                                 for token in proposal['command_templates'][tool]]
                        if 'depth' in case:command[command.index('--depth')+1]=case['depth']
                        if metric=='apparent':command.insert(command.index('--depth'),'--apparent-size')
                        remaining=deadline-time.monotonic()
                        if remaining<=0:raise TimeoutError('150-second functional deadline')
                        sample={'index':index,**case,'command':command,'oracle_before':snapshot,'valid_for_clean_component':False,'expected_contract_observed':False}
                        try:
                            child=subprocess.run(command,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                                                 cwd=owned,env=env,timeout=min(10,remaining))
                            stdout,stderr=child.stdout,child.stderr
                            sample.update(returncode=child.returncode,stdout_bytes=len(stdout),stdout_lines=len(stdout.splitlines()),
                                          stdout_sha256=hashlib.sha256(stdout).hexdigest(),stderr_bytes=len(stderr),stderr_sha256=hashlib.sha256(stderr).hexdigest())
                            if len(stdout)>65536 or len(stderr)>65536:raise ValueError('Owned output limit')
                            negative=name in ('missing','denied','hostile-output') or 'depth' in case
                            if name in ('missing','denied'):
                                if child.returncode==0 or not re.search(rb'<[1-9][0-9]* IO Errors?>',stdout) or b'panicked' in stdout+stderr:
                                    raise ValueError('Expected declared error exit/inline error absent')
                                sample['declared_error_observed']=True
                            else:
                                if child.returncode or stderr:raise ValueError('Unexpected application exit/stderr')
                                parse=lambda:parse_rdu_tree(stdout) if tool=='rdu' else parse_dua_tree(stdout,str(path),str(sentinel))
                                if name=='hostile-output':
                                    try:parse()
                                    except ValueError as error:sample['expected_output_coverage_rejection']=str(error)
                                    else:raise ValueError('Newline output coverage unexpectedly accepted')
                                else:
                                    parsed=parse()
                                    if 'depth' in case:
                                        if set(parsed['rows'])!={''} or parsed['rows']['']!=snapshot[metric]:raise ValueError('Declared shallow root contract differs')
                                        try:validate_rows(parsed,snapshot_model,metric,tool)
                                        except ValueError:sample['expected_full_retention_rejection']=True
                                        else:raise ValueError('Shallow tree incorrectly eligible')
                                    else:
                                        sample['validation']=validate_rows(parsed,snapshot_model,metric,tool)
                                        sample['valid_for_clean_component']=True
                            sample['expected_contract_observed']=True
                            if negative:
                                sample['owned_stdout_excerpt_escaped']=stdout[:2048].decode('utf8','backslashreplace').encode('unicode_escape').decode('ascii')
                                sample['owned_stderr_excerpt_escaped']=stderr[:2048].decode('utf8','backslashreplace').encode('unicode_escape').decode('ascii')
                        except subprocess.TimeoutExpired:
                            sample['unexpected_failure']='Owned child10-second/remaining deadline';sample['timed_out']=True
                        except (ValueError,UnicodeError) as error:
                            sample['unexpected_failure']=str(error)
                            if 'stdout' in locals():sample['owned_stdout_excerpt_escaped']=stdout[:2048].decode('utf8','backslashreplace').encode('unicode_escape').decode('ascii')
                            if 'stderr' in locals():sample['owned_stderr_excerpt_escaped']=stderr[:2048].decode('utf8','backslashreplace').encode('unicode_escape').decode('ascii')
                        sample['oracle_after']=oracle(path);sample['input_stable']=snapshot==sample['oracle_after']
                        if snapshot_model is not None:sample['metadata_stable']=snapshot_model==model(path)
                        else:sample['metadata_stable']=True
                        result['samples'].append(sample);save();print(json.dumps({'index':index,'case':name,'tool':tool,'expected':sample['expected_contract_observed']}),flush=True)
                        if not sample['expected_contract_observed'] or not sample['input_stable'] or not sample['metadata_stable']:
                            raise ValueError('Unexpected functional failure; stopped without retry')
                        if any(sha(c['path'])!=c['sha256'] for c in checks) or any(sha(v['path'])!=v['sha256'] for v in tools.values()):
                            raise ValueError('Pinned binary/source changed')
                    finally:
                        if denied:blocked.chmod(blocked_mode)
                result['status']='completed'
            finally:
                blocked.chmod(blocked_mode)
                after={name:model(path) for name,path in fixtures.items() if name!='missing'}
                result['fixtures_stable']=before==after
                result['fixture_oracles_after']={name:{'totals':m['totals'],'root_identity':m['nodes']['']['identity'],'metadata_fingerprint':fingerprint(m)} for name,m in after.items()}
                result['sentinel_after']=oracle(sentinel)
                result['sentinel_stable']=result['sentinel_before']==result['sentinel_after']
                result['owned_budget_after']=oracle(owned)
                result['budget_exceeded_fields_after']=exceeded_budget_fields(result['owned_budget_after']);save()
                if result['budget_exceeded_fields_after']:raise ValueError('Owned resource budget exceeded after cases')
                if not result['fixtures_stable'] or not result['sentinel_stable']:raise ValueError('Owned metadata/identity changed')
        result['owned_cleanup_complete']=not owned.exists()
    except Exception as error:
        result['status']='stopped';result['stop_reason']=type(error).__name__+': '+str(error)
        result['owned_cleanup_complete']=not owned.exists() if 'owned' in locals() else None
    finally:
        result['binaries_after']={k:sha(v['path']) for k,v in tools.items()}
        result['binaries_unchanged']=result['binaries_before']==result['binaries_after']
        result['source_checks_after']=all(sha(c['path'])==c['sha256'] for c in checks)
        result['child_calls']=len(result['samples'])
        result['clean_component_eligible']=result['status']=='completed' and sum(s['valid_for_clean_component'] for s in result['samples'])==8 and result.get('fixtures_stable') and result.get('sentinel_stable') and result['binaries_unchanged'] and result['source_checks_after']
        save();OUTPUT.chmod(0o444);fcntl.flock(lock,fcntl.LOCK_UN);lock.close()
    print(json.dumps({'status':result['status'],'children':result['child_calls'],'clean_component_eligible':result['clean_component_eligible'],'stop_reason':result.get('stop_reason')}),flush=True)


if __name__=='__main__':main()

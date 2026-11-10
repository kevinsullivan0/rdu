#!/usr/bin/env python3
"""Bounded stock dua root/config semantics follow-up, no performance timing."""
import datetime,fcntl,json,os,subprocess,tempfile
from pathlib import Path
from harness import oracle
from next_controls import isolated_env,parse_dua_root_and_sentinel,sha
ROOT=Path(__file__).resolve().parent.parent
OUT=ROOT/'evidence/next/controls-dua-semantics-02.json'
if OUT.exists():raise SystemExit('Refusing to replace prior semantic evidence')
lock=(ROOT/'bench/data/measurement.lock').open('a');fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
binary=ROOT/'bench/bin/dua-v3.0.0'
r={'phase':'stock command/config correctness only; no performance timing','source_hypothesis':'A sole directory expands to contents and excludes root/symlinks; two explicit inputs bypass expansion. Owned zero-length sentinel forces root-preserving Paths route with explicitly different work.','source_urls':['https://github.com/Byron/dua-cli/blob/v3.0.0/src/main.rs#L732','https://github.com/Byron/dua-cli/blob/v3.0.0/src/config.rs#L685'],'binary_before_sha256':sha(binary),'script_sha256':sha(__file__),'samples':[],'started_utc':datetime.datetime.now(datetime.timezone.utc).isoformat()}
def save():OUT.write_text(json.dumps(r,indent=2)+'\n')
try:
 with tempfile.TemporaryDirectory(prefix='rdu-next-dua-semantics-') as temp:
  owned=Path(temp);home=owned/'isolated';home.mkdir();env=isolated_env(home);sentinel=owned/'zero-sentinel';sentinel.touch()
  fixtures={n:owned/n for n in ('flat','mixed','nested')}
  for path in fixtures.values():path.mkdir()
  for n in range(8):(fixtures['flat']/f'f{n}').write_bytes(b'x'*n)
  m=fixtures['mixed'];(m/'payload').write_bytes(b'h'*4097);os.link(m/'payload',m/'hard');(m/'dir').mkdir();(m/'dir'/'nested').write_bytes(b'inner');(m/'symlink-file').symlink_to('payload');(m/'symlink-dir').symlink_to('dir',target_is_directory=True);(m/'dangling').symlink_to('absent')
  here=fixtures['nested']
  for n in range(3):here=here/f'd{n}';here.mkdir();(here/'payload').write_bytes(b'x'*(n+1))
  r['oracles_before']={n:oracle(p) for n,p in fixtures.items()};r['sentinel_oracle']=oracle(sentinel)
  def run(label,cmd,env_override=None):
   p=subprocess.run(cmd,cwd=home,env=env if env_override is None else env_override,stdin=subprocess.DEVNULL,capture_output=True,timeout=15)
   entry={'case':label,'command':cmd,'returncode':p.returncode,'stdout':p.stdout.decode('utf8','replace'),'stderr':p.stderr.decode('utf8','replace')};r['samples'].append(entry);save();return entry,p.stdout
  for name,path in fixtures.items():
   for mode in ('allocated','apparent'):
    cmd=[str(binary),'aggregate','--threads','8','--format','bytes','--no-sort','--no-total']
    if mode=='apparent':cmd+=['--apparent-size']
    entry,out=run('two-input-'+name+'-'+mode,cmd+[str(path),str(sentinel)])
    try:entry['reported_bytes']=parse_dua_root_and_sentinel(out,path,sentinel)
    except ValueError as error:entry['parse_error']=str(error)
    entry.update(expected_bytes=r['oracles_before'][name][mode],total_matches=entry.get('reported_bytes')==r['oracles_before'][name][mode],config='empty isolated HOME/XDG',mode=mode)
  # One diagnostic tests depth as an alternative, without changing parser/gate contracts.
  entry,out=run('single-input-depth1-mixed-apparent',[str(binary),'aggregate','--threads','8','--format','bytes','--no-sort','--depth','1','--apparent-size',str(m)])
  entry['expectation']='Source says sole-root expansion remains; output is a work-difference diagnostic, not a substitute accepted contract'
  # Place poison ONLY at the source-documented path below the owned subprocess HOME.
  config=Path(env['HOME'])/'Library/Application Support/dua-cli/config.toml';config.parent.mkdir(parents=True)
  config.write_text('format = [\n')
  cmd=[str(binary),'aggregate','--threads','8','--format','bytes','--no-sort','--no-total',str(fixtures['flat']),str(sentinel)]
  entry,out=run('owned-mac-config-poison',cmd);entry['owned_config_path']=str(config);entry['detected_owned_poison']=entry['returncode']!=0 and str(config) in entry['stderr']
  config.unlink()
  xdg=Path(env['XDG_CONFIG_HOME'])/'dua-cli/config.toml';xdg.parent.mkdir(parents=True,exist_ok=True);xdg.write_text('format = [\n')
  entry,out=run('owned-xdg-poison-mac-ignores',cmd);entry['mac_xdg_ignored']=entry['returncode']==0 and not entry['stderr']
  xdg.unlink()
  config.write_text('format = "bytes"\n')
  no_format=[a for a in cmd];i=no_format.index('--format');del no_format[i:i+2]
  entry,out=run('owned-mac-config-format-without-cli-override',no_format)
  try:entry['reported_bytes']=parse_dua_root_and_sentinel(out,fixtures['flat'],sentinel)
  except ValueError as error:entry['parse_error']=str(error)
  entry['owned_config_applied']=entry['returncode']==0 and entry.get('reported_bytes')==r['oracles_before']['flat']['allocated']
  config.unlink()
  poisoned={**env,'DUA_FORMAT':'NOT_A_FORMAT','DUA_THREADS':'NOT_A_NUMBER'}
  entry,out=run('unsanitized-env-negative-control',cmd,poisoned);entry['invalid_env_rejected']=entry['returncode']!=0
  cleaned={k:v for k,v in poisoned.items() if not k.startswith('DUA_')}
  entry,out=run('sanitized-env-positive-control',cmd,cleaned)
  try:entry['reported_bytes']=parse_dua_root_and_sentinel(out,fixtures['flat'],sentinel)
  except ValueError as error:entry['parse_error']=str(error)
  entry['sanitized_command_passed']=entry['returncode']==0 and not entry['stderr'] and entry.get('reported_bytes')==r['oracles_before']['flat']['allocated']
  r['oracles_after']={n:oracle(p) for n,p in fixtures.items()};r['inputs_stable']=r['oracles_before']==r['oracles_after'];r['cleanup']='Owned temporary fixture/config/sentinel paths removed; no user config edited/read by harness'
finally:
 r['binary_after_sha256']=sha(binary);r['binary_unchanged']=r['binary_before_sha256']==r['binary_after_sha256'];save();OUT.chmod(0o444);fcntl.flock(lock,fcntl.LOCK_UN);lock.close()
print(json.dumps({'samples':len(r['samples']),'two_input_matches':sum(s.get('total_matches',False) for s in r['samples']),'stable':r.get('inputs_stable'),'binary_unchanged':r['binary_unchanged']}))

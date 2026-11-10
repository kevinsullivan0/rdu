#!/usr/bin/env python3
"""Generate sealed bounded owned holdouts; no scanner or timing is launched."""
import datetime,hashlib,json,os,random
from pathlib import Path
from harness import oracle
ROOT=Path(__file__).resolve().parent.parent
BASE=ROOT/'bench/data/next-holdouts-v1'
OUT=ROOT/'evidence/next/holdouts-v1.json'
SEED=2026111103

def main():
    if BASE.exists() or OUT.exists():raise SystemExit('Refusing to replace an existing holdout or seal')
    BASE.mkdir(parents=True);(BASE/'RDU_OWNED_MARKER').write_text('Owned synthetic holdout fixtures; manifest outside input roots\n')
    rng=random.Random(SEED);recipes={}
    burst=BASE/'burst';burst.mkdir()
    for branch in range(128):
        here=burst
        for depth in range(8):here=here/f'b{branch:03}-d{depth}';here.mkdir()
        for f in range(256):(here/f'f{f:04}-{rng.getrandbits(32):08x}').touch()
    recipes['burst']={'independent_trunks':128,'levels_per_trunk':8,'files_per_leaf':256,'expected_files':32768,'expected_directories':1025}
    skew=BASE/'skew';skew.mkdir()
    for n in range(4000):(skew/f'empty{n:05}').mkdir()
    for branch in range(4):
        hot=skew/f'hot{branch}';hot.mkdir()
        for f in range(15000):(hot/f'f{f:05}-{rng.getrandbits(32):08x}').touch()
    recipes['skew']={'empty_directories':4000,'hot_directories':4,'files_per_hot':15000,'expected_files':60000,'expected_directories':4005}
    long=BASE/'long-names';long.mkdir()
    for group in range(12):
        directory=long/f'g{group:02}';directory.mkdir()
        for f in range(2000):
            prefix=f'f{f:04}-{rng.getrandbits(32):08x}-';target=80+rng.randrange(161)
            # Alternating ASCII/UTF-8, with exactly the chosen encoded length <=240.
            if f%2:
                remaining=target-len(prefix);tail='λ'*(remaining//2)+'x'*(remaining%2)
            else:tail='x'*(target-len(prefix))
            name=prefix+tail;assert len(name.encode())==target
            (directory/name).touch()
    recipes['long-names']={'groups':12,'files_per_group':2000,'encoded_name_bytes':[80,240],'expected_files':24000,'expected_directories':13}
    oracles={name:oracle(BASE/name) for name in recipes}
    for name,o in oracles.items():
        assert o['files']==recipes[name]['expected_files'];assert o['directories']==recipes[name]['expected_directories'];assert o['errors']==0
    total_names=sum(o['files']+o['directories'] for o in oracles.values());allocated=sum(o['allocated'] for o in oracles.values())
    assert total_names<200000 and allocated<384*1024*1024
    record={'created_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'seed':SEED,'generator_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'recipes':recipes,'input_paths':{n:str(BASE/n) for n in recipes},'oracles_at_seal':oracles,'total_names':total_names,'total_st_blocks_allocated_bytes':allocated,'logical_payload_bytes':0,'limits':{'max_names':200000,'max_st_blocks_allocated_bytes':384*1024*1024},'allocation_limit_note':'st_blocks excludes uncharged filesystem inode/catalog overhead; no total physical-footprint claim. No payload copies or large files.','exposure':'New unmeasured synthetic shapes; not used by controls correctness gate; disclosed recipes and oracle warming mean not blind or cold. No performance or selection has run.','larger_public_tree':'Proposed whole public SDK root remains unscanned; bounded preflight and separate grant required.','sealed_before_tuning':True,'timed_runs':0}
    OUT.parent.mkdir(parents=True,exist_ok=True);OUT.write_text(json.dumps(record,indent=2)+'\n');OUT.chmod(0o444)
    print(json.dumps({'names':total_names,'allocated_st_blocks':allocated,'output':str(OUT)}))
if __name__=='__main__':main()

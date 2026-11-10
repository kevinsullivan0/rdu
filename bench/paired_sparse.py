"""Exact-case extension for parent-frozen mechanism screens; old matrix stays intact."""
import fcntl,hashlib,json,os,platform,random,tempfile,time
from pathlib import Path
from harness import measure,oracle
from next_controls import parse_diskus,parse_dua_root_and_sentinel
from output_policy import parse_gdu_root,redact_gdu_output
from paired_stats import paired_effect

KEYS=('allocated','apparent','files','directories','errors')

def bounded_campaign_oracle(path,deadline,bounded):
    if not bounded:return oracle(path)
    import signal
    remaining=deadline-time.monotonic()
    if remaining<=0:raise TimeoutError('Frozen total deadline exhausted before oracle')
    def expired(signum,frame):raise TimeoutError('Frozen total deadline during metadata oracle')
    previous=signal.signal(signal.SIGALRM,expired)
    signal.setitimer(signal.ITIMER_REAL,remaining)
    try:return oracle(path)
    finally:
        signal.setitimer(signal.ITIMER_REAL,0)
        signal.signal(signal.SIGALRM,previous)

def validate_sample(variant,sample,expected,path):
    kind=variant.get('kind','rdu');sample['matches_oracle']=False
    stdout=sample.get('stdout','');stderr=sample.get('stderr','')
    try:
        clean=sample.get('returncode')==0 and not stderr and not sample.get('timed_out',False) and expected['errors']==0
        if kind in ('rdu','wide-probe'):
            parsed=json.loads(stdout);totals=parsed.get('totals',parsed);sample['totals']=totals
            okay=all(type(totals[k]) is int and totals[k]>=0 and totals[k]==expected[k] for k in KEYS)
            if kind=='wide-probe':
                okay=okay and expected['directories']==1 and parsed.get('flat_only') is True and parsed.get('complete') is True and parsed.get('unsupported_directories')==0
                sample['wide_complete']=parsed.get('complete');sample['wide_unsupported_directories']=parsed.get('unsupported_directories')
        else:
            if kind=='diskus':total=parse_diskus(stdout.encode('utf8'))
            elif kind=='dua-two-input':total=parse_dua_root_and_sentinel(stdout.encode('utf8'),path,variant['sentinel'])
            elif kind=='gdu':total=parse_gdu_root(stdout)
            else:raise ValueError('Unsupported output contract: '+kind)
            sample['reported_bytes']=total;sample['expected_bytes']=expected['allocated'];okay=total==expected['allocated']
        sample['matches_oracle']=bool(clean and okay)
    except (ValueError,KeyError,TypeError) as error:sample['parse_error']=str(error)
    if kind not in ('rdu','wide-probe'):
        redact_gdu_output(sample)
        sample['output_retention']='numeric root contract, original-byte hashes/counts only; competitor stdout/stderr text stripped'
    return sample

def case_valid(samples,dataset,labels,stable,binaries_unchanged):
    selected=[s for s in samples if s['dataset']==dataset and s['variant'] in labels]
    return bool(selected) and set(labels)<={s['variant'] for s in selected} and stable and binaries_unchanged and all(s['matches_oracle'] for s in selected)

def validate_schedule(config,rounds,warmups,seed):
    if config.get('campaign_contract')=='product-composition-dev-v1':
        from product_baseline_contract import validate_composition_schedule
        return validate_composition_schedule(config,rounds,warmups,seed)
    if config.get('campaign_contract')=='product-architecture-dev-v1':
        from product_baseline_contract import validate_architecture_schedule
        return validate_architecture_schedule(config,rounds,warmups,seed)
    if config.get('campaign_contract')=='product-baseline-dev-v1':
        from product_baseline_contract import validate_product_schedule
        return validate_product_schedule(config,rounds,warmups,seed)
    if (rounds,warmups,seed)!=(config['rounds'],config['warmups'],config['seed']):raise ValueError('CLI schedule differs from frozen config')
    calls=len(config['cases'])*(rounds+warmups)
    if (rounds,warmups,len(config['cases']),calls) not in ((15,1,12,192),(9,1,8,80)) or calls!=config['max_child_scan_calls']:
        raise ValueError('Only parent-frozen192-call or80-call schedules are supported')
    return calls

def validate_matrix(config,expected):
    seen=set()
    for case in config['cases']:
        key=case['dataset'],case['variant']
        if key in seen:raise ValueError('Duplicate eligible case')
        seen.add(key)
        if config['variants'].get(key[1],{}).get('require_read_only') and '--read-only' not in config['variants'][key[1]]['command']:raise ValueError('Current product command requires explicit read-only flag')
        if key[0] not in config['datasets'] or key[1] not in config['variants']:raise ValueError('Unknown case label')
        if config['variants'][key[1]].get('kind')=='wide-probe' and expected[key[0]]['directories']!=1:
            raise ValueError('Strict wide probe is ineligible for nested input')
    for contrast in config.get('contrasts',[]):
        for dataset in contrast['datasets']:
            if (dataset,contrast['reference']) not in seen or (dataset,contrast['candidate']) not in seen:raise ValueError('Unpaired contrast outside eligible matrix')

def isolated_environment(owned):
    env={k:v for k,v in os.environ.items() if not k.startswith(('DUA_','DISKUS_','GDU_','RDU_')) and k not in ('GOGC','GOMAXPROCS')}
    for key in ('HOME','XDG_CONFIG_HOME','XDG_CACHE_HOME','XDG_DATA_HOME'):
        p=owned/key.lower();p.mkdir();env[key]=str(p)
    env.update(LC_ALL='C',TERM='dumb',NO_COLOR='1',BLOCK_SIZE='1',BLOCKSIZE='1')
    return env

def run_sparse_campaign(args,config):
    from paired import summarize
    expected_calls=validate_schedule(config,args.rounds,args.warmups,args.seed)
    root=Path(__file__).resolve().parent.parent
    lock=(root/'bench/data/measurement.lock').open('a+');fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    deadline=time.monotonic()+config.get('deadline_seconds',600)
    def metadata_oracle(path):return bounded_campaign_oracle(path,deadline,config.get('campaign_contract') in ('product-baseline-dev-v1','product-architecture-dev-v1','product-composition-dev-v1'))
    lock.seek(0);lock.truncate();lock.write(json.dumps({'pid':os.getpid(),'slot':args.slot}));lock.flush()
    result={'schema':2,'experiment_slot':args.slot,'config':config,'config_sha256':hashlib.sha256(args.config.read_bytes()).hexdigest(),'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'condition':'warm; no OS cache eviction; source-generation/oracle warmed','rounds':args.rounds,'warmups':args.warmups,'seed':args.seed,'samples':[],'summaries':[],'paired_effects':[],'status':'preflight','platform':platform.platform(),'logical_cpus':os.cpu_count(),'adapter_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'measure_sha256':hashlib.sha256((root/'bench/harness.py').read_bytes()).hexdigest()}
    args.output.parent.mkdir(parents=True,exist_ok=True)
    def save():
        temp=args.output.with_suffix('.partial');temp.write_text(json.dumps(result,indent=2)+'\n');temp.replace(args.output)
    def files_ok():
        checks=[{'path':v['path'],'sha256':v['sha256']} for v in config['frozen_binaries'].values()]+config.get('source_checks',[])
        return all(Path(v['path']).is_file() and hashlib.sha256(Path(v['path']).read_bytes()).hexdigest()==v['sha256'] for v in checks)
    expected={};after={};sentinel_before=None;sentinel=None
    try:
        if config.get('environment_observations')=='bounded-uptime-pmset-v1':
            from runtime_observations import capture_runtime_observations
            result['environment_observations_before']=capture_runtime_observations(deadline)
        if not files_ok():raise ValueError('Frozen binary/source mismatch before any scan')
        result['binary_and_source_checks_before']=True
        expected={n:metadata_oracle(Path(p)) for n,p in config['datasets'].items()};result['dataset_oracles_before']=expected
        if any(o['errors'] for o in expected.values()):raise ValueError('Oracle error before campaign')
        for name,sealed in config.get('sealed_oracles',{}).items():
            if expected[name]!=sealed:raise ValueError('Sealed synthetic input changed: '+name)
        for name,sealed in config.get('required_aggregate_oracles',{}).items():
            if any(type(value) is not int or expected[name].get(key)!=value for key,value in sealed.items()):raise ValueError('Prior aggregate input contract changed: '+name)
        validate_matrix(config,expected)
        dua_variants=[v for v in config['variants'].values() if v.get('kind')=='dua-two-input']
        if dua_variants:
            if len(dua_variants)!=1:raise ValueError('One declared sentinel contract only')
            sentinel=Path(dua_variants[0]['sentinel']);sentinel_before=metadata_oracle(sentinel)
            if not sentinel.is_file() or sentinel.is_symlink() or sentinel_before!={'allocated':0,'apparent':0,'files':1,'directories':0,'hardlink_duplicates':0,'errors':0,'symlinks':0,'directory_allocated':0,'directory_apparent':0}:raise ValueError('Zero sentinel is no longer an empty regular-file contract')
            result['sentinel_oracle_before']=sentinel_before
        with tempfile.TemporaryDirectory(prefix='rdu-screen-01-env-') as temporary:
            env=isolated_environment(Path(temporary));result['config_environment_policy']='one shared empty temporary HOME/XDG tree; DUA_ DISKUS_ GDU_ RDU_ and GOGC/GOMAXPROCS cleared; explicit CPU flags; directories created outside child wall; no user configuration read by harness'
            rng=random.Random(args.seed)
            for phase,rounds in (('warmup',args.warmups),('measured',args.rounds)):
                for iteration in range(rounds):
                    order=list(config['cases']);rng.shuffle(order)
                    for case in order:
                        remaining=deadline-time.monotonic()
                        if remaining<=0:raise TimeoutError('Frozen total deadline exhausted')
                        name,label=case['dataset'],case['variant'];variant=config['variants'][label]
                        command=[token.replace('{path}',str(config['datasets'][name])) for token in variant['command']]
                        sample=measure(command,env=env,cwd=temporary,timeout=min(config.get('child_timeout_seconds',30),remaining));sample.update(dataset=name,variant=label,phase=phase,iteration=iteration)
                        validate_sample(variant,sample,expected[name],config['datasets'][name]);result['samples'].append(sample);save()
                        if not sample['matches_oracle']:raise ValueError('Correctness/exit/error/timeout failure; stopped without retry: '+name+'/'+label)
                    if not files_ok():raise ValueError('Binary or source changed during round; stopped')
                    print(json.dumps({'phase':phase,'round':iteration,'completed_calls':len(result['samples'])}),flush=True)
            if sentinel is not None:
                result['sentinel_oracle_after']=metadata_oracle(sentinel)
                if result['sentinel_oracle_after']!=sentinel_before:raise ValueError('Sentinel changed')
        result['status']='completed'
    except Exception as error:
        result['status']='stopped';result['stop_reason']=type(error).__name__+': '+str(error)
    finally:
        if expected:
            try:after={n:metadata_oracle(Path(p)) for n,p in config['datasets'].items()}
            except (OSError,TimeoutError) as error:
                after={};result['oracle_after_error']=type(error).__name__
            result['dataset_oracles_after']=after;result['dataset_stable']={n:expected[n]==after.get(n) for n in expected}
            if not all(result['dataset_stable'].values()):result['status']='stopped';result['stop_reason']='Input oracle changed; inference invalid'
        result['binary_and_source_checks_after']=files_ok()
        if not result['binary_and_source_checks_after']:result['status']='stopped';result['stop_reason']='Binary/source changed; inference invalid'
        if config.get('environment_observations')=='bounded-uptime-pmset-v1':
            from runtime_observations import capture_runtime_observations
            result['environment_observations_after']=capture_runtime_observations(deadline)
        for case in config['cases']:
            selected=[s for s in result['samples'] if s['dataset']==case['dataset'] and s['variant']==case['variant'] and s['phase']=='measured']
            if selected:result['summaries'].append({**summarize(selected),**case})
        for contrast in config['contrasts']:
            for name in contrast['datasets']:
                a,b=contrast['reference'],contrast['candidate'];groups={label:sorted([s for s in result['samples'] if s['dataset']==name and s['variant']==label and s['phase']=='measured'],key=lambda s:s['iteration']) for label in (a,b)}
                valid=result['status']=='completed' and all(len(v)==args.rounds for v in groups.values()) and case_valid(result['samples'],name,(a,b),result.get('dataset_stable',{}).get(name,False),result['binary_and_source_checks_after'])
                effect={'dataset':name,'baseline':a,'candidate':b,'valid_for_inference':valid}
                if valid:effect.update(paired_effect([s['wall_seconds'] for s in groups[a]],[s['wall_seconds'] for s in groups[b]],seed=args.seed))
                else:effect['reason']='Incomplete/stopped campaign, failed contract, changed input/binary/source; no CI'
                result['paired_effects'].append(effect)
        result['completed_calls']=len(result['samples']);save();args.output.chmod(0o444);fcntl.flock(lock,fcntl.LOCK_UN);lock.close()
    print(json.dumps({'status':result['status'],'calls':len(result['samples']),'stop_reason':result.get('stop_reason'),'paired_effects':result['paired_effects']}),flush=True)

#!/usr/bin/env python3
"""One exact256-call whole-product and retained-output final comparison."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import random
import tempfile
import time

from harness import measure
from paired import summarize
from paired_sparse import bounded_campaign_oracle, isolated_environment, validate_sample
from paired_stats import paired_effect
from product_final_contract import REAL, validate
from retained_real import build_model, validate_sample as validate_retained
from runtime_observations import capture_runtime_observations

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--slot', choices=['PRODUCT-FINAL-03'], required=True)
    args = parser.parse_args()
    if args.output.exists(): raise SystemExit('Existing final run; no retry or overwrite')
    config = json.loads(args.config.read_text())
    if config.get('prepared_only'): raise SystemExit('Final config has no execution grant')
    assert validate(config) == 256
    lock = (ROOT / 'bench/data/measurement.lock').open('a+')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    lock.seek(0); lock.truncate(); lock.write(json.dumps({'pid': os.getpid(), 'slot': args.slot})); lock.flush()
    deadline = time.monotonic() + 600
    result = {'schema': 3, 'experiment_slot': args.slot, 'config': config,
              'config_sha256': hashlib.sha256(args.config.read_bytes()).hexdigest(),
              'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'status': 'preflight', 'rounds': 7, 'warmups': 1, 'seed': config['seed'],
              'condition': 'Warm after creation/preflight and independent metadata oracles; no OS eviction',
              'samples': [], 'summaries': [], 'paired_effects': []}
    def save():
        temporary = args.output.with_suffix('.partial'); temporary.write_text(json.dumps(result, indent=2) + '\n'); temporary.replace(args.output)
    def files_ok():
        records = list(config['frozen_binaries'].values()) + config['source_checks']
        return all(Path(item['path']).is_file() and hashlib.sha256(Path(item['path']).read_bytes()).hexdigest() == item['sha256'] for item in records)
    def oracle(path): return bounded_campaign_oracle(Path(path), deadline, True)
    expected = {}; models = {}; before_models = {}
    save()
    try:
        result['environment_observations_before'] = capture_runtime_observations(deadline)
        if not files_ok(): raise ValueError('Frozen binary/source mismatch before scans')
        result['binary_and_source_checks_before'] = True
        result['root_identities_before'] = {}
        for name, path in config['datasets'].items():
            metadata = Path(path).lstat()
            result['root_identities_before'][name] = [metadata.st_dev, metadata.st_ino]
        expected = {name: oracle(path) for name, path in config['datasets'].items()}
        result['dataset_oracles_before'] = expected
        if expected != config['sealed_oracles'] or any(item['errors'] for item in expected.values()):
            raise ValueError('Sealed aggregate oracle gate failed')
        for name in REAL:
            models[name], before_models[name] = build_model(config['datasets'][name], deadline)
            if models[name]['totals'] != expected[name] or before_models[name] != config['sealed_retained_models'][name]:
                raise ValueError('Sealed real retained model/coverage changed')
        result['retained_models_before'] = before_models
        sentinel = Path(config['sentinel']); sentinel_before = oracle(sentinel)
        sentinel_stat = sentinel.lstat(); sentinel_identity = [sentinel_stat.st_dev, sentinel_stat.st_ino]
        if not sentinel.is_file() or sentinel.is_symlink() or sentinel_before != config['sealed_sentinel_oracle']:
            raise ValueError('Owned zero sentinel contract changed')
        result.update(sentinel_before=sentinel_before, sentinel_identity_before=sentinel_identity)
        with tempfile.TemporaryDirectory(prefix='rdu-final-env-', dir='/tmp') as temporary:
            env = isolated_environment(Path(temporary))
            rng = random.Random(config['seed'])
            for phase, rounds in (('warmup', 1), ('measured', 7)):
                for iteration in range(rounds):
                    order = list(config['cases']); rng.shuffle(order)
                    for case in order:
                        remaining = deadline - time.monotonic()
                        if remaining <= 0: raise TimeoutError('Final total deadline')
                        name, label = case['dataset'], case['variant']; variant = config['variants'][label]
                        command = [token.replace('{path}', config['datasets'][name]) for token in variant['command']]
                        sample = measure(command, env=env, cwd=temporary, timeout=min(20, remaining))
                        sample.update(dataset=name, variant=label, phase=phase, iteration=iteration)
                        if variant['kind'].startswith('retained-'):
                            validate_retained(sample, variant, models[name], config['datasets'][name], config['sentinel'])
                        else:
                            validate_sample(variant, sample, expected[name], config['datasets'][name])
                        result['samples'].append(sample); save()
                        if not sample['matches_oracle']: raise ValueError('Defining sample process/accounting/retained-output contract failed; no retry')
                    if not files_ok(): raise ValueError('Bound binary/source changed during round')
                    print(json.dumps({'phase': phase, 'round': iteration, 'completed_calls': len(result['samples'])}), flush=True)
            result['sentinel_after'] = oracle(sentinel)
            sentinel_stat = sentinel.lstat(); result['sentinel_identity_after'] = [sentinel_stat.st_dev, sentinel_stat.st_ino]
            if result['sentinel_after'] != sentinel_before or result['sentinel_identity_after'] != sentinel_identity:
                raise ValueError('Zero sentinel changed')
        result['status'] = 'completed'
    except Exception as error:
        result['status'] = 'stopped'
        # Preserve defining failure without serializing a real path from OSError.
        result['stop_reason'] = type(error).__name__ + ': final defining gate failed; see retained path-free sample contracts; no retry'
    finally:
        if expected:
            try:
                after = {name: oracle(path) for name, path in config['datasets'].items()}
                result['dataset_oracles_after'] = after
                result['dataset_stable'] = {name: expected[name] == after.get(name) for name in expected}
                result['root_identities_after'] = {}
                for name, path in config['datasets'].items():
                    metadata = Path(path).lstat()
                    result['root_identities_after'][name] = [metadata.st_dev, metadata.st_ino]
                result['root_identities_stable'] = result['root_identities_before'] == result['root_identities_after']
                after_models = {}
                for name in REAL:
                    _, after_models[name] = build_model(config['datasets'][name], deadline)
                result['retained_models_after'] = after_models
                result['retained_models_stable'] = before_models == after_models
                if not all(result['dataset_stable'].values()) or not result['retained_models_stable'] or not result['root_identities_stable']:
                    result['status'] = 'stopped'; result['stop_reason'] = 'Metadata inputs changed; all inference invalid'
            except Exception as error:
                result['status'] = 'stopped'; result['after_oracle_error'] = type(error).__name__
        result['binary_and_source_checks_after'] = files_ok()
        if not result['binary_and_source_checks_after']:
            result['status'] = 'stopped'; result['stop_reason'] = 'Bound binary/source changed; all inference invalid'
        result['environment_observations_after'] = capture_runtime_observations(deadline)
        for case in config['cases']:
            selected = [sample for sample in result['samples'] if sample['phase'] == 'measured'
                        and (sample['dataset'], sample['variant']) == (case['dataset'], case['variant'])]
            if selected: result['summaries'].append({**summarize(selected), **case})
        for contrast in config['contrasts']:
            a, b = contrast['reference'], contrast['candidate']
            for name in contrast['datasets']:
                groups = {label: sorted([sample for sample in result['samples'] if sample['phase'] == 'measured'
                                        and sample['dataset'] == name and sample['variant'] == label], key=lambda sample: sample['iteration']) for label in (a, b)}
                valid = (result['status'] == 'completed' and len(result['samples']) == 256
                         and all(len(group) == 7 for group in groups.values())
                         and all(sample['matches_oracle'] for sample in result['samples']))
                effect = {'dataset': name, 'baseline': a, 'candidate': b, 'valid_for_inference': valid}
                if valid:
                    effect.update(paired_effect([sample['wall_seconds'] for sample in groups[a]],
                                                [sample['wall_seconds'] for sample in groups[b]], seed=config['seed']))
                result['paired_effects'].append(effect)
        result['completed_calls'] = len(result['samples']); save(); args.output.chmod(0o444)
        fcntl.flock(lock, fcntl.LOCK_UN); lock.close()
    print(json.dumps({'status': result['status'], 'calls': result['completed_calls'], 'stop_reason': result.get('stop_reason')}), flush=True)


if __name__ == '__main__': main()

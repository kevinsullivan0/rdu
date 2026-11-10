#!/usr/bin/env python3
"""Fixed-plan round-paired scanner comparison; acquire the parent's slot first."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import random
import statistics
import time

from harness import environment, measure, oracle
from paired_stats import paired_effect
from output_policy import parse_gdu_root, redact_gdu_output

HERE = Path(__file__).resolve().parent


def case_can_infer(samples, name, labels, stable, binaries_unchanged):
    selected = [s for s in samples if s['dataset'] == name and s['variant'] in labels]
    return bool(selected) and stable and binaries_unchanged and all(s['matches_oracle'] for s in selected)


def summarize(samples):
    times = [sample['wall_seconds'] for sample in samples]
    summary = {'n': len(times), 'median_seconds': statistics.median(times),
            'min_seconds': min(times), 'max_seconds': max(times),
            'pstdev_seconds': statistics.pstdev(times),
            'median_peak_rss_bytes': statistics.median(s['peak_rss_bytes'] for s in samples),
            'max_peak_rss_bytes': max(s['peak_rss_bytes'] for s in samples),
            'median_user_seconds': statistics.median(s['user_seconds'] for s in samples),
            'median_system_seconds': statistics.median(s['system_seconds'] for s in samples),
            'all_correct': all(s.get('matches_oracle', False) for s in samples),
            'failed_oracle_samples': sum(not s.get('matches_oracle', False) for s in samples)}
    for key in ('stdout_bytes', 'stdout_lines', 'stderr_bytes', 'stderr_lines'):
        if all(key in sample for sample in samples):
            values = [sample[key] for sample in samples]
            summary['median_' + key] = statistics.median(values)
            summary['min_' + key] = min(values)
            summary['max_' + key] = max(values)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--slot', required=True, help='parent-granted serialized experiment identifier')
    parser.add_argument('--rounds', type=int, default=15)
    parser.add_argument('--warmups', type=int, default=2)
    parser.add_argument('--seed', type=int, default=20261109)
    args = parser.parse_args()
    if args.rounds < 7 or args.warmups < 1:
        parser.error('use at least seven measured rounds and one warmup; seven is opt-in product development only, legacy matrices require fifteen')
    if args.output.exists():
        parser.error('refusing to overwrite an existing run bundle')
    config = json.loads(args.config.read_text())
    if config.get('prepared_only'):
        parser.error('draft preparation plan: materialize final commands/hashes after source freeze and a parent slot grant')
    if 'cases' in config:
        from paired_sparse import run_sparse_campaign
        run_sparse_campaign(args, config)
        return
    if args.rounds < 15:
        parser.error('legacy full matrices require at least fifteen measured paired rounds')
    labels = [variant['label'] for variant in config['variants']]
    if len(set(labels)) != len(labels) or config['baseline'] not in labels:
        parser.error('variant labels must be unique and baseline must name one')
    datasets = {label: Path(path).expanduser().resolve(strict=True) for label, path in config['datasets'].items()}
    (HERE / 'data').mkdir(exist_ok=True)
    lock = (HERE / 'data/measurement.lock').open('a+')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        parser.error('another benchmark or profile holds the measurement lock')
    lock.seek(0); lock.truncate(); lock.write(json.dumps({'pid': os.getpid(), 'slot': args.slot}) + '\n'); lock.flush()
    variants = config['variants']
    frozen = {}
    for variant in variants:
        path = Path(variant['command'][0]).resolve(strict=True)
        frozen[variant['label']] = {'binary_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                                    'binary_bytes': path.stat().st_size, 'command': variant['command'],
                                    'kind': variant.get('kind', 'rdu'),
                                    'output_contract': variant.get('output_contract', 'single JSON summary' if variant.get('kind', 'rdu') == 'rdu' else 'root numeric total is first stdout row')}
    expected = {label: oracle(path) for label, path in datasets.items()}
    result = {'schema': 1, 'experiment_slot': args.slot, 'question': config['question'],
              'hypotheses': config.get('hypotheses', []), 'baseline': config['baseline'],
              'inference_status': config.get('inference_status'),
              'comparability_notes': config.get('comparability_notes', []),
              'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'output_policy_sha256': hashlib.sha256((HERE / 'output_policy.py').read_bytes()).hexdigest(),
              'gdu_output_policy': 'parse first root row then retain numeric total, original byte/line counts and hashes only; discard path-bearing stdout/stderr before saving',
              'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'rounds': args.rounds, 'warmups': args.warmups, 'seed': args.seed,
              'variants': frozen, 'dataset_oracles_before': expected,
              'environment': environment(), 'samples': [], 'summaries': [], 'paired_effects': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    def save():
        temporary = args.output.with_suffix(args.output.suffix + '.partial')
        temporary.write_text(json.dumps(result, indent=2) + '\n')
        temporary.replace(args.output)
    save()
    rng = random.Random(args.seed)
    for phase, rounds in [('warmup', args.warmups), ('measured', args.rounds)]:
        for iteration in range(rounds):
            names = list(datasets)
            rng.shuffle(names)
            for name in names:
                order = variants.copy()
                rng.shuffle(order)
                for variant in order:
                    command = [token.replace('{path}', str(datasets[name])) for token in variant['command']]
                    sample = measure(command)
                    sample.update(dataset=name, variant=variant['label'], phase=phase, iteration=iteration)
                    sample['matches_oracle'] = False
                    try:
                        if sample['returncode'] == 0:
                            if variant.get('kind', 'rdu') == 'gdu':
                                metric = variant.get('metric', 'allocated')
                                wanted = expected[name][metric]
                                if metric == 'apparent':
                                    wanted -= expected[name]['directory_apparent']
                                sample.update(reported_bytes=parse_gdu_root(sample['stdout']), expected_bytes=wanted)
                                sample['matches_oracle'] = sample['reported_bytes'] == wanted
                            else:
                                parsed = json.loads(sample['stdout'])
                                totals = parsed.get('totals', parsed)
                                sample['totals'] = totals
                                sample['matches_oracle'] = all(totals[key] == expected[name][key] for key in
                                                               ('allocated', 'apparent', 'files', 'directories', 'errors'))
                    except (ValueError, KeyError, TypeError) as error:
                        sample['parse_error'] = str(error)
                    if variant.get('kind', 'rdu') == 'gdu':
                        redact_gdu_output(sample)
                    result['samples'].append(sample)
                save()
    after = {label: oracle(path) for label, path in datasets.items()}
    result['dataset_oracles_after'] = after
    result['dataset_stable'] = {name: expected[name] == after[name] for name in datasets}
    result['binaries_unchanged'] = all(hashlib.sha256(Path(v['command'][0]).resolve().read_bytes()).hexdigest()
                                       == frozen[v['label']]['binary_sha256'] for v in variants)
    for name in datasets:
        grouped = {}
        for variant in labels:
            selected = sorted([s for s in result['samples'] if s['phase'] == 'measured' and s['dataset'] == name
                               and s['variant'] == variant], key=lambda sample: sample['iteration'])
            grouped[variant] = selected
            summary = summarize(selected)
            summary.update(dataset=name, variant=variant)
            result['summaries'].append(summary)
        baseline = grouped[config['baseline']]
        for label in labels:
            if label == config['baseline']:
                continue
            candidate = grouped[label]
            # A failed warmup, measured oracle, unstable input or changed binary
            # invalidates the complete case; never cherry-pick valid pairs.
            accepted = case_can_infer(result['samples'], name, (label, config['baseline']),
                                      result['dataset_stable'][name], result['binaries_unchanged'])
            effect = {'dataset': name, 'baseline': config['baseline'], 'candidate': label,
                      'valid_for_inference': accepted}
            if accepted:
                effect.update(paired_effect([s['wall_seconds'] for s in baseline], [s['wall_seconds'] for s in candidate], seed=args.seed))
            else:
                effect['reason'] = 'oracle failure, changed metadata input or changed binary; all samples retained and no CI claimed'
            result['paired_effects'].append(effect)
    save()
    args.output.chmod(0o444)
    print(json.dumps({'summaries': result['summaries'], 'paired_effects': result['paired_effects']}, indent=2))


if __name__ == '__main__':
    main()

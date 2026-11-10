"""Statistics for fixed, correctness-checked, round-paired observations."""
import math
import random
import statistics


def paired_effect(baseline, candidate, seed=20261109, bootstraps=10000):
    if len(baseline) != len(candidate) or not baseline:
        raise ValueError('paired observations must have equal nonzero length')
    if any(not math.isfinite(value) or value <= 0 for value in baseline + candidate):
        raise ValueError('wall times must be positive')
    logs = [math.log(a / b) for a, b in zip(baseline, candidate)]
    deltas = [a - b for a, b in zip(baseline, candidate)]
    rng = random.Random(seed)
    estimates = []
    differences = []
    for _ in range(bootstraps):
        indexes = [rng.randrange(len(logs)) for _ in logs]
        estimates.append(math.exp(statistics.median(logs[i] for i in indexes)))
        differences.append(statistics.median(deltas[i] for i in indexes))
    estimates.sort()
    differences.sort()
    lower = int(.025 * (bootstraps - 1))
    upper = int(.975 * (bootstraps - 1))
    return {'paired_n': len(logs), 'median_paired_speedup': math.exp(statistics.median(logs)),
            'speedup_bootstrap_95_percentile_ci': [estimates[lower], estimates[upper]],
            'median_paired_saved_seconds': statistics.median(deltas),
            'saved_seconds_bootstrap_95_percentile_ci': [differences[lower], differences[upper]],
            'candidate_faster_rounds': sum(b < a for a, b in zip(baseline, candidate)),
            'bootstrap_resamples': bootstraps, 'seed': seed,
            'caution': 'round pairing mitigates drift; warm local results do not prove an optimum or remove serial-order effects'}

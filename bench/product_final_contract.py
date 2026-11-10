"""The single parent-selected bounded final product matrix."""
INPUTS = ('cached-crates', 'full-sdk', 'skew', 'uneven')
SCALAR = ('current', 'selected', 'diskus-default', 'diskus8', 'dua-default', 'dua8', 'gdu-full16')
RETAINED = ('retained-rdu', 'retained-dua')
REAL = INPUTS[:2]


def validate(config):
    expected = {(name, label) for name in INPUTS for label in SCALAR}
    expected |= {(name, label) for name in REAL for label in RETAINED}
    cases = [(case['dataset'], case['variant']) for case in config['cases']]
    if (config.get('campaign_contract') != 'product-final-bounded-v1'
            or (config['rounds'], config['warmups'], config['seed']) != (7, 1, 2026111057)
            or len(cases) != 32 or set(cases) != expected or config['max_child_scan_calls'] != 256
            or set(config['datasets']) != set(INPUTS) or set(config['variants']) != set(SCALAR + RETAINED)
            or config['child_timeout_seconds'] != 20 or config['deadline_seconds'] != 600
            or config['retained_output_bound_bytes'] != 33554432
            or config['retained_depth_limit'] != 511
            or config['environment_observations'] != 'bounded-uptime-pmset-v1'):
        raise ValueError('Not the exact parent-selected final256 product contract')
    variants = config['variants']
    for variant in variants.values():
        binary = config['frozen_binaries'].get(variant['binary'])
        if binary is None or variant['command'][0] != binary['path']:
            raise ValueError('Every command must use its exact frozen executable')
    summary = ['--summary', '--read-only', '--backend', 'bulk', '--threads', '8', '{path}']
    for label in ('current', 'selected'):
        if variants[label]['kind'] != 'rdu' or variants[label]['command'][1:] != summary:
            raise ValueError('Final full retained read-only product command required')
    for label in ('diskus-default', 'diskus8'):
        expected_command = (['--threads', '8'] if label == 'diskus8' else []) + ['--directories', 'included', '{path}']
        if variants[label]['kind'] != 'diskus' or variants[label]['command'][1:] != expected_command:
            raise ValueError('Fixed stock Diskus policy command required')
    sentinel = config['sentinel']
    for label in ('dua-default', 'dua8'):
        expected_command = ['aggregate'] + (['--threads', '8'] if label == 'dua8' else [])
        expected_command += ['--format', 'bytes', '--no-sort', '--no-total', '{path}', sentinel]
        if (variants[label]['kind'] != 'dua-two-input' or variants[label]['command'][1:] != expected_command
                or variants[label]['sentinel'] != sentinel):
            raise ValueError('Both stock dua scalar policies require the same strict root+sentinel contract')
    if variants['retained-rdu']['kind'] != 'retained-rdu' or variants['retained-rdu']['command'][1:] != ['--non-interactive', '--read-only', '--backend', 'bulk', '--threads', '8', '--reverse', '--bytes', '--depth', '512', '{path}']:
        raise ValueError('Selected full retained ascending RDU output command required')
    if variants['retained-dua']['kind'] != 'retained-dua' or variants['retained-dua']['command'][1:] != ['aggregate', '--threads', '8', '--format', 'bytes', '--no-total', '--depth', '512', '{path}', sentinel]:
        raise ValueError('Sorted full retained stock dua8 output command required')
    if variants['retained-rdu']['command'][0] != variants['selected']['command'][0]:
        raise ValueError('Retained RDU must be the selected actual product binary')
    if variants['gdu-full16']['kind'] != 'gdu' or variants['gdu-full16']['command'][1:] != ['-nps', '--no-prefix', '--config-file', config['gdu_empty_config'], '--max-cores', '16', '--depth', '1', '{path}']:
        raise ValueError('Correct stock GDU full16 mode required')
    expected_contrasts = {(label, 'selected', INPUTS) for label in SCALAR if label != 'selected'}
    expected_contrasts.add(('retained-dua', 'retained-rdu', REAL))
    contrasts = [(item['reference'], item['candidate'], tuple(item['datasets'])) for item in config['contrasts']]
    if len(contrasts) != 7 or set(contrasts) != expected_contrasts:
        raise ValueError('Keep every selected-versus-reference and real retained contrast')
    return 256

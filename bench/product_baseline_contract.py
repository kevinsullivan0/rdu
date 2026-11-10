"""Opt-in seven-pair development contract; historical schedules remain strict."""
MAIN = ('cached-crates', 'full-sdk', 'flat-long')
EXTRA = ('dense', 'chain', 'burst')
LABELS = ('current', 'diskus-default', 'diskus8', 'dua8', 'gdu-seq', 'gdu-full16')
ARCH_INPUTS = ('cached-crates', 'full-sdk', 'flat-long', 'dense', 'chain')


def validate_product_schedule(config, rounds, warmups, seed):
    expected = {(name, label) for name in MAIN for label in LABELS}
    expected |= {(name, 'current') for name in EXTRA}
    cases = [(case['dataset'], case['variant']) for case in config['cases']]
    if (config.get('campaign_contract') != 'product-baseline-dev-v1'
            or (rounds, warmups) != (7, 1)
            or (rounds, warmups, seed) != (config['rounds'], config['warmups'], config['seed'])
            or len(cases) != 21 or set(cases) != expected
            or config['max_child_scan_calls'] != 168
            or set(config['variants']) != set(LABELS)
            or config.get('child_timeout_seconds') != 20
            or config.get('deadline_seconds') != 600):
        raise ValueError('Not the frozen 168-call whole-product development contract')
    current = config['variants']['current']
    if (current.get('kind') != 'rdu' or not current.get('require_read_only')
            or current['command'][1:] != ['--summary', '--read-only', '--backend',
                                          'bulk', '--threads', '8', '{path}']):
        raise ValueError('Whole-product baseline requires actual complete read-only RDU command')
    if config['variants']['dua8'].get('kind') != 'dua-two-input':
        raise ValueError('Exact dua two-input contract required')
    if not config.get('current_product_receipt'):
        raise ValueError('Current product freeze receipt required')
    return 168


def validate_architecture_schedule(config, rounds, warmups, seed):
    selected = config.get('selected_candidates')
    if selected not in (['A'], ['B'], ['A', 'B']):
        raise ValueError('Select one or two separate consequential architectures')
    labels = ['control', *selected]
    cases = [(case['dataset'], case['variant']) for case in config['cases']]
    expected = {(name, label) for name in ARCH_INPUTS for label in labels}
    calls = len(expected) * 8
    if (config.get('campaign_contract') != 'product-architecture-dev-v1'
            or (rounds, warmups) != (7, 1)
            or (rounds, warmups, seed) != (config['rounds'], config['warmups'], config['seed'])
            or len(cases) != len(expected) or set(cases) != expected
            or config['max_child_scan_calls'] != calls or calls not in (80, 120)
            or set(config['variants']) != set(labels)
            or set(config['datasets']) != set(ARCH_INPUTS)
            or config.get('child_timeout_seconds') != 20
            or config.get('deadline_seconds') != 600):
        raise ValueError('Not the selected 80/120-call architecture development contract')
    if not config.get('current_product_receipt'):
        raise ValueError('Current integrated source freeze must be preserved')
    receipts = config.get('matched_build_receipts', {})
    if set(receipts) != set(labels):
        raise ValueError('Missing matched normal-release build receipts')
    for label in labels:
        variant = config['variants'][label]
        if (variant.get('kind') != 'rdu' or not variant.get('require_read_only')
                or variant['command'][1:] != ['--summary', '--read-only', '--backend',
                                              'bulk', '--threads', '8', '{path}']):
            raise ValueError('Architecture must retain full current read-only product command')
        receipt = receipts[label]
        if (receipt.get('base_source_commit') != config['current_product_receipt']['source_commit']
                or receipt.get('same_build_recipe_verified') is not True
                or receipt.get('complete_retained_tree_tests_passed') is not True
                or receipt.get('source_delta_reviewed') is not True
                or receipt.get('normal_release_features') is not True):
            raise ValueError('Matched build/retained-product evidence is incomplete')
    return calls


def validate_composition_schedule(config,rounds,warmups,seed):
    labels=('B','AB');cases=[(c['dataset'],c['variant']) for c in config['cases']]
    expected={(name,label) for name in ARCH_INPUTS for label in labels}
    if (config.get('campaign_contract')!='product-composition-dev-v1'
            or (rounds,warmups)!=(7,1)
            or (rounds,warmups,seed)!=(config['rounds'],config['warmups'],config['seed'])
            or len(cases)!=10 or set(cases)!=expected or config['max_child_scan_calls']!=80
            or set(config['variants'])!=set(labels) or set(config['datasets'])!=set(ARCH_INPUTS)
            or config.get('child_timeout_seconds')!=20 or config.get('deadline_seconds')!=600
            or config.get('environment_observations')!='bounded-uptime-pmset-v1'
            or config.get('auxiliary_observer_calls_max')!=6):
        raise ValueError('Not the frozen80-call B versusAB composition contract')
    if not config.get('matched_base_source_commit') or not config.get('current_product_receipt'):
        raise ValueError('B matched-base and current-product provenance required')
    receipts=config.get('matched_build_receipts',{})
    if set(receipts)!=set(labels):raise ValueError('Missing matched composition build receipts')
    for label in labels:
        variant=config['variants'][label];receipt=receipts[label]
        if (variant.get('kind')!='rdu' or variant.get('require_read_only') is not True
                or variant['command'][1:]!=['--summary','--read-only','--backend','bulk','--threads','8','{path}']):
            raise ValueError('Composition must preserve complete read-only retained product')
        if (receipt.get('base_source_commit')!=config['matched_base_source_commit']
                or any(receipt.get(key) is not True for key in ('same_build_recipe_verified','complete_retained_tree_tests_passed','source_delta_reviewed','normal_release_features'))):
            raise ValueError('Composition source/build/full-product evidence incomplete')
    return 80

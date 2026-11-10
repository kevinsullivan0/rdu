import copy
import unittest
from product_baseline_contract import MAIN, EXTRA, LABELS, validate_product_schedule
from product_baseline_contract import ARCH_INPUTS, validate_architecture_schedule, validate_composition_schedule
from paired_sparse import validate_schedule
from paired_sparse import bounded_campaign_oracle
from unittest.mock import patch
import time


def example():
    return {'campaign_contract': 'product-baseline-dev-v1', 'rounds': 7, 'warmups': 1,
            'seed': 2026111031, 'max_child_scan_calls': 168,
            'child_timeout_seconds': 20, 'deadline_seconds': 600,
            'current_product_receipt': {'sha256': 'example'},
            'variants': {label: ({'kind': 'rdu', 'require_read_only': True,
                                  'command': ['binary', '--summary', '--read-only', '--backend',
                                              'bulk', '--threads', '8', '{path}']}
                                 if label == 'current' else
                                 {'kind': 'dua-two-input' if label == 'dua8' else 'other'})
                         for label in LABELS},
            'cases': [{'dataset': name, 'variant': label} for name in MAIN for label in LABELS]
                     + [{'dataset': name, 'variant': 'current'} for name in EXTRA]}


class ProductScheduleTests(unittest.TestCase):
    def test_exact_development_matrix(self):
        self.assertEqual(validate_schedule(example(), 7, 1, 2026111031), 168)

    def test_more_calls_reordered_read_only_flags_or_missing_receipt_fail(self):
        for field, value in [('rounds', 8), ('max_child_scan_calls', 169),
                             ('deadline_seconds', 601), ('child_timeout_seconds', 21),
                             ('current_product_receipt', None)]:
            bad = example(); bad[field] = value
            with self.assertRaises(ValueError): validate_product_schedule(bad, 7, 1, 2026111031)
        bad = example(); bad['variants']['current']['command'].remove('--read-only')
        with self.assertRaises(ValueError): validate_product_schedule(bad, 7, 1, 2026111031)

    def test_missing_duplicate_or_extra_case_fails(self):
        for change in ('missing', 'duplicate', 'extra'):
            bad = example()
            if change == 'missing': bad['cases'].pop()
            elif change == 'duplicate': bad['cases'][-1] = copy.deepcopy(bad['cases'][0])
            else: bad['cases'].append({'dataset': 'reserve', 'variant': 'current'})
            with self.assertRaises(ValueError): validate_product_schedule(bad, 7, 1, 2026111031)

    def test_old_screens_do_not_gain_seven_pair_permission(self):
        bad = example(); bad.pop('campaign_contract')
        with self.assertRaises(ValueError): validate_schedule(bad, 7, 1, 2026111031)

    def test_expired_oracle_deadline_does_not_traverse(self):
        with patch('paired_sparse.oracle') as scan:
            with self.assertRaises(TimeoutError): bounded_campaign_oracle('unused',time.monotonic()-1,True)
            scan.assert_not_called()

    def test_metadata_oracle_keeps_exact_contract_without_running_a_scan(self):
        with patch('paired_sparse.oracle',return_value={'errors':0}) as scan:
            self.assertEqual(bounded_campaign_oracle('unused',time.monotonic()+1,True),{'errors':0})
            scan.assert_called_once_with('unused')


class ArchitectureScheduleTests(unittest.TestCase):
    def config(self, selected=('A', 'B')):
        labels=['control',*selected]
        return {'campaign_contract':'product-architecture-dev-v1','rounds':7,'warmups':1,'seed':2026111037,
                'selected_candidates':list(selected),'max_child_scan_calls':len(labels)*40,
                'child_timeout_seconds':20,'deadline_seconds':600,
                'datasets':{name:name for name in ARCH_INPUTS},
                'current_product_receipt':{'source_commit':'base'},
                'matched_build_receipts':{label:{'base_source_commit':'base','same_build_recipe_verified':True,
                                                'complete_retained_tree_tests_passed':True,'source_delta_reviewed':True,
                                                'normal_release_features':True} for label in labels},
                'variants':{label:{'kind':'rdu','require_read_only':True,
                                    'command':['binary','--summary','--read-only','--backend','bulk','--threads','8','{path}']}
                            for label in labels},
                'cases':[{'dataset':name,'variant':label} for name in ARCH_INPUTS for label in labels]}

    def test_exact_two_and_three_binary_recipes(self):
        for selected,calls in [(('A',),80),(('B',),80),(('A','B'),120)]:
            config=self.config(selected)
            self.assertEqual(validate_schedule(config,7,1,2026111037),calls)

    def test_missing_receipt_retained_tree_or_matching_base_fails(self):
        for field,value in [('base_source_commit','historical'),('same_build_recipe_verified',False),
                            ('complete_retained_tree_tests_passed',False),('source_delta_reviewed',False),
                            ('normal_release_features',False),('normal_release_features','false')]:
            bad=self.config();bad['matched_build_receipts']['A'][field]=value
            with self.assertRaises(ValueError):validate_architecture_schedule(bad,7,1,2026111037)
        bad=self.config();bad['matched_build_receipts'].pop('B')
        with self.assertRaises(ValueError):validate_architecture_schedule(bad,7,1,2026111037)

    def test_extra_cases_scalar_mode_budget_or_combination_fails(self):
        for mutate in [lambda c:c['cases'].append(c['cases'][0]),
                       lambda c:c.update(max_child_scan_calls=121),
                       lambda c:c.update(selected_candidates=['A+B']),
                       lambda c:c['variants']['A'].update(kind='diskus')]:
            bad=self.config();mutate(bad)
            with self.assertRaises(ValueError):validate_architecture_schedule(bad,7,1,2026111037)


class CompositionScheduleTests(unittest.TestCase):
    def config(self):
        config=ArchitectureScheduleTests().config(('B',))
        config['campaign_contract']='product-composition-dev-v1'
        config['seed']=2026111041
        config['variants']={'B':config['variants']['control'],'AB':config['variants']['B']}
        config['cases']=[{'dataset':name,'variant':label} for name in ARCH_INPUTS for label in ('B','AB')]
        config['matched_base_source_commit']='standalone-B'
        config['matched_build_receipts']={label:{**config['matched_build_receipts']['B'],'base_source_commit':'standalone-B'} for label in ('B','AB')}
        config['environment_observations']='bounded-uptime-pmset-v1'
        config['auxiliary_observer_calls_max']=6
        return config

    def test_exact_composition_selects_matched_B_reference(self):
        self.assertEqual(validate_schedule(self.config(),7,1,2026111041),80)

    def test_wrong_original_control_extra_case_budget_or_missing_observations_fail(self):
        for mutate in [lambda c:c['matched_build_receipts']['B'].update(base_source_commit='original'),
                       lambda c:c.update(max_child_scan_calls=120),
                       lambda c:c['cases'].append(c['cases'][0]),
                       lambda c:c.update(environment_observations=None),
                       lambda c:c.update(auxiliary_observer_calls_max=7)]:
            config=self.config();mutate(config)
            with self.assertRaises(ValueError):validate_composition_schedule(config,7,1,2026111041)
        config=self.config();config['matched_build_receipts'].pop('AB')
        with self.assertRaises(ValueError):validate_composition_schedule(config,7,1,2026111041)

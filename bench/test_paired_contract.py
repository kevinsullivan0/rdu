import unittest
from paired import summarize, case_can_infer


class PairedContractTests(unittest.TestCase):
    def test_failed_warmup_invalidates_entire_case(self):
        samples = [{'dataset': 'tree', 'variant': 'baseline', 'phase': 'warmup', 'matches_oracle': False},
                   {'dataset': 'tree', 'variant': 'candidate', 'phase': 'measured', 'matches_oracle': True}]
        self.assertFalse(case_can_infer(samples, 'tree', ('baseline', 'candidate'), True, True))

    def test_changed_inputs_invalidate_otherwise_correct_case(self):
        samples = [{'dataset': 'tree', 'variant': 'baseline', 'matches_oracle': True},
                   {'dataset': 'tree', 'variant': 'candidate', 'matches_oracle': True}]
        self.assertFalse(case_can_infer(samples, 'tree', ('baseline', 'candidate'), False, True))
        self.assertFalse(case_can_infer(samples, 'tree', ('baseline', 'candidate'), True, False))

    def test_failure_is_retained_in_descriptive_summary(self):
        samples = [{'wall_seconds': n, 'peak_rss_bytes': 100, 'user_seconds': .01,
                    'system_seconds': .02, 'matches_oracle': n != 2} for n in [1., 2., 3.]]
        observed = summarize(samples)
        self.assertEqual(observed['n'], 3)
        self.assertEqual(observed['median_seconds'], 2.)
        self.assertEqual(observed['failed_oracle_samples'], 1)


if __name__ == '__main__':
    unittest.main()

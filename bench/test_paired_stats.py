import unittest
from paired_stats import paired_effect


class PairedStatisticsTests(unittest.TestCase):
    def test_constant_twofold_gain_is_not_lost_to_shared_round_drift(self):
        # Catch using unpaired medians/ratios or reversing the effect direction.
        observed = paired_effect([2., 4., 10., 20.], [1., 2., 5., 10.], bootstraps=500)
        self.assertEqual(observed['median_paired_speedup'], 2.)
        self.assertEqual(observed['speedup_bootstrap_95_percentile_ci'], [2., 2.])
        self.assertEqual(observed['candidate_faster_rounds'], 4)

    def test_mismatched_pairs_are_rejected(self):
        with self.assertRaises(ValueError):
            paired_effect([1.], [1., 2.])

    def test_pairing_matters_when_round_order_and_durations_change(self):
        observed = paired_effect([1., 100., 1000.], [100., .5, 500.], bootstraps=500)
        self.assertEqual(observed['median_paired_speedup'], 2.)

    def test_nonfinite_time_is_rejected(self):
        with self.assertRaises(ValueError):
            paired_effect([1.], [float('nan')])

    def test_zero_time_is_rejected(self):
        with self.assertRaises(ValueError):
            paired_effect([1.], [0.])


if __name__ == '__main__':
    unittest.main()

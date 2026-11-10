import subprocess
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from runtime_observations import capture_runtime_observations


class RuntimeObservationTests(unittest.TestCase):
    def test_fixed_read_only_commands_and_small_retention(self):
        with patch('runtime_observations.time.monotonic',return_value=10),patch('runtime_observations.subprocess.run',return_value=SimpleNamespace(returncode=0,stdout=b'ok',stderr=b'')) as child:
            result=capture_runtime_observations(30)
            self.assertEqual(child.call_count,3)
            self.assertEqual([c.args[0] for c in child.call_args_list],[['/usr/bin/uptime'],['/usr/bin/pmset','-g','batt'],['/usr/bin/pmset','-g','therm']])
            self.assertTrue(all(c.kwargs['timeout']==2 for c in child.call_args_list))
            self.assertTrue(all(x['status']=='observed' for x in result['observations'].values()))

    def test_absence_timeout_and_oversize_are_unknown_not_clean_or_inferred(self):
        with patch('runtime_observations.time.monotonic',return_value=10),patch('runtime_observations.subprocess.run',side_effect=[FileNotFoundError(),subprocess.TimeoutExpired('observer',2),SimpleNamespace(returncode=0,stdout=b'x'*5000,stderr=b'')]):
            result=capture_runtime_observations(30)
            self.assertTrue(all(x['status']=='unknown' for x in result['observations'].values()))
            self.assertEqual(len(result['observations']['thermal_status']['stdout']),4096)

    def test_expired_total_deadline_launches_nothing(self):
        with patch('runtime_observations.time.monotonic',return_value=10),patch('runtime_observations.subprocess.run') as child:
            result=capture_runtime_observations(9)
            child.assert_not_called()
            self.assertTrue(all(x['status']=='unknown' for x in result['observations'].values()))

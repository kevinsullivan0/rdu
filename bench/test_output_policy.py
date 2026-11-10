import hashlib
import json
import unittest

from output_policy import parse_gdu_root, redact_gdu_output


class OutputPolicyTests(unittest.TestCase):
    def test_root_first_total_ignores_child_rows(self):
        self.assertEqual(parse_gdu_root(' 42 /root with spaces\n 999 /root with spaces/private-child\n'), 42)

    def test_invalid_first_row_is_not_repaired_from_a_later_child(self):
        self.assertIsNone(parse_gdu_root('error\n42 /root\n'))
        self.assertIsNone(parse_gdu_root('-42 /root\n'))
        self.assertIsNone(parse_gdu_root('42root\n'))

    def test_path_output_is_discarded_but_fingerprint_and_counts_remain(self):
        text = '7 root\nα\n'
        sample = {'stdout': text, 'stderr': 'error: private-child\n', 'reported_bytes': 7}
        redact_gdu_output(sample)
        self.assertNotIn('stdout', sample)
        self.assertNotIn('stderr', sample)
        self.assertNotIn('private-child', json.dumps(sample))
        self.assertEqual(sample['reported_bytes'], 7)
        self.assertEqual(sample['stdout_bytes'], 10)
        self.assertEqual(sample['stdout_lines'], 2)
        self.assertEqual(sample['stdout_sha256'], hashlib.sha256(text.encode()).hexdigest())

    def test_original_byte_fingerprint_is_preserved_over_replacement_decoding(self):
        sample = {'stdout': '�\n', 'stderr': '', 'stdout_bytes': 2,
                  'stdout_lines': 1, 'stdout_sha256': 'raw-byte-fingerprint'}
        redact_gdu_output(sample)
        self.assertEqual(sample['stdout_bytes'], 2)
        self.assertEqual(sample['stdout_sha256'], 'raw-byte-fingerprint')


if __name__ == '__main__':
    unittest.main()

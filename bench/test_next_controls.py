import unittest
from next_controls import parse_diskus, parse_dua, parse_dua_root_and_sentinel

class ParseControls(unittest.TestCase):
    def test_diskus_exact_integer(self):
        self.assertEqual(parse_diskus(b'1234\n'),1234)
        for data in (b'',b'-1\n',b'123 b\n',b'123\n456\n',b'warning 123\n'):
            with self.assertRaises(ValueError):parse_diskus(data)
    def test_dua_one_explicit_root_row(self):
        self.assertEqual(parse_dua(b'   4096 b /owned/path with spaces\n','/owned/path with spaces'),4096)
        for data in (b'4096\n',b'4096 b /other\n',b'4096 b /owned\n8192 b total\n',b'4096 MB /owned\n'):
            with self.assertRaises(ValueError):parse_dua(data,'/owned')
    def test_two_explicit_roots_no_total(self):
        self.assertEqual(parse_dua_root_and_sentinel(b"12 b /owned/root\n0 b /owned/sentinel\n", "/owned/root", "/owned/sentinel"), 12)
        for data in (b"12 b /owned/root\n1 b /owned/sentinel\n", b"12 b /owned/root\n", b"12 b /owned/root\n0 b /owned/sentinel\n12 b total\n"):
            with self.assertRaises(ValueError):parse_dua_root_and_sentinel(data,"/owned/root","/owned/sentinel")
    def test_no_lossy_decode_or_terminal_escape(self):
        for data in (b'1 b /owned\xff\n', b'\x1b[31m1 b /owned\n'):
            with self.assertRaises(ValueError):parse_dua(data,'/owned')

if __name__=='__main__':unittest.main()

import unittest
from dua_tree_capability import completed_frame
from vt_screen import VTScreen
E={'allocated':20480,'files':4,'directories':2,'errors':0}
class DuaFrameContract(unittest.TestCase):
 def frame(self):
  a=[' '*160 for _ in range(40)];a[1]='/owned/tree';a[3]='largest_dir';a[4]='small_a';a[5]='small_b';a[-1]='Sort mode: size, large first  Total disk usage: 20480 b  Processed 5 entries in 0.00s';return a
 def test_exact_completion_rows_and_accounting(self):self.assertTrue(completed_frame(self.frame(),E,'/owned/tree',['largest_dir','small_a','small_b'])['validated'])
 def test_loading_is_not_completion(self):
  a=self.frame();a[-1]='Sort mode: size, large first  Total disk usage: 20480 b  Processed 5 entries in 0s (100/s)';self.assertIsNone(completed_frame(a,E,'/owned/tree',['largest_dir','small_a','small_b']))
 def test_wrong_completed_counts_or_total_fail(self):
  for replacement in ('20481 b  Processed 5','20480 b  Processed 4'):
   a=self.frame();a[-1]=a[-1].replace('20480 b  Processed 5',replacement)
   with self.assertRaises(ValueError):completed_frame(a,E,'/owned/tree',['largest_dir','small_a','small_b'])
 def test_cleared_stale_footer_is_not_accepted(self):
  screen=VTScreen();screen.feed(b'\x1b[40;1H'+self.frame()[-1].encode());screen.feed(b'\x1b[2J\x1b[HLoading')
  self.assertIsNone(completed_frame(screen.lines(),E,'/owned/tree',['largest_dir','small_a','small_b']))
if __name__=='__main__':unittest.main()

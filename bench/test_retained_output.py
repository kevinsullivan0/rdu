import copy
import unittest
from unittest.mock import MagicMock
from retained_output import parse_rdu_tree, parse_dua_tree, validate_rows
from run_retained_correctness import exceeded_budget_fields, make_sparse


def model():
    entries = {'': ('directory',64), 'a': ('file',4), 'folder': ('directory',64),
               'folder/file': ('file',8), 'link': ('symlink',3)}
    return {'nodes': {name: {'kind':kind,'allocated':size,'apparent':size,'hardlink':None}
                      for name,(kind,size) in entries.items()},
            'totals': {'allocated':143,'apparent':143,'files':3,'directories':2,'errors':0}}


RDU = b' 3  link\n 4  a\n72  folder/\n8  folder/file\n143  TOTAL (3 entries, 2 directories, 0 errors; 0.001s)\n'
DUA = b' 0 b /owned/zero\n143 b /owned/root\n3 b   link\n4 b   a\n72 b   folder\n8 b     file\n'


class RetainedTests(unittest.TestCase):
    def test_budget_names_values_not_lost_to_opaque_guard(self):
        good={'files':100,'directories':28,'apparent':2097152,'allocated':1048576,'errors':0}
        self.assertEqual(exceeded_budget_fields(good),[])
        self.assertEqual(exceeded_budget_fields({**good,'files':101,'apparent':2097153,'allocated':1048577,'errors':1}),
                         ['oracle_errors','observed_names','apparent_bytes','allocated_bytes'])

    def test_reviewed_sparse_recipe_is_small_pure_hole_no_payload_write(self):
        path=MagicMock();stream=path.open.return_value.__enter__.return_value
        make_sparse(path)
        path.open.assert_called_once_with('wb');stream.truncate.assert_called_once_with(262144)
        stream.seek.assert_not_called();stream.write.assert_not_called()
    def test_full_named_edges_and_subtotals_both_formats(self):
        for tool,parsed in [('rdu',parse_rdu_tree(RDU)),('dua',parse_dua_tree(DUA,'/owned/root','/owned/zero'))]:
            verdict=validate_rows(parsed,model(),'allocated',tool)
            self.assertTrue(verdict['complete_nodes_and_edges'])
            self.assertEqual(verdict['nodes'],5)

    def test_missing_duplicate_wrong_subtotal_and_extra_rows_fail(self):
        for text in [RDU.replace(b'8  folder/file\n',b''),RDU.replace(b'8  folder/file\n',b'8  folder/file\n8  folder/file\n'),
                     RDU.replace(b'72  folder/',b'71  folder/'),RDU.replace(b'8  folder/file',b'8  unknown')]:
            with self.assertRaises(ValueError):validate_rows(parse_rdu_tree(text),model(),'allocated','rdu')

    def test_shallow_dua_cannot_pass_complete_retention(self):
        parsed=parse_dua_tree(b'0 b /owned/zero\n143 b /owned/root\n','/owned/root','/owned/zero')
        self.assertEqual(parsed['rows'][''],143)
        with self.assertRaises(ValueError):validate_rows(parsed,model(),'allocated','dua')

    def test_invalid_utf8_newline_ansi_unit_and_depth_jump_fail(self):
        for text in [DUA+b'\xff\n',DUA.replace(b'  link',b'  line\nbreak'),DUA+b'\x1b[6n',DUA.replace(b' b ',b' B ',1),DUA.replace(b'    file',b'        file')]:
            with self.assertRaises(ValueError):parse_dua_tree(text,'/owned/root','/owned/zero')

    def test_sort_counts_marker_sentinel_and_symlink_edges_are_strict(self):
        for text in [RDU.replace(b' 3  link\n 4  a',b' 4  a\n 3  link'),RDU.replace(b'3 entries',b'4 entries'),
                     RDU.replace(b'folder/\n',b'folder\n'),RDU.replace(b'link\n',b'link/\n')]:
            with self.assertRaises(ValueError):validate_rows(parse_rdu_tree(text),model(),'allocated','rdu')
        with self.assertRaises(ValueError):parse_dua_tree(DUA.replace(b' 0 b /owned/zero',b' 1 b /owned/zero'),'/owned/root','/owned/zero')
        with self.assertRaises(ValueError):validate_rows(parse_dua_tree(DUA.replace(b'3 b   link\n4 b   a',b'4 b   a\n3 b   link'),'/owned/root','/owned/zero'),model(),'allocated','dua')

    def test_hardlink_names_all_retained_but_charge_once(self):
        fixture={'nodes':{'':{'kind':'directory','allocated':0,'apparent':0,'hardlink':None},
                          'a':{'kind':'file','allocated':4096,'apparent':8193,'hardlink':(1,2)},
                          'b':{'kind':'file','allocated':4096,'apparent':8193,'hardlink':(1,2)}},
                 'totals':{'allocated':4096,'apparent':8193,'files':2,'directories':1,'errors':0}}
        good=b'0  a\n4096  b\n4096  TOTAL (2 entries, 1 directories, 0 errors; 0.001s)\n'
        self.assertTrue(validate_rows(parse_rdu_tree(good),fixture,'allocated','rdu')['hardlink_charge_sums'])
        with self.assertRaises(ValueError):validate_rows(parse_rdu_tree(good.replace(b'0  a',b'4096  a')),fixture,'allocated','rdu')

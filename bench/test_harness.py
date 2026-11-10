import os
from pathlib import Path
import tempfile
import unittest

from harness import oracle, dataset_paths


class OracleTests(unittest.TestCase):
    # Catch following links, double-counting hardlinks, or omitting directory metadata.
    def test_sparse_hardlink_and_symlink_accounting(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            child = root / "child"
            child.mkdir()
            file = child / "file"
            file.write_bytes(b"abc")
            os.link(file, root / "hard")
            os.symlink("child/file", root / "link")
            os.symlink("child", root / "dirlink")
            os.symlink("missing", root / "dangling")
            sparse = root / "sparse"
            with sparse.open("wb") as stream:
                stream.truncate(1024 * 1024)
            observed = oracle(root)
            entries = [root, child, file, root / "link", root / "dirlink", root / "dangling", sparse]
            self.assertEqual(observed["allocated"], sum(p.lstat().st_blocks * 512 for p in entries))
            self.assertEqual(observed["apparent"], sum(p.lstat().st_size for p in entries))
            self.assertEqual(observed["files"], 6)
            self.assertEqual(observed["directories"], 2)
            self.assertEqual(observed["hardlink_duplicates"], 1)
            self.assertEqual(observed["errors"], 0)

    # Catch silently dropping requested external metadata-only trees or mutating them.
    def test_existing_dataset_is_included_without_creating_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "existing").write_bytes(b"kept")
            before = sorted(p.name for p in root.iterdir())
            selected = dataset_paths(["wide"], [["cached", str(root)]])
            self.assertEqual(selected["cached"], root.resolve())
            self.assertIn("wide", selected)
            self.assertEqual(sorted(p.name for p in root.iterdir()), before)

    def test_existing_dataset_cannot_shadow_a_synthetic_label(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                dataset_paths(["wide"], [["wide", temporary]])


if __name__ == "__main__":
    unittest.main()

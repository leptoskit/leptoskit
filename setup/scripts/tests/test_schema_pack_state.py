"""The acceptance observer must detect data changes and never create a DB."""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[3] / "crates/leptoskit_cli/tests/fixtures/schema_pack_state.py"


class SchemaPackState(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.path = Path(self.scratch.name) / "fixture with spaces.sqlite3"

    def run_snapshot(self):
        return subprocess.run([sys.executable, str(SCRIPT), str(self.path)], capture_output=True, text=True, timeout=10)

    def seed(self):
        with sqlite3.connect(self.path) as db:
            db.executescript('CREATE TABLE notes(id BLOB PRIMARY KEY, body TEXT); CREATE TABLE "a""b"(n INTEGER);')
            db.execute("INSERT INTO notes VALUES(?,?)", (b"\x00\xff", "retained note"))
            db.execute('INSERT INTO "a""b" VALUES(3)')

    def test_missing_database_fails_without_creating_it(self):
        self.assertNotEqual(self.run_snapshot().returncode, 0)
        self.assertFalse(self.path.exists())

    def test_snapshot_reads_real_rows_and_leaves_file_unchanged(self):
        self.seed()
        before = self.path.read_bytes()
        result = self.run_snapshot()
        self.assertEqual(result.returncode, 0, result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(value["rows"]["notes"], [[{"blob": "00ff"}, "retained note"]])
        self.assertEqual(value["rows"]['a"b'], [[3]])
        self.assertEqual(self.path.read_bytes(), before)
        self.assertEqual(self.run_snapshot().stdout, result.stdout)

    def test_same_schema_with_changed_data_is_distinguishable(self):
        self.seed()
        before = json.loads(self.run_snapshot().stdout)
        with sqlite3.connect(self.path) as db:
            db.execute("UPDATE notes SET body='changed'")
        after = json.loads(self.run_snapshot().stdout)
        self.assertEqual(before["schema"], after["schema"])
        self.assertNotEqual(before["rows"], after["rows"])


if __name__ == "__main__":
    unittest.main()

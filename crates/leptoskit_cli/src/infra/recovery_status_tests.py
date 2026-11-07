"""Run the generated recovery worker's status command without external services."""
import datetime
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

WORKER = pathlib.Path(sys.argv.pop(1)).resolve()


class BackupStatus(unittest.TestCase):
    def invoke(self, status=None, failure=None):
        with tempfile.TemporaryDirectory(prefix="lk-backup-status-") as directory:
            root = pathlib.Path(directory)
            shutil.copyfile(WORKER, root / "recovery.py")
            (root / "recovery.json").write_text(json.dumps({
                "name": "fixture", "repository": str(root / "repository"), "max_age_hours": 36,
            }))
            (root / "backup-secrets.json").write_text("{}")
            if status is not None:
                (root / "backup-status.json").write_text(json.dumps(status))
            if failure is not None:
                (root / "backup-error.json").write_text(json.dumps(failure))
            result = subprocess.run(
                [sys.executable, str(root / "recovery.py"), "status"],
                capture_output=True, text=True, check=False,
            )
            return result.returncode, json.loads(result.stdout)

    def completed(self, hours_ago):
        timestamp = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(hours=hours_ago)
        return {"verified": True, "snapshot": "a" * 64, "completed_at": timestamp.isoformat()}

    def test_recent_verified_restore_check_is_healthy(self):
        code, result = self.invoke(self.completed(1))
        self.assertEqual(code, 0)
        self.assertTrue(result["healthy"])
        self.assertGreaterEqual(result["age_seconds"], 3600)

    def test_absent_check_is_not_healthy(self):
        code, result = self.invoke()
        self.assertEqual(code, 0)
        self.assertFalse(result["healthy"])
        self.assertIsNone(result["age_seconds"])

    def test_expired_check_is_not_healthy(self):
        code, result = self.invoke(self.completed(37))
        self.assertEqual(code, 0)
        self.assertFalse(result["healthy"])

    def test_future_check_is_not_healthy(self):
        code, result = self.invoke(self.completed(-24))
        self.assertEqual(code, 0)
        self.assertFalse(result["healthy"])
        self.assertLess(result["age_seconds"], 0)

    def test_unverified_timestamp_is_not_healthy(self):
        status = self.completed(1)
        status["verified"] = False
        code, result = self.invoke(status)
        self.assertEqual(code, 0)
        self.assertFalse(result["healthy"])

    def test_later_failure_overrides_a_recent_success(self):
        failure = {"failed_at": self.completed(0)["completed_at"], "error": "verification failed"}
        code, result = self.invoke(self.completed(1), failure)
        self.assertEqual(code, 0)
        self.assertFalse(result["healthy"])
        self.assertEqual(result["last_error"], failure)

    def test_invalid_or_timezone_free_timestamp_fails_closed(self):
        for timestamp in ["not a timestamp", "2026-11-07T12:00:00"]:
            with self.subTest(timestamp=timestamp):
                code, result = self.invoke({"verified": True, "completed_at": timestamp})
                self.assertNotEqual(code, 0)
                self.assertIn("error", result)
                self.assertNotIn("healthy", result)


unittest.main()

import hashlib
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import session


class BenchmarkLifecycleTests(unittest.TestCase):
    def test_checksum_is_content_based(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "artifact"
            path.write_bytes(b"verified upstream artifact")
            self.assertEqual(session.sha256(path), hashlib.sha256(path.read_bytes()).hexdigest())

    def test_early_process_exit_is_not_readiness(self):
        class Exited:
            args, returncode = ["failed-engine"], 23
            def poll(self):
                return self.returncode
        with self.assertRaisesRegex(RuntimeError, "exited 23"):
            session.wait_ready(lambda: True, [Exited()])

    def test_deadline_is_not_success(self):
        with self.assertRaisesRegex(RuntimeError, "deadline"):
            session.wait_ready(lambda: False, [], timeout=0)

    def test_started_process_is_reaped_on_failure(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(session, "RESULTS", pathlib.Path(directory)):
            child = None
            with self.assertRaisesRegex(RuntimeError, "probe failed"):
                with session.processes() as (_, start):
                    child = start("engine", [sys.executable, "-c", "import time; time.sleep(60)"], None)
                    raise RuntimeError("probe failed")
            self.assertIsNotNone(child.poll())

    def test_summary_retains_exclusions_and_converts_units(self):
        report = {"latency": [{"cpp": {"median": 20000}, "rust": {"median": 10000}}],
                  "memory": {"cpp": [{"pss_kib": 200}], "rust": [{"pss_kib": 100}]}}
        text = session.summarize([report])
        self.assertIn("20.000 | 10.000", text)
        self.assertIn("Search is excluded", text)
        self.assertIn("not a feature-equivalent efficiency claim", text)

    def test_profiles_are_separate_and_runtime_private(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(session, "RESULTS", pathlib.Path(directory)):
            a, b = session.profile("cpp"), session.profile("rust")
            for key in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_RUNTIME_DIR"):
                self.assertNotEqual(a[key], b[key])
            self.assertEqual(pathlib.Path(a["XDG_RUNTIME_DIR"]).stat().st_mode & 0o777, 0o700)

    def test_onboarding_is_not_launcher_readiness(self):
        replies = [subprocess.CompletedProcess([], 0, "42\n"),
                   subprocess.CompletedProcess([], 0, "Welcome to Vicinae\n")]
        with patch.object(session.subprocess, "run", side_effect=replies):
            self.assertEqual(session.visible(123, {}, "Vicinae Launcher"), [])

    def test_named_mapped_launcher_is_ready(self):
        replies = [subprocess.CompletedProcess([], 0, "42\n"),
                   subprocess.CompletedProcess([], 0, "Vicinae Launcher\n")]
        with patch.object(session.subprocess, "run", side_effect=replies):
            self.assertEqual(session.visible(123, {}, "Vicinae Launcher"),
                             [{"id": "42", "title": "Vicinae Launcher", "pid": 123}])


if __name__ == "__main__":
    unittest.main()

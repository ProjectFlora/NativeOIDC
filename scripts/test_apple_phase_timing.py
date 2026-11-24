import os
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = os.path.dirname(os.path.abspath(__file__))
HELPER = os.path.join(SCRIPTS, "apple-phase-timing.sh")
BASH = shutil.which("bash")


@unittest.skipUnless(BASH, "bash is not installed")
class PhaseTimingTestCase(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.directory = directory.name
        self.timing_file = os.path.join(self.directory, "timings.tsv")
        self.summary_file = os.path.join(self.directory, "summary.md")

    def run_bash(self, body, timing_file=True, summary=True):
        env = {key: value for key, value in os.environ.items()
               if key not in ("APPLE_PHASE_TIMING_FILE", "GITHUB_STEP_SUMMARY")}
        if timing_file:
            env["APPLE_PHASE_TIMING_FILE"] = self.timing_file
        if summary:
            env["GITHUB_STEP_SUMMARY"] = self.summary_file
        script = "set -euo pipefail\nsource \"$1\"\n" + body
        return subprocess.run((BASH, "-c", script, "bash", HELPER.replace("\\", "/")),
                              env=env, capture_output=True, text=True)

    def read(self, path):
        with open(path, encoding="utf-8") as handle:
            return handle.read()

    def rows(self):
        # A phase with no work can still cross a $SECONDS tick, so a row's
        # seconds are checked to be 0 or 1 and left out of the comparison.
        rows = [line.split("\t") for line in self.read(self.timing_file).splitlines()]
        for row in rows:
            self.assertIn(row[2], ("0", "1"), row)
        return [row[:2] for row in rows]

    def test_begin_and_end_record_an_ok_row(self):
        result = self.run_bash("phase_begin 'Build: one'\nphase_end\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.rows(), [["Build: one", "ok"]])
        self.assertRegex(result.stderr, r"\[timing\] Build: one: [01]s \(ok\)")
        # With a timing file, rows wait for phase_write_summary.
        self.assertFalse(os.path.exists(self.summary_file))

    def test_begin_ends_the_open_phase(self):
        result = self.run_bash("phase_begin first\nphase_begin second\nphase_end\nphase_end\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.rows(), [["first", "ok"], ["second", "ok"]])

    def test_failing_command_records_the_open_phase_as_failed(self):
        result = self.run_bash("trap phase_fail_open EXIT\nphase_begin done\nphase_end\n"
                               "phase_begin broken\nfalse\nphase_end\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.rows(), [["done", "ok"], ["broken", "failed"]])

    def test_fail_open_without_a_phase_records_nothing(self):
        result = self.run_bash("phase_begin done\nphase_end\nphase_fail_open\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.rows(), [["done", "ok"]])

    def test_summary_table(self):
        result = self.run_bash("phase_record 'Build: Rust' ok 99\nphase_record 'iOS flow: launch' failed 7\n"
                               "phase_write_summary 'Apple packaging phases'\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        table = ("### Apple packaging phases\n\n"
                 "| Phase | Result | Time |\n"
                 "|---|---|---:|\n"
                 "| Build: Rust | ok | 1m39s |\n"
                 "| iOS flow: launch | failed | 7s |\n")
        self.assertEqual(result.stdout, table)
        self.assertEqual(self.read(self.summary_file), table + "\n")

    def test_summary_without_rows_writes_nothing(self):
        result = self.run_bash("phase_write_summary 'Apple packaging phases'\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertFalse(os.path.exists(self.summary_file))

    def test_standalone_run_lists_rows_in_the_summary(self):
        result = self.run_bash("phase_record 'iOS flow: install app' ok 61\n", timing_file=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read(self.summary_file), "- iOS flow: install app: 1m01s (ok)\n")
        self.assertFalse(os.path.exists(self.timing_file))

    def test_standalone_run_without_summary_only_logs(self):
        result = self.run_bash("phase_begin alone\nphase_end\n", timing_file=False, summary=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stderr, r"\[timing\] alone: [01]s \(ok\)")
        self.assertFalse(os.path.exists(self.summary_file))


@unittest.skipUnless(BASH, "bash is not installed")
class AppleScriptSyntaxTestCase(unittest.TestCase):
    def test_apple_packaging_scripts_parse(self):
        for name in ("apple-phase-timing.sh", "boot-ios-simulator.sh", "check-apple-static-link-smoke.sh",
                     "check-ios-authorization-flow.sh", "package-apple-xcframework.sh"):
            with self.subTest(script=name):
                result = subprocess.run((BASH, "-n", os.path.join(SCRIPTS, name).replace("\\", "/")),
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()

import os
import shutil
import subprocess
import tempfile
import time
import unittest

SCRIPTS = os.path.dirname(os.path.abspath(__file__))
HELPER = os.path.join(SCRIPTS, "apple-phase-timing.sh")
BASH = shutil.which("bash")


class BashHelperMixin:
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


@unittest.skipUnless(BASH, "bash is not installed")
class PhaseTimingTestCase(BashHelperMixin, unittest.TestCase):
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
class UnwritableDestinationTestCase(BashHelperMixin, unittest.TestCase):
    """Timing is best-effort: a failed write must not stop a script or its cleanup."""

    def setUp(self):
        super().setUp()
        # A directory cannot be appended to, so every write to it fails.
        os.mkdir(self.timing_file)
        os.mkdir(self.summary_file)

    def test_failed_writes_warn_and_continue(self):
        result = self.run_bash("phase_begin one\nphase_end\nphase_record two ok 3\necho after\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "after\n")
        self.assertIn("warning: could not write phase timing to", result.stderr)

    def test_failed_write_in_exit_trap_reaches_cleanup_and_keeps_status(self):
        result = self.run_bash("cleanup() {\n    phase_fail_open\n    echo cleanup-reached\n}\n"
                               "trap cleanup EXIT\nphase_begin broken\nexit 3\n")
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertEqual(result.stdout, "cleanup-reached\n")
        self.assertIn("warning: could not write phase timing to", result.stderr)

    def test_failed_summary_write_continues(self):
        os.rmdir(self.timing_file)
        with open(self.timing_file, "w", encoding="utf-8") as handle:
            handle.write("one\tok\t1\n")
        result = self.run_bash("phase_write_summary title\necho after\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.endswith("| one | ok | 1s |\nafter\n"), result.stdout)
        self.assertIn("warning: could not write phase timing to", result.stderr)

    def test_standalone_failed_summary_write_continues(self):
        result = self.run_bash("phase_record one ok 1\necho after\n", timing_file=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "after\n")
        self.assertIn("warning: could not write phase timing to", result.stderr)


def script_lines(name):
    with open(os.path.join(SCRIPTS, name), encoding="utf-8") as handle:
        return handle.read().splitlines()


def first_line(lines, text, start=0):
    return next(index for index in range(start, len(lines)) if text in lines[index])


@unittest.skipUnless(BASH, "bash is not installed")
class PackageCleanupTestCase(BashHelperMixin, unittest.TestCase):
    """The package script's own cleanup function, with self-test removal failing."""

    def run_cleanup(self, ending):
        lines = script_lines("package-apple-xcframework.sh")
        start = first_line(lines, "cleanup() {")
        end = lines.index("}", start)
        cleanup = "\n".join(lines[start:end + 1])
        body = (cleanup + "\n"
                # Only the self-test removal (`rm -rf`) fails; other rm calls run.
                'rm() { if [[ "$1" == -rf ]]; then return 1; fi; command rm "$@"; }\n'
                "scratch_xcframework=scratch.xcframework\nrecheck=recheck.zip\n"
                "package_started=$SECONDS\ntrap cleanup EXIT\n"
                "phase_begin 'Archive: zip and determinism self-test'\n" + ending)
        return self.run_bash(body)

    def test_packaging_error_is_kept_and_the_table_is_written(self):
        result = self.run_cleanup("exit 17\n")
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertIn("| Archive: zip and determinism self-test | failed |", result.stdout)
        self.assertIn("| Total | failed |", result.stdout)
        self.assertIn("Could not remove the packaging self-test outputs", result.stderr)
        self.assertFalse(os.path.exists(self.timing_file))

    def test_cleanup_failure_fails_an_otherwise_successful_run(self):
        result = self.run_cleanup("phase_end\n")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("| Archive: zip and determinism self-test | ok |", result.stdout)
        self.assertIn("| Total | failed |", result.stdout)
        self.assertIn("Could not remove the packaging self-test outputs", result.stderr)
        self.assertFalse(os.path.exists(self.timing_file))


@unittest.skipUnless(BASH and shutil.which("perl"), "bash or perl is not installed")
class SettleSamplerTestCase(BashHelperMixin, unittest.TestCase):
    """The iOS flow's settle sampler, with a stub that starts a long child."""

    def run_sampler(self, between):
        lines = script_lines("check-ios-authorization-flow.sh")
        start = first_line(lines, "settle_start() {")
        end = first_line(lines, "} >/dev/null 2>&1", start)
        functions = "\n".join(lines[start:end + 1])
        child_file = os.path.join(self.directory, "child.pid")
        body = (functions + "\n"
                # The stub records its child, a command that outlives any test.
                'settle_sampler() { sleep 60 & echo "$!" > "$SETTLE_CHILD_FILE"; wait; }\n'
                f'export SETTLE_CHILD_FILE="{child_file}"\n'
                f'settle_log="{os.path.join(self.directory, "settle.log")}"\nsettle_pid=""\n'
                "settle_start\n" + between + "settle_stop\n"
                'echo "stopped:$settle_pid"\n')
        result = self.run_bash(body)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("stopped:\n", result.stdout)
        return child_file

    def assert_stopped(self, child_file):
        with open(child_file, encoding="utf-8") as handle:
            child = int(handle.read())
        for _ in range(50):
            try:
                os.kill(child, 0)
            except ProcessLookupError:
                return
            time.sleep(0.1)
        os.kill(child, 9)
        self.fail("the sampler's child outlived settle_stop")

    def test_stop_stops_the_running_sampler_and_its_child(self):
        child_file = self.run_sampler('while [[ ! -s "$SETTLE_CHILD_FILE" ]]; do sleep 0.1; done\n')
        self.assert_stopped(child_file)

    def test_stop_during_startup_stops_a_child_started_in_between(self):
        # The perl override holds the launcher before it makes the group, so
        # settle_stop's first group signal must fail. The kill override then
        # releases the launcher and waits for the sampler's child before the
        # process signal, so only the final group signal can stop the child.
        gate = os.path.join(self.directory, "gate")
        first_status = os.path.join(self.directory, "first-status")
        child_file = self.run_sampler(
            "perl() {\n"
            f'    while [[ ! -e "{gate}" ]]; do sleep 0.05; done\n'
            '    exec perl "$@"\n'
            "}\n"
            "kill() {\n"
            "    local status=0\n"
            '    builtin kill "$@" || status=$?\n'
            f'    if [[ ! -e "{gate}" ]]; then\n'
            f'        echo "$status" > "{first_status}"\n'
            f'        : > "{gate}"\n'
            '        for _ in {1..100}; do [[ -s "$SETTLE_CHILD_FILE" ]] && break; sleep 0.1; done\n'
            "    fi\n"
            '    return "$status"\n'
            "}\n")
        with open(first_status, encoding="utf-8") as handle:
            self.assertNotEqual(handle.read().strip(), "0", "the group existed before the first signal")
        self.assertTrue(os.path.exists(child_file), "the sampler did not start its child within 10s")
        self.assert_stopped(child_file)


class ScriptOrderTestCase(unittest.TestCase):
    """Failure paths that the bash tests above cannot reach without Xcode."""

    def test_ios_flow_cleanup_is_installed_before_any_phase_sdk_lookup_or_boot(self):
        lines = script_lines("check-ios-authorization-flow.sh")
        trap = first_line(lines, "trap cleanup EXIT")
        for text in ("phase_begin", "--show-sdk-path", "simctl boot", "ios-mock-oauth-provider.py"):
            with self.subTest(text=text):
                self.assertLess(trap, first_line(lines, text))

    def test_ios_flow_selection_phase_covers_the_toolchain_diagnostics(self):
        lines = script_lines("check-ios-authorization-flow.sh")
        begin = first_line(lines, 'phase_begin "iOS flow: select simulator"')
        end = first_line(lines, "phase_end", begin)
        for text in ("sw_vers", "xcodebuild -version", "iOS authorization: "):
            with self.subTest(text=text):
                self.assertLess(begin, first_line(lines, text, begin))
                self.assertLess(first_line(lines, text, begin), end)

    def test_package_timing_file_setup_cannot_stop_packaging(self):
        lines = script_lines("package-apple-xcframework.sh")
        trap = first_line(lines, "trap cleanup EXIT")
        setup = lines[:trap]
        mktemp = first_line(setup, "mktemp")
        self.assertTrue(setup[mktemp].lstrip().startswith("if "), setup[mktemp])
        # No top-level removal of a fixed timing path before the trap (cleanup's
        # own best-effort removal is indented inside the function).
        self.assertFalse([line for line in setup if line.startswith('rm -f "$APPLE_PHASE_TIMING_FILE"')])


@unittest.skipUnless(BASH, "bash is not installed")
class AppleScriptSyntaxTestCase(unittest.TestCase):
    def test_apple_packaging_scripts_parse(self):
        for name in ("apple-phase-timing.sh", "check-apple-static-link-smoke.sh",
                     "check-ios-authorization-flow.sh", "package-apple-xcframework.sh"):
            with self.subTest(script=name):
                result = subprocess.run((BASH, "-n", os.path.join(SCRIPTS, name).replace("\\", "/")),
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()

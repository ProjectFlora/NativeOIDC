import importlib.util
import os
import subprocess
import tempfile
import unittest

import select_ci_jobs
from select_ci_jobs import ALL, JOBS, classify, compute, select, verify


def selected(*paths):
    return set(select(paths))


class RepresentativeChangeTests(unittest.TestCase):
    def test_unshipped_documentation_runs_nothing(self):
        self.assertEqual(selected("CHANGELOG.md", "docs/agent-workflows/issue-to-pr.md", ".gitignore"), set())
        self.assertEqual(selected("scripts/agent-precheck.sh"), set())

    def test_release_documentation_runs_tests(self):
        # The Tests job builds and checks the release documentation set.
        self.assertEqual(selected("README.md", "docs/user-guide.md", "ROADMAP.md", "FEATURE_MATRIX.md"), {"test"})
        self.assertEqual(selected("scripts/build-release-docs.py"), {"test"})
        self.assertEqual(selected("scripts/check-release-docs.py"), {"test"})
        self.assertEqual(selected("scripts/test_check_release_docs.py"), {"test"})

    def test_shared_rust_core_runs_every_platform(self):
        self.assertEqual(selected("src/authorization/session.rs"), ALL - {"licenses", "spm_manifest"})

    def test_windows_rust_module_runs_rust_checks_and_windows_only(self):
        self.assertEqual(selected("src/windows/loopback.rs"), {"msrv", "test", "lint", "coverage", "windows"})

    def test_swift_adapter_runs_apple_only(self):
        self.assertEqual(selected("Sources/NativeOIDC/MacOSAuthorizationAdapter.swift"), {"apple", "test"})

    def test_swift_tests_run_apple(self):
        # #167 moves macOS smoke checks into SwiftTests.
        self.assertEqual(selected("SwiftTests/NativeOIDCTests/NewTests.swift"), {"apple", "test"})

    def test_generated_swift_bindings_run_apple_and_binding_check(self):
        self.assertEqual(selected("Sources/NativeOIDC/native_oidc.swift"), {"apple", "test"})

    def test_android_emulator_tests_run_android_only(self):
        # #166 runs these on API 28 and API 35.
        self.assertEqual(
            selected("native/android/src/androidTest/kotlin/net/projectflora/nativeoidc/AndroidExternalBrowserAuthorizationTest.kt"),
            {"android", "test"},
        )

    def test_windows_adapter_runs_windows_only(self):
        self.assertEqual(selected("native/windows/dotnet/NativeOidcWindows.cs"), {"windows", "test"})

    def test_workflow_change_runs_everything(self):
        self.assertEqual(selected(".github/workflows/ci.yml"), ALL)
        self.assertEqual(selected(".github/workflows/release.yml"), ALL)

    def test_selector_change_runs_everything(self):
        self.assertEqual(selected("scripts/select_ci_jobs.py"), ALL)
        self.assertEqual(selected("scripts/test_select_ci_jobs.py"), ALL)

    def test_cargo_manifest_runs_everything(self):
        self.assertEqual(selected("Cargo.lock"), ALL)


class SharedInputTests(unittest.TestCase):
    def test_files_shipped_in_packages_are_not_documentation(self):
        self.assertEqual(selected("LICENSE"), {"apple", "android", "windows"})
        self.assertEqual(selected("docs/swift-xcframework.md"), {"apple", "test"})
        self.assertEqual(selected("native/windows/README.md"), {"windows", "test"})

    def test_compiled_guide_is_not_documentation(self):
        self.assertEqual(selected("docs/windows-cpp-guide.md"), {"windows", "test"})
        self.assertEqual(selected("docs/windows-dotnet-guide.md"), {"windows", "test"})

    def test_notice_template_runs_licenses_and_packages(self):
        self.assertEqual(selected("about.hbs"), {"licenses", "apple", "android", "windows"})

    def test_package_manifest_runs_apple_and_manifest_check(self):
        self.assertEqual(selected("Package.swift"), {"apple", "spm_manifest"})

    def test_scripts_also_run_script_unit_tests(self):
        self.assertEqual(selected("scripts/package-windows.ps1"), {"test", "windows"})
        self.assertEqual(selected("scripts/test_select_ios_simulator.py"), {"test"})
        self.assertEqual(selected("scripts/check-macos-real-browser.sh"), {"test"})
        self.assertEqual(selected("native/apple/MacOSRealBrowserCheck.swift"), {"apple", "test"})
        self.assertEqual(selected("scripts/generate-bindings.sh"), {"test", "apple", "android"})
        self.assertEqual(selected("scripts/apple-phase-timing.sh"), {"test", "apple"})
        self.assertEqual(selected("scripts/check-apple-static-link-smoke.sh"), {"test", "apple"})
        self.assertEqual(selected("scripts/test_apple_phase_timing.py"), {"test"})

    def test_unknown_paths_run_everything(self):
        self.assertEqual(selected("rust-toolchain.toml"), ALL)
        self.assertEqual(selected("scripts/new-script.sh"), ALL)
        self.assertEqual(selected("native/linux/adapter.c"), ALL)
        self.assertEqual(classify("rust-toolchain.toml")[1], "unclassified path")

    def test_union_of_paths(self):
        self.assertEqual(selected("README.md", "native/windows/include/native_oidc_windows.h",
                                  "native/consumer-smoke/android/app/build.gradle.kts"),
                         {"android", "windows", "test"})

    def test_release_documentation_set_inputs_run_tests(self):
        # Every repository file the release documentation set ships must
        # select the Tests job, which builds and checks the set; a guide
        # that starts linking a new file is caught here.
        root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        spec = importlib.util.spec_from_file_location(
            "build_release_docs", os.path.join(root, "scripts", "build-release-docs.py"))
        build_release_docs = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(build_release_docs)
        missing = [path for path in build_release_docs.link_closure(root) if "test" not in classify(path)[0]]
        self.assertEqual(missing, [], "add `test` to the rule for each path (docs/ci-job-selection.md)")

    def test_every_tracked_file_is_classified(self):
        root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        try:
            files = subprocess.run(("git", "-C", root, "ls-files", "-z"), check=True,
                                   capture_output=True, text=True).stdout.split("\0")
        except (OSError, subprocess.CalledProcessError):
            self.skipTest("not a git checkout")
        unclassified = [path for path in files if path and classify(path)[1] == "unclassified path"]
        self.assertEqual(unclassified, [], "add a rule to RULES and docs/ci-job-selection.md")


class GitRepositoryTestCase(unittest.TestCase):
    def setUp(self):
        self._directory = tempfile.TemporaryDirectory()
        self.addCleanup(self._directory.cleanup)
        self._cwd = os.getcwd()
        os.chdir(self._directory.name)
        self.addCleanup(os.chdir, self._cwd)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.email", "ci@example.invalid")
        self.git("config", "user.name", "CI")
        self.commit("README.md")

    def git(self, *args):
        return subprocess.run(("git",) + args, check=True, capture_output=True, text=True).stdout.strip()

    def commit(self, path):
        directory = os.path.dirname(path)
        if directory:
            os.makedirs(directory, exist_ok=True)
        with open(path, "a", encoding="utf-8") as handle:
            handle.write("change\n")
        self.git("add", path)
        self.git("commit", "-q", "-m", path)
        return self.git("rev-parse", "HEAD")


class DiffRangeTests(GitRepositoryTestCase):
    def test_push_compares_against_previous_commit(self):
        before = self.git("rev-parse", "HEAD")
        self.commit("docs/user-guide.md")
        self.commit("native/windows/README.md")
        jobs, context = compute("push", {"before": before})
        self.assertEqual(set(jobs), {"windows", "test"})
        self.assertEqual(context["files"], ["docs/user-guide.md", "native/windows/README.md"])

    def test_first_push_runs_everything(self):
        jobs, context = compute("push", {"before": "0" * 40})
        self.assertEqual(set(jobs), ALL)
        self.assertEqual(context["mode"], "all")

    def test_unavailable_previous_commit_runs_everything(self):
        jobs, context = compute("push", {"before": "1" * 40})
        self.assertEqual(set(jobs), ALL)
        self.assertIn("not available", context["reason"])

    def test_rewritten_history_runs_everything(self):
        self.git("checkout", "-q", "-b", "other")
        other = self.commit("docs/a.md")
        self.git("checkout", "-q", "main")
        self.commit("docs/b.md")
        jobs, context = compute("push", {"before": other})
        self.assertEqual(set(jobs), ALL)
        self.assertIn("not an ancestor", context["reason"])

    def test_pull_request_compares_merge_commit_with_base_parent(self):
        self.git("checkout", "-q", "-b", "feature")
        self.commit("Sources/NativeOIDC/IOSAuthorizationAdapter.swift")
        self.git("checkout", "-q", "main")
        self.commit("src/lib.rs")  # on the base branch: not part of the PR
        self.git("merge", "-q", "--no-ff", "-m", "merge", "feature")
        jobs, context = compute("pull_request", {})
        self.assertEqual(set(jobs), {"apple", "test"})
        self.assertEqual(context["files"], ["Sources/NativeOIDC/IOSAuthorizationAdapter.swift"])

    def test_pull_request_without_merge_commit_runs_everything(self):
        jobs, context = compute("pull_request", {})
        self.assertEqual(set(jobs), ALL)
        self.assertEqual(context["mode"], "all")

    def test_other_events_run_everything(self):
        jobs, _ = compute("workflow_dispatch", {})
        self.assertEqual(set(jobs), ALL)


def changes(**selection):
    outputs = {job: "true" for job in JOBS}
    outputs.update({job: str(value).lower() for job, value in selection.items()})
    return {"result": "success", "outputs": outputs}


class VerifyTests(unittest.TestCase):
    def test_passed_and_intended_skips_pass(self):
        ok, rows = verify({
            "changes": changes(windows=False),
            "test": {"result": "success"},
            "windows_core": {"result": "skipped"},
            "windows_client": {"result": "skipped"},
        })
        self.assertTrue(ok)
        self.assertIn("skipped as intended", rows[1][2])

    def test_failure_fails(self):
        ok, rows = verify({"changes": changes(), "test": {"result": "failure"}})
        self.assertFalse(ok)
        self.assertEqual(rows[0][2], "failed")

    def test_cancel_fails_and_is_named(self):
        ok, rows = verify({"changes": changes(), "apple_adapters": {"result": "cancelled"}})
        self.assertFalse(ok)
        self.assertEqual(rows[0][2], "cancelled")

    def test_skip_of_selected_job_fails(self):
        ok, rows = verify({
            "changes": changes(),
            "windows_core": {"result": "failure"},
            "windows_client": {"result": "skipped"},
        })
        self.assertFalse(ok)
        self.assertIn("skipped although selected", rows[1][2])

    def test_failed_selection_requires_every_job(self):
        ok, rows = verify({
            "changes": {"result": "failure", "outputs": {}},
            "test": {"result": "success"},
            "android_adapters": {"result": "skipped"},
        })
        self.assertFalse(ok)
        self.assertEqual(rows[0][0], "changes")
        self.assertIn("skipped although selected", rows[2][2])

    def test_missing_selection_output_requires_the_job(self):
        ok, _ = verify({"changes": {"result": "success", "outputs": {}}, "lint": {"result": "skipped"}})
        self.assertFalse(ok)

    def test_unknown_job_fails(self):
        ok, rows = verify({"changes": changes(), "new_job": {"result": "success"}})
        self.assertFalse(ok)
        self.assertIn("unknown job", rows[0][2])


class ReportTests(unittest.TestCase):
    def test_report_lists_every_group_and_file(self):
        report = select_ci_jobs.report_markdown(
            select(["CHANGELOG.md", "native/android/build.gradle.kts"]),
            {"mode": "diff", "base": "a" * 40, "head": "b" * 40,
             "files": ["CHANGELOG.md", "native/android/build.gradle.kts"]},
        )
        for job in JOBS:
            self.assertIn(f"| {job} |", report)
        self.assertIn("| android | yes | Android adapter, tests or consumer |", report)
        self.assertIn("| windows | no |", report)
        self.assertIn("- `CHANGELOG.md` → none (documentation)", report)

    def test_report_names_why_everything_runs(self):
        report = select_ci_jobs.report_markdown(select_ci_jobs.select_all("first push"),
                                                {"mode": "all", "reason": "first push", "files": []})
        self.assertIn("All jobs run: first push.", report)


if __name__ == "__main__":
    unittest.main()

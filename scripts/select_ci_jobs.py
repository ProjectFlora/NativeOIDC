#!/usr/bin/env python3
"""Select the Rust CI jobs that a change needs, and check their results.

The rules and the reasons behind them are in docs/ci-job-selection.md.
Every changed path is matched against RULES, first match wins, and the
selected jobs are the union over all paths. A path that matches no rule,
a diff that cannot be computed, or an event without a known base selects
every job: the selector only skips a job when it can name why the change
cannot affect it.

  select_ci_jobs.py select   reads GITHUB_EVENT_NAME, GITHUB_EVENT_PATH and
                             the checked-out history, and writes one
                             `<job>=true|false` output per job, `selection`
                             (JSON) and `report` (Markdown) to GITHUB_OUTPUT.
  select_ci_jobs.py verify   reads NEEDS_JSON (the workflow's `needs`
                             context) and fails unless every job either
                             passed, or was skipped because the selection
                             left it out.
"""

import json
import os
import re
import subprocess
import sys

# Job groups, in the order the workflow and the report list them. Each
# group is one `changes` output that one or more workflow jobs read.
JOBS = (
    "msrv",
    "test",
    "lint",
    "coverage",
    "licenses",
    "spm_bindings",
    "spm_manifest",
    "apple",
    "android",
    "windows",
)
ALL = frozenset(JOBS)
RUST = frozenset({"msrv", "test", "lint", "coverage"})
# The packaging jobs embed LICENSE and the cargo-about notices.
PACKAGES = frozenset({"apple", "android", "windows"})

# (patterns, jobs, reason). Order matters: the first rule whose pattern
# matches a path decides that path, so specific rules come before the
# directory-wide ones that would also match.
RULES = (
    # CI definition and the selector itself: a mistake here could hide a
    # failure, so they run everything.
    ((".github/**",), ALL, "CI workflow"),
    (("scripts/select_ci_jobs.py", "scripts/test_select_ci_jobs.py"), ALL, "CI job selector"),
    # Shared build inputs.
    (("Cargo.toml", "Cargo.lock"), ALL, "Cargo manifest or lockfile"),
    ((".gitattributes",), ALL, "checkout attributes (line endings of build inputs)"),
    # Rust sources. src/windows is compiled only for Windows and for tests
    # (`#[cfg(any(windows, test))]` in src/lib.rs), so Apple, Android and
    # the generated Swift bindings never contain it.
    (("src/windows/**",), RUST | {"windows"}, "Windows Rust module"),
    (("src/**",), RUST | PACKAGES | {"spm_bindings"}, "shared Rust core"),
    (("uniffi-bindgen.rs",), RUST | {"apple", "android", "spm_bindings"}, "UniFFI binding generator"),
    (("tests/live-idp/**",), frozenset(), "manual live-IdP fixture, not run in CI"),
    (("tests/**", "examples/**"), RUST, "Rust tests or examples"),
    # License inputs.
    (("deny.toml",), {"licenses"}, "cargo-deny policy"),
    (("about.toml", "about.hbs"), {"licenses"} | PACKAGES, "third-party notice template"),
    (("LICENSE",), PACKAGES, "LICENSE shipped in every package"),
    # Swift package and Apple adapters.
    (("Package.swift",), {"apple", "spm_manifest"}, "Swift package manifest"),
    (("Sources/NativeOIDC/native_oidc.swift",), {"apple", "spm_bindings"}, "generated Swift bindings"),
    (("Sources/**", "SwiftTests/**", "native/apple/**", "native/consumer-smoke/apple/**"),
     {"apple"}, "Apple adapters, tests or consumer"),
    (("docs/swift-xcframework.md",), {"apple"}, "README shipped in the Apple package"),
    (("docs/windows-cpp-guide.md",), {"windows"}, "C++ guide compiled by the Windows consumer"),
    # Android and Windows adapters. native/windows/README.md is shipped in
    # the Windows package, so it is not treated as documentation.
    (("native/android/**", "native/consumer-smoke/android/**"), {"android"}, "Android adapter, tests or consumer"),
    (("native/windows/**", "native/consumer-smoke/windows/**"), {"windows"}, "Windows adapter or consumer"),
    # Scripts. The Tests job runs every scripts/test_*.py, so each script
    # also selects it.
    (("scripts/generate-bindings.sh",), {"test", "apple", "android", "spm_bindings"}, "binding generation script"),
    (("scripts/generate-third-party-licenses.sh",), {"test", "licenses"} | PACKAGES, "license notice script"),
    (("scripts/check-spm-bindings.sh",), {"test", "spm_bindings"}, "Swift bindings check"),
    (("scripts/check-spm-manifest.sh", "scripts/update-package-swift.py"), {"test", "spm_manifest"},
     "Swift package manifest check"),
    (("scripts/test_normalize_zip.py",), {"test", "apple"}, "zip normalizer tests (real ditto runs on macOS)"),
    (("scripts/check-apple-adapters.sh", "scripts/check-apple-release-consumer.sh",
      "scripts/check-ios-authorization-flow.sh", "scripts/check-spm-package.sh",
      "scripts/install-ios-simulator-runtime.sh", "scripts/ios-mock-oauth-provider.py",
      "scripts/normalize_xcframework_plist.py", "scripts/normalize_zip.py",
      "scripts/package-apple-xcframework.sh", "scripts/select_ios_simulator.py"),
     {"test", "apple"}, "Apple CI script"),
    (("scripts/check-android-targets.sh", "scripts/check-android-release-consumer.sh",
      "scripts/check-android-maven-publication.sh"),
     {"test", "android"}, "Android CI script"),
    (("scripts/package-windows.ps1", "scripts/check-windows-release-consumer.ps1"),
     {"test", "windows"}, "Windows CI script"),
    # Used only by the release workflows, which keep their full checks, or
    # by the manual real-browser verification; the Tests job runs their
    # unit tests.
    (("scripts/android_maven_central_release.py", "scripts/download-release-aar.sh",
      "scripts/check-android-maven-consumer.sh", "scripts/check-android-maven-publication-status.sh",
      "scripts/verify-android-maven-consumer.sh", "scripts/check-gpg-keyserver.sh",
      "scripts/check-macos-real-browser.sh", "scripts/check-windows-real-browser.ps1",
      "scripts/real_browser_mock_provider.py", "scripts/test_*.py"),
     {"test"}, "release-only or manual script, or script unit test"),
    # Documentation that no build, package or check reads.
    (("README.md", "CHANGELOG.md", "ROADMAP.md", "FEATURE_MATRIX.md", "docs/**", ".gitignore"),
     frozenset(), "documentation"),
)

ZERO_SHA = "0" * 40
# Keeps the PR report comment well under GitHub's comment size limit.
MAX_LISTED_FILES = 200


def _pattern_regex(pattern):
    parts = []
    for token in re.split(r"(\*\*|\*)", pattern):
        if token == "**":
            parts.append(".*")
        elif token == "*":
            parts.append("[^/]*")
        else:
            parts.append(re.escape(token))
    return re.compile("".join(parts))


_COMPILED = tuple(
    (tuple(_pattern_regex(pattern) for pattern in patterns), frozenset(jobs), reason)
    for patterns, jobs, reason in RULES
)


def classify(path):
    """Return (jobs, reason) for one repository-relative path."""
    for patterns, jobs, reason in _COMPILED:
        if any(pattern.fullmatch(path) for pattern in patterns):
            return jobs, reason
    return ALL, "unclassified path"


def select(paths):
    """Return {job: [reasons]} for the jobs that `paths` select."""
    selected = {}
    for path in paths:
        jobs, reason = classify(path)
        for job in jobs:
            selected.setdefault(job, [])
            if reason not in selected[job]:
                selected[job].append(reason)
    return selected


def select_all(reason):
    return {job: [reason] for job in JOBS}


def _git(*args):
    return subprocess.run(("git",) + args, check=True, capture_output=True, text=True).stdout


def _commit_exists(sha):
    return subprocess.run(("git", "cat-file", "-e", f"{sha}^{{commit}}"), capture_output=True).returncode == 0


def diff_range(event_name, event):
    """Return (base, head, None) for the change to test, or (None, None, reason)."""
    if event_name == "pull_request":
        # actions/checkout checks out the PR's test merge commit, whose first
        # parent is the base branch it would merge into.
        parents = _git("rev-list", "--parents", "-n", "1", "HEAD").split()
        if len(parents) != 3:
            return None, None, "pull request checkout is not a merge commit"
        return parents[1], "HEAD", None
    if event_name == "push":
        before = event.get("before") or ""
        if not before or before == ZERO_SHA:
            return None, None, "first push of the branch (no previous commit)"
        if not _commit_exists(before):
            return None, None, f"previous commit {before} is not available"
        if subprocess.run(("git", "merge-base", "--is-ancestor", before, "HEAD"), capture_output=True).returncode:
            return None, None, f"previous commit {before} is not an ancestor (history rewritten)"
        return before, "HEAD", None
    return None, None, f"event `{event_name}` has no base to compare"


def changed_paths(base, head):
    output = _git("diff", "--name-only", "--no-renames", "-z", base, head)
    return [path for path in output.split("\0") if path]


def compute(event_name, event):
    """Return (selected, context) where context describes what was compared."""
    try:
        base, head, reason = diff_range(event_name, event)
        if reason:
            return select_all(reason), {"mode": "all", "reason": reason, "files": []}
        paths = changed_paths(base, head)
    except (subprocess.CalledProcessError, OSError) as error:
        reason = f"could not compute the diff ({error})"
        return select_all(reason), {"mode": "all", "reason": reason, "files": []}
    base_sha = _git("rev-parse", base).strip()
    head_sha = _git("rev-parse", head).strip()
    return select(paths), {"mode": "diff", "base": base_sha, "head": head_sha, "files": paths}


def report_markdown(selected, context):
    lines = ["#### Job selection", ""]
    if context["mode"] == "all":
        lines.append(f"All jobs run: {context['reason']}.")
    else:
        files = context["files"]
        lines.append(
            f"Compared `{context['base'][:12]}`..`{context['head'][:12]}`: {len(files)} changed file(s)."
        )
    lines += ["", "| Job group | Selected | Why |", "| --- | --- | --- |"]
    for job in JOBS:
        if job in selected:
            lines.append(f"| {job} | yes | {'; '.join(selected[job])} |")
        else:
            lines.append(f"| {job} | no | no changed file affects it |")
    if context["mode"] == "diff":
        lines += ["", "<details><summary>Changed files</summary>", ""]
        for path in context["files"][:MAX_LISTED_FILES]:
            jobs, reason = classify(path)
            names = ", ".join(job for job in JOBS if job in jobs) or "none"
            lines.append(f"- `{path}` → {names} ({reason})")
        if len(context["files"]) > MAX_LISTED_FILES:
            lines.append(f"- … and {len(context['files']) - MAX_LISTED_FILES} more")
        lines += ["", "</details>"]
    return "\n".join(lines)


def _write_output(name, value):
    path = os.environ.get("GITHUB_OUTPUT")
    if not path:
        print(f"{name}={value}")
        return
    with open(path, "a", encoding="utf-8") as output:
        if "\n" in value:
            delimiter = "EOF_SELECT_CI_JOBS"
            output.write(f"{name}<<{delimiter}\n{value}\n{delimiter}\n")
        else:
            output.write(f"{name}={value}\n")


def command_select():
    event_name = os.environ.get("GITHUB_EVENT_NAME", "")
    event = {}
    event_path = os.environ.get("GITHUB_EVENT_PATH")
    if event_path:
        with open(event_path, encoding="utf-8") as handle:
            event = json.load(handle)
    selected, context = compute(event_name, event)
    report = report_markdown(selected, context)
    for job in JOBS:
        _write_output(job, "true" if job in selected else "false")
    _write_output("selection", json.dumps({job: job in selected for job in JOBS}))
    _write_output("report", report)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(report + "\n")
    return 0


# Workflow job id -> the group whose selection decides whether it runs.
JOB_GROUPS = {
    "msrv": "msrv",
    "test": "test",
    "lint": "lint",
    "coverage": "coverage",
    "licenses": "licenses",
    "spm_bindings": "spm_bindings",
    "spm_manifest": "spm_manifest",
    "apple_adapters": "apple",
    "android_adapters": "android",
    "windows_core": "windows",
    "windows_client": "windows",
}


def verify(needs):
    """Return (ok, rows); rows are (job, result, verdict) for the needs given."""
    changes = needs.get("changes")
    rows = []
    ok = True
    selection_known = changes is not None and changes.get("result") == "success"
    if changes is not None and not selection_known:
        ok = False
        rows.append(("changes", changes.get("result", "missing"),
                     "job selection did not succeed, so every job was required to run"))
    outputs = changes.get("outputs", {}) if selection_known else {}
    for job, need in needs.items():
        if job == "changes":
            continue
        group = JOB_GROUPS.get(job)
        if group is None:
            ok = False
            rows.append((job, need.get("result", "missing"), "unknown job: add it to JOB_GROUPS"))
            continue
        expected = outputs.get(group) != "false" if selection_known else True
        result = need.get("result", "missing")
        if result == "success":
            rows.append((job, result, "passed"))
        elif result == "skipped" and not expected:
            rows.append((job, result, "skipped as intended: not affected by the change"))
        elif result == "skipped":
            ok = False
            rows.append((job, result, "skipped although selected (a job it needs did not pass)"))
        elif result == "cancelled":
            ok = False
            rows.append((job, result, "cancelled"))
        else:
            ok = False
            rows.append((job, result, "failed"))
    return ok, rows


def command_verify():
    needs = json.loads(os.environ.get("NEEDS_JSON") or "{}")
    if not needs:
        print("NEEDS_JSON is empty: no job results to check", file=sys.stderr)
        return 1
    ok, rows = verify(needs)
    lines = ["| Job | Result | Verdict |", "| --- | --- | --- |"]
    lines += [f"| {job} | {result} | {verdict} |" for job, result, verdict in rows]
    table = "\n".join(lines)
    print(table)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(table + "\n")
    if not ok:
        print("::error::One or more CI jobs failed, were cancelled, or were skipped although selected.")
    return 0 if ok else 1


def main(argv):
    commands = {"select": command_select, "verify": command_verify}
    if len(argv) != 2 or argv[1] not in commands:
        print(f"usage: {argv[0]} select|verify", file=sys.stderr)
        return 2
    return commands[argv[1]]()


if __name__ == "__main__":
    sys.exit(main(sys.argv))

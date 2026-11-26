# CI job selection

The Rust CI workflow (`.github/workflows/ci.yml`) runs only the jobs that a
change can affect. A documentation-only pull request does not start the macOS,
Windows and Android validation, while a change to the shared Rust core still
runs every platform. The release workflows (`release.yml`,
`release-consumer.yml`) are not affected: they always run their full
validation.

The rules are in [`scripts/select_ci_jobs.py`](../scripts/select_ci_jobs.py)
(`RULES`), and their unit tests in
[`scripts/test_select_ci_jobs.py`](../scripts/test_select_ci_jobs.py) cover the
representative cases. This page explains them; when the two differ, the
script is what CI runs, and both must be updated together.

## How a run is decided

1. The `Select CI jobs` job (`changes`) lists the changed files (renames count
   as both the old and the new path).
2. Each path is matched against the rules in order; the first matching rule
   decides which job groups that path needs.
3. The selected groups are the union over all changed paths. Each group is
   one `changes` output (`true` or `false`). A job is skipped only when
   `Select CI jobs` succeeded and set its output to `false`.

The selector is conservative. It skips a job only when it can name why the
change cannot affect it. Every job runs when:

- a path matches no rule ("unclassified path"),
- the change touches `.github/**`, the selector or its tests, `Cargo.toml`,
  `Cargo.lock` or `.gitattributes`,
- the diff cannot be computed (see below), or
- the `Select CI jobs` job itself fails, even if it wrote some outputs before
  failing.

### What is compared

| Event | Compared range |
| --- | --- |
| Pull request (same repository or fork) | The PR's test merge commit against its first parent, the base branch it merges into. Every push to the PR is judged on the whole PR, not only on the last push. Fork PRs need no token for this. |
| Push to `main` | The pushed commit against `before`, the commit `main` pointed to previously. |
| First push of a branch (`before` is all zeros), `before` not in the history, or history rewritten (`before` is not an ancestor) | Nothing: every job runs. |
| Any other event | Nothing: every job runs. |

## Job groups and their inputs

| Group | Jobs (check names) |
| --- | --- |
| `msrv` | MSRV |
| `test` | Tests (Rust tests, rustdoc, the release documentation set build and check, binding generation, Swift package bindings match, `scripts/test_*.py`) |
| `lint` | Formatting and Clippy |
| `coverage` | Coverage |
| `licenses` | Dependency licenses |
| `spm_manifest` | Swift package manifest is consistent |
| `apple` | Apple adapters validation (iOS 18, iOS latest), and the required `Apple adapters validation` check that collects them |
| `android` | Android adapters validation (unit tests, API 28 and API 35 emulator tests, release AAR and Maven publication checks) |
| `windows` | Windows core target validation, Windows 11 client consumers (x64 emulation) |

"Rust" below means `msrv`, `test`, `lint` and `coverage`; "packages" means
`apple`, `android` and `windows`, the jobs that build and check a release
package.

| Changed path | Groups | Why |
| --- | --- | --- |
| `.github/**` | all | A workflow change can change any check, so it is not judged by the rules it may itself break. |
| `scripts/select_ci_jobs.py`, `scripts/test_select_ci_jobs.py` | all | The selector itself. |
| `Cargo.toml`, `Cargo.lock` | all | Dependencies, features and version feed every build, the license notices and the Swift package manifest check. |
| `.gitattributes` | all | Line endings of checked-out build inputs (for example `about.hbs`). |
| `src/windows/**` | Rust, `windows` | Compiled only for Windows and for tests (`#[cfg(any(windows, test))]` in `src/lib.rs`), so it is in neither the Apple and Android libraries nor the generated bindings. |
| `src/**` (other) | Rust, packages | The shared core every platform links, and the source of the UniFFI bindings. |
| `uniffi-bindgen.rs` | Rust, `apple`, `android` | Generates the Swift and Kotlin bindings. |
| `tests/live-idp/**` | none | Manual live-IdP fixture; no CI job uses it. |
| `tests/**`, `examples/**` | Rust | Rust integration tests and examples. |
| `deny.toml` | `licenses`, `test` | `cargo deny` reads it, and the release documentation set ships it. |
| `about.toml`, `about.hbs` | `licenses`, packages | Every package ships the generated third-party notices. |
| `LICENSE` | packages | Shipped in every package, and the package checks look for it. |
| `Package.swift` | `apple`, `spm_manifest` | Swift package build and manifest check. |
| `Sources/NativeOIDC/native_oidc.swift` | `apple`, `test` | Generated bindings, compared with a fresh generation. The drift diff now runs in the `Tests` job, so `native_oidc.swift` (the checked-in copy) selects `test` as well as `apple`. |
| `Sources/**`, `SwiftTests/**`, `native/apple/**`, `native/consumer-smoke/apple/**` | `apple`, `test` | Apple adapters, XCTest (including the tests #167 moves there), smoke tests and the release consumer. The guides link some of these files, so the release documentation set ships them. |
| `docs/swift-xcframework.md` | `apple`, `test` | Shipped as the Apple package's README, and in the release documentation set. |
| `docs/windows-cpp-guide.md` | `windows`, `test` | The Windows C++ consumer compiles and runs the guide's `cpp` blocks; the release documentation set ships it. |
| `docs/windows-dotnet-guide.md` | `windows`, `test` | The Windows .NET guide fixture compiles and runs the guide's `csharp` blocks; the release documentation set ships it. |
| `native/android/**`, `native/consumer-smoke/android/**` | `android`, `test` | Android adapter, unit and emulator tests (including the API 35 run from #166), and the release consumer. The guides link some of these files. |
| `native/windows/**`, `native/consumer-smoke/windows/**` | `windows`, `test` | Windows header, .NET wrapper, consumers. `native/windows/README.md` is shipped in the Windows package; the header, the wrapper and the guide examples are in the release documentation set. |
| `scripts/generate-bindings.sh` | `test`, `apple`, `android` | Binding generation. |
| `scripts/generate-third-party-licenses.sh` | `test`, `licenses`, packages | License notices. |
| `scripts/check-spm-bindings.sh` | `test` | Swift bindings check, now runs inside the `Tests` job after binding generation reuses its output. |
| `scripts/check-spm-manifest.sh`, `scripts/update-package-swift.py` | `test`, `spm_manifest` | |
| `scripts/test_normalize_zip.py` | `test`, `apple` | Its real-`ditto` tests run only on macOS. |
| Apple CI scripts (`apple-phase-timing.sh`, `check-apple-*`, `check-ios-*`, `check-spm-package.sh`, `install-ios-simulator-runtime.sh`, `ios-mock-oauth-provider.py`, `normalize_*.py`, `package-apple-xcframework.sh`, `select_ios_simulator.py`) | `test`, `apple` | |
| Android CI scripts (`check-android-targets.sh`, `check-android-release-consumer.sh`, `check-android-maven-publication.sh`) | `test`, `android` | |
| Windows CI scripts (`package-windows.ps1`, `check-windows-release-consumer.ps1`) | `test`, `windows` | |
| Release-only scripts (`check-android-maven-consumer.sh`, `check-android-maven-publication-status.sh`, `verify-android-maven-consumer.sh`, `check-gpg-keyserver.sh`), manual real-browser scripts (`check-macos-real-browser.sh`, `check-windows-real-browser.ps1`, `real_browser_mock_provider.py`) and other `scripts/test_*.py` | `test` | The release workflows or a maintainer run them; the Tests job runs their unit tests. The real-browser check sources (`native/apple/MacOSRealBrowserCheck.swift`, `native/consumer-smoke/windows/real-browser/**`) follow their platform's rule: the Apple job type-checks the Swift file, and the Windows project is kept under the Windows consumer rule. |
| `scripts/build-release-docs.py`, `scripts/check-release-docs.py` | `test` | The Tests job builds and checks the release documentation set with them; the release workflows run them too. |
| `scripts/agent-precheck.sh` | none | Precheck for the agent workflow automations; only Orca runs it, and no build, package or check reads it. |
| Any other file under `scripts/` | all | Unclassified. |
| `docs/agent-workflows/**` | none | Agent workflow notes; the documentation index does not link them, so they are not in the release documentation set. |
| `README.md`, `ROADMAP.md`, `FEATURE_MATRIX.md`, `docs/**` (except the rules above) | `test` | The release documentation set: the Tests job builds it and checks every link. |
| `CHANGELOG.md`, `.gitignore` | none | No build, package or check reads them. |
| Anything else | all | Unclassified. |

Documentation is not excluded by file extension: a Markdown file that a
package ships (`docs/swift-xcframework.md`, `native/windows/README.md`) or a
file that a check reads (`docs/windows-cpp-guide.md`, `docs/windows-dotnet-guide.md`) is classified with the job that uses it. The unit
test `test_every_tracked_file_is_classified` fails when a tracked file
matches no rule, so a new file needs a rule (or knowingly runs everything).

The release documentation set is every file reachable by relative links
from `docs/README.md` ([`scripts/build-release-docs.py`](../scripts/build-release-docs.py)),
so the documentation and the source files it links all feed the `test`
job. The unit test `test_release_documentation_set_inputs_run_tests`
computes that set from the checkout and fails when one of its files does
not select `test`, so a guide that starts linking a new file needs its rule
updated.

## Skips, failures and required checks

GitHub reports a job skipped by its `if` condition as successful, including
for a required check. A skip is therefore safe only when it was intended,
which the workflow checks in two places:

- **`Apple adapters validation`** (required) collects both iOS
  configurations. It passes when both pass, or when both were skipped because
  the selection set `apple` to `false`.
- **`CI result`** collects every job. It passes only when each job either
  passed or was skipped because its group was not selected. It fails, and
  names the job, when a job failed, was cancelled, or was skipped although
  selected (for example the Windows 11 client consumers when the Windows core
  job failed), and when the `Select CI jobs` job did not succeed.

Both run `scripts/select_ci_jobs.py verify` with the workflow's `needs`
context, and both run with `always()`, so a cancelled run makes them fail
instead of leaving them pending.

The other required checks keep their names and are each a single job, so a
failure in one of them is reported by that check. When the selection fails,
they all run. The one remaining gap is a run cancelled while
`Select CI jobs` is still running: the jobs waiting for it are then skipped,
which GitHub shows as passed, while `CI result` and `Apple adapters
validation` fail. Requiring `CI result` in the branch protection closes that
gap; it is the only check that covers every job, including the ones that
are not required individually.

## PR report

The `PR report` comment marks each check as passed, failed, `⏭️ skipped (not
affected)` (an intended skip) or `⚠️ skipped` (any other skip), and adds the
job selection: the compared commits, each group with the reasons it was
selected, and each changed file with its groups. The same selection is
written to the `Select CI jobs` job summary, and each check of the run is in
the `CI result` job summary.

## Changing the rules

1. Edit `RULES` in `scripts/select_ci_jobs.py`, placing a specific rule
   before a directory-wide rule that would also match.
2. Add a test case to `scripts/test_select_ci_jobs.py`.
3. Update the table above.
4. A new job in `ci.yml` needs a group (a `changes` output and an entry in
   `JOBS`), an `if` on that output, an entry in `JOB_GROUPS`, and a place in
   the `needs` of `CI result` and `PR report`. `CI result` fails on a job it
   does not know.

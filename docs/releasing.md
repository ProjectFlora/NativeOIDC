# Releasing

This is the maintainer procedure for publishing a version of NativeOIDC.
Every release is produced by the [Release workflow](../.github/workflows/release.yml),
which is dispatched manually. Do not push version tags by hand: the workflow
creates the tag itself (see [Why CI creates the tag](#why-ci-creates-the-tag)).

## Overview

| Step | Where | Result |
| --- | --- | --- |
| 1. Prepare | Pull request to `main` | Version, changelog, and docs updated |
| 2. Candidate run | Release workflow, `publish: false` | All packages built and consumer-validated; nothing published |
| 3. Publish run | Release workflow from `main`, `publish: true` | Release commit, `vX.Y.Z` tag, GitHub Release with assets and `SHA256SUMS`, and the Android library on Maven Central |
| 4. Verify | Published release consumer validation workflow; real system-browser verification on a Mac and a Windows machine | Published assets and the Swift package resolve and build; the real-browser result is recorded |

Every release run — a candidate or a publish — starts with a key server
check: it derives the fingerprint of every primary key in the
`GPG_PRIVATE_KEY` secret and requires each to resolve at `keys.openpgp.org`,
retrying briefly to absorb key-server propagation lag. Maven Central's sync
validation fetches the signing key's public key from the key server, so a
key that is not there yet fails the run before any package is built rather
than after a full build ([Repository settings](#repository-settings)). The
check is fail-safe: it also fails when the secret holds only public key
material (nothing to sign with) or a key it cannot fingerprint (an
unsupported version, or a truncated or malformed packet), so those break the
run early instead of at the Maven signing step.

## 1. Prepare the release

In a pull request to `main`:

- Set `version` in `Cargo.toml` to the release version and update
  `Cargo.lock` (`cargo update -p native-oidc`). The workflow rejects any
  version that does not match the crate version.
- Move the `CHANGELOG.md` `[Unreleased]` entries under the new version.
- Update README's download section, the SPM installation version (the
  `from:` version and Xcode dependency rule), the Maven Central
  dependency version, and any version-specific notes.
- If dependencies changed, check that `THIRD_PARTY_LICENSES.txt` still
  reads correctly (CI uploads it as the `third-party-licenses` artifact;
  see [License notices](#license-notices)).
- Recheck Windows client support as required by the
  [support matrix](support-matrix.md). For 1.0.0, complete the
  [final verification record](support-matrix.md#final-100-verification).

Do not edit the `NativeOIDCBinary` URL or checksum in `Package.swift`; the
release workflow writes them into the release commit at the version tag.
`main`'s copy is not updated by a release (see
[Why CI creates the tag](#why-ci-creates-the-tag)).

To keep `main` closer to released versions, maintainers may optionally update
the binary target in a release preparation PR to point at the previous
published release. This is not required: the workflow will rewrite it at the
tag anyway, and `scripts/check-spm-manifest.sh` allows the manifest to lag
the Cargo version (see [Why CI creates the tag](#why-ci-creates-the-tag)).

Merge the pull request once CI passes.

## 2. Run a release candidate

A candidate run builds and validates every package without publishing
anything. It can be dispatched from any branch, which is how release
pipeline changes get checked before they merge — but only from `main` or
a fully trusted branch: the key server check holds the signing key
(`GPG_PRIVATE_KEY`) in its environment and runs the dispatched branch's
copy of the check script, so a branch that is not trusted with the private
signing key must not be dispatched. (Publish runs are already restricted
to `main` by the version check, so this concerns candidates only.)

The first job is the key server check (the signing key's public part must
resolve at `keys.openpgp.org`). If the key is not published, the run fails
there and the build jobs are skipped, so a missing key costs one job, not a
full build.

```sh
gh workflow run release.yml --ref main -f version=X.Y.Z -f publish=false
gh run watch "$(gh run list --workflow release.yml --limit 1 --json databaseId -q '.[0].databaseId')"
```

Or, in GitHub: **Actions > Release > Run workflow**, choose the branch, enter
the version, and leave **publish** unchecked.

Confirm that every job succeeds, including **Prepare and validate release
Package.swift** (`spm_manifest`). That job's log shows the rewritten
`Package.swift` diff: the URL must be
`https://github.com/ProjectFlora/NativeOIDC/releases/download/vX.Y.Z/NativeOIDC-Apple-X.Y.Z.zip`.
The **Validate release version** step summary names the release the
generated notes will start from and links the comparison; it must be the
previous release (see [Release notes range](#release-notes-range)).

The Apple jobs build and validate with the Xcode set by `XCODE_VERSION` in
`release.yml` (a stable Xcode 26.x, since App Store Connect requires Xcode
26 or later), and `release-consumer.yml` sets the same version. Check the
**Build Apple release package** step summary for the recorded Rust and Xcode
versions and the iOS simulator runtime the iOS flow ran on. To change the
Xcode version, update the runner label and `XCODE_VERSION` in both
workflows together, after checking the
[runner-images README](https://github.com/actions/runner-images/tree/main/images/macos)
for the image's installed Xcode versions and simulator runtimes.

When a change touches the Maven Central publication, add
`-f dry_run_maven=true` to the candidate run. The `dry_run_maven` job runs the
`publish_maven` steps against the real Maven Central and Central Portal, but
read-only. It adopts the AAR of the GitHub Release `vX.Y.Z` if one exists,
and otherwise the candidate AAR from the `android` job. It classifies the
version (`decide --read-only`, see [the Maven Central state](#the-maven-central-state)),
checks that the Portal accepts the token with a status query for a
deployment ID that does not exist, and builds and checks the signed bundle.
It never uploads, publishes, drops, or writes the deployment record. Its
step summary lists the decision, its evidence, and the bundle's files. Like
the key server check, it holds the signing key and the Portal token, so
dispatch it only from `main` or a fully trusted branch.

## 3. Publish

Before dispatching, confirm the signing key's public part is resolvable at
the key server. The run re-checks this with a short retry, but a freshly
published key can take minutes to hours to propagate through the pool:

```sh
curl -s -o /dev/null -w '%{http_code}\n' \
    "https://keys.openpgp.org/vks/v1/by-fingerprint/<fingerprint>"
```

The fingerprint is the one `gpg --list-keys --fingerprint` shows for the
key in the `GPG_PRIVATE_KEY` secret. Expect `200`.

Dispatch from `main` with **publish** checked:

```sh
gh workflow run release.yml --ref main -f version=X.Y.Z -f publish=true
```

The run rebuilds and revalidates everything; the archive it publishes, and
the checksum written into `Package.swift`, come from this run, not from the
candidate run in step 2, and its bytes may differ from the candidate's.
Then the `publish` job:

1. Commits the validated `Package.swift` as `release: vX.Y.Z` on top of the
   dispatched `main` commit (it fails if anything else changed).
2. Creates the annotated `vX.Y.Z` tag on that commit and pushes **only the
   tag**. `main` is not updated.
3. Creates the GitHub Release with the Apple, Android, Windows, and API docs
   assets plus `SHA256SUMS`, and release notes generated from the previous
   release (see [Release notes range](#release-notes-range)).

After the GitHub Release is created, the `publish_maven` job publishes
`net.project-flora.nativeoidc:native-oidc-android:X.Y.Z` to Maven Central:

1. It checks out the `vX.Y.Z` tag, downloads `native-oidc-android-X.Y.Z.aar`
   and `SHA256SUMS` from the GitHub Release, and checks the AAR against its
   `SHA256SUMS` line (`scripts/download-release-aar.sh`). This SHA-256 is the
   expected value for every later check.
2. It classifies the version (`scripts/android_maven_central_release.py decide`,
   see [the Maven Central state](#the-maven-central-state)). A version that
   is already complete on Maven Central with the same AAR passes here.
3. Otherwise it builds the signed Central bundle with
   `scripts/check-android-maven-publication.sh`. Gradle adopts the Release
   AAR instead of building one (`-PadoptedAar`, with the AAR build excluded),
   so the job runs no Rust NDK build. The POM, the module metadata, and the
   sources and javadoc jars are generated from the tag. The check requires
   the AAR, and its entry in the module metadata, to have the expected
   SHA-256, and requires every file's signature and `.md5`/`.sha1` checksums.
4. It uploads the bundle to the Central Portal (`android_maven_central_release.py upload`,
   `publishingType=AUTOMATIC`), records the deployment, and waits until the
   Portal starts publishing it. It does not wait for the Central sync; the
   consumer validation in step 4 does.

So Maven Central and the GitHub Release ship the same AAR bytes. The
candidate run and pull request CI run the same publication check on an
unsigned publication that builds its own AAR.

### The Maven Central state

`publish_maven` records each upload in the GitHub Release asset
`maven-central-deployment.json`: an `upload-started` attempt is written
before the bundle is posted, and the deployment ID once the Portal returns
it. If the record cannot be written, nothing is uploaded. The Gradle plugin's
own upload (`publishAndReleaseToMavenCentral`) is not used, because it does
not report the deployment ID. Only documented
[Publisher API](https://central.sonatype.org/publish/publish-portal-api/)
endpoints are called: upload and status. Nothing publishes or drops a
deployment; `AUTOMATIC` deployments publish themselves after validation.

`decide` reads Maven Central (the same ten files as the consumer
availability gate), the record, and the Portal status of the latest recorded
deployment:

| Maven Central | Latest record attempt | Portal status | Outcome |
| --- | --- | --- | --- |
| All ten files | Any | Not queried | The AAR on Central must have the expected SHA-256: `verified`, or a failure on a mismatch |
| Incomplete | None | Not queried | `upload` |
| Incomplete | `upload-started`, no ID | Not queried | Fail closed (see [Upload started but not recorded](#upload-started-but-not-recorded)) |
| Incomplete | Deployment ID | `PENDING`, `VALIDATING`, `VALIDATED`, `PUBLISHING` | Wait (30 × 60 s), then continue with the state reached; fail with "re-run later" if still in flight. `VALIDATED` that persists fails closed: it waits for a manual publish, which this pipeline never requests |
| Incomplete | Deployment ID | `PUBLISHED` | Wait for the ten files (30 × 60 s), then check the AAR: `verified`; fail with "re-run later" if the sync has not completed |
| Incomplete | Deployment ID | `FAILED` | Show the Portal's `errors`, mark the attempt `failed`, and `upload`. The deployment is not dropped, because Sonatype asks to keep failed deployments for support requests |
| Incomplete | `failed` | Not queried | `upload`, unless three attempts have failed: then fail closed, since the failure is not transient |
| Any | Deployment ID | Unknown ID, 4xx, or no answer after retries | Fail closed |

`upload` runs `decide` again and uploads only on `upload`. A run therefore
never uploads a version that is on Maven Central or has a deployment in
flight or published.

Publishing is refused unless the run is dispatched from `main`, the commit is
on `main`, and the `vX.Y.Z` tag does not already exist.

## 4. Verify the published release

Tags pushed by the workflow's `GITHUB_TOKEN` do not trigger other workflows,
so run the published consumer checks manually:

```sh
gh workflow run release-consumer.yml --ref main -f version=X.Y.Z
```

This downloads each published asset, verifies it against `SHA256SUMS`, and
runs the Apple, Android, and Windows consumers against the downloaded
archives. Its `android_maven` job additionally consumes the Android library
from Maven Central the way a foreign Gradle consumer would: a
publication-status check fails fast when the version was never published to
Central (releases before `publish_maven` was added in #132, for example
v0.5.0, or a release whose `publish_maven` job failed or was cancelled --
the tag is pushed before that job runs, so the check reads the release
run's job results, and checks Maven Central itself before rejecting a
failed publication: a visible or still-syncing artifact is let through,
and an incomplete or in-progress outcome falls back to the retry loop),
then a retry loop absorbs the Central sync -- a direct HTTP check
on the full publication (the AAR, the POM, the module metadata, the
sources and javadoc jars, and their signatures), a strict
`checkDebugAarMetadata` resolution of the POM, AAR and transitive
dependencies, and the full consumer smoke on an emulator (Issue #137).

Its `spm` job resolves the `vX.Y.Z` tag through Swift Package Manager by URL, which also verifies the
manifest's checksum against the published archive. Record the run URL in
the release tracking issue.

CI does not open a real system browser on macOS or Windows. Run the
[real system-browser verification](real-browser-verification.md) on a Mac
and a Windows machine at the `vX.Y.Z` tag (on Windows, with the published
`native-oidc-windows-X.Y.Z.zip`), and record the result in the release
tracking issue in the format that document gives.

## License notices

Every release package ships `LICENSE`, `THIRD_PARTY_LICENSES.txt` and
`RUST_STANDARD_LIBRARY_COPYRIGHT.html` (see the README's
[Third-party licenses](../README.md#third-party-licenses) section for the
locations). The notices are generated, never edited by hand:

- `scripts/generate-third-party-licenses.sh` runs cargo-about with
  `about.toml` (which crates and targets: the crates linked into the
  release libraries for every release target, without dev and build
  dependencies) and `about.hbs` (the text, including the Rust standard
  library and UniFFI-generated bindings sections). It runs offline and
  requires the pinned cargo-about version.
- The Rust standard library is not in `Cargo.lock`, so the script takes its
  notices from the active Rust toolchain: it appends the toolchain version
  and the license files of compiler-builtins and libm (the parts not under
  `MIT OR Apache-2.0`, with copyright notices of their own) from the
  rust-src component, which it installs, and copies the toolchain's
  `share/doc/rust/COPYRIGHT-library.html` as
  `RUST_STANDARD_LIBRARY_COPYRIGHT.html`. The output therefore depends on
  `Cargo.lock` and the toolchain, and each package generates it with the
  toolchain that builds it. The release workflow pins every package job to
  the same Rust version (`dtolnay/rust-toolchain@1.100.0`), so all packages
  ship the same notices; change them together.
- The Apple packaging script, the Android release build
  (`generateLicenseNotices`) and `scripts/package-windows.ps1` (after the
  workflow step that runs the script) put the files into the packages.
  The consumer checks and `scripts/check-android-maven-publication.sh`
  fail when a package lacks them; the Android consumer check also requires
  them in the consumer app's APK.
- `cargo deny check licenses` (`deny.toml`) runs in CI and in the release
  workflow's version validation. It allows MPL-2.0 only for the UniFFI
  crates and CDLA-Permissive-2.0 only for `webpki-roots`.

Maintenance:

- **A dependency under a new license**: check that the license is
  compatible with Apache-2.0 and what it requires of binary distribution,
  then add it to `deny.toml` (as a per-crate exception when it should stay
  limited to that crate) and to `accepted` in `about.toml`.
- **A `ring` update**: `about.toml` pins the checksums of ring's license
  files, so generation fails until they are reviewed and the checksums
  updated.
- **A Rust toolchain update**: check that the toolchain still has the two
  license files the script appends, and whether another part of the
  standard library gained a license other than `MIT OR Apache-2.0` (its
  `library/` sources in rust-src).
- **A cargo-about or cargo-deny update**: change the version in
  `scripts/generate-third-party-licenses.sh` (cargo-about) and in every
  `taiki-e/install-action` step of `ci.yml` and `release.yml` together.

`release-consumer.yml` runs the same consumer checks, so it requires the
notices too and can validate only v0.7.1 and later. v0.1.0 through v0.7.0
shipped without them; their notices were generated afterwards from each
tag's `Cargo.lock` and the Rust version of each release run, and attached
to the GitHub Releases as `THIRD_PARTY_LICENSES-<version>.txt` and
`RUST_STANDARD_LIBRARY_COPYRIGHT-<version>.html`, leaving `SHA256SUMS`
unchanged
([#161](https://github.com/ProjectFlora/NativeOIDC/issues/161)).

Its Windows job checks out the release tag instead of the dispatched ref,
so the Windows consumer script and fixtures match the package under test.
The success-path fixtures added for
[#165](https://github.com/ProjectFlora/NativeOIDC/issues/165) need a
loopback HTTP revocation endpoint, which releases before that change
reject.

## Why CI creates the tag

Swift Package Manager reads `Package.swift` at the version tag, and that
manifest must contain the SHA-256 checksum of the published XCFramework
archive. The checksum is a build output: it is only known after the
archive is actually built, so a tag pushed before the build can never
contain it. The workflow therefore builds and validates the archive first
and then tags a release commit that carries it:

```text
main ── A ── B            (B: dispatched commit)
               \
                R          release: vX.Y.Z  (Package.swift only)  ← tag vX.Y.Z
```

The release commit is reachable only from its tag; the workflow never
rewrites `main`'s `Package.swift`. `main` therefore keeps the binary target
last committed to it by hand, and it falls further behind the Cargo version
with every release unless someone updates it.
`scripts/check-spm-manifest.sh` accepts any such lag and rejects only a
manifest that leads the Cargo version. Releases up to 0.6.0 were published
while the repository was named `ProjectFlora/AppAuth-Rust`; the check also
accepts their original asset URLs, which GitHub redirects after the rename
([#146](https://github.com/ProjectFlora/NativeOIDC/issues/146)), so
renaming the repository does not invalidate the binary target on `main`.
SPM consumers resolve released
versions by tag and are unaffected; depending on the `main` branch through
SPM is not supported.

### Updating main's Package.swift

To keep `main` closer to released versions, maintainers may optionally update
the binary target in a release preparation PR to point at the previous
published release (e.g., for preparing v0.8.0, use v0.7.0's URL and checksum).

- Update both `url` and `checksum` together; do not edit them manually after
  that.
- Merge the PR once CI passes (`scripts/check-spm-manifest.sh` will succeed if
  the manifest does not lead the Cargo version).

Once main catches up to a prior release (e.g., v0.7.x), legacy URL acceptances
in `scripts/check-spm-manifest.sh` will no longer be needed but are retained
for safety until then.

### Release notes range

Because each release commit is reachable only from its tag, earlier release
tags are not in the new tag's history. Left to choose the starting tag
itself, GitHub picks the last tag in that history, which is `v0.4.0`, the
last release tagged on `main`, so every release would list everything
since `v0.4.0`. The workflow therefore passes the starting tag explicitly
(`--notes-start-tag`): the highest published, non-draft,
non-prerelease GitHub Release whose `vX.Y.Z` tag sorts below the new
version by SemVer. A patch for an older line (for example `v0.6.1` after
`v0.7.0`) therefore starts at `v0.6.0`, and a tag left without a GitHub
Release by a failed run is skipped. If no earlier release is found, the
run fails rather than generating notes from the wrong range. GitHub
compares the two tags through their merge base, so the notes list the pull
requests merged to `main` between the two dispatched commits.
Releases v0.6.0 and v0.7.0 were published with notes starting at `v0.4.0`
and were corrected afterwards
([#151](https://github.com/ProjectFlora/NativeOIDC/issues/151)).

### What the packaging self-test checks

This section is the reference description of the determinism self-test in
`scripts/package-apple-xcframework.sh`; other documents link here rather
than restating it.

The release run builds the Rust libraries and stages the package once.
The script then repeats two steps and requires identical bytes:

- `xcodebuild -create-xcframework` runs a second time, and both
  canonicalized `Info.plist` files must match
  (`scripts/normalize_xcframework_plist.py`).
- `ditto` zips the same staged tree a second time, and both archives must
  have the same SHA-256 after `scripts/normalize_zip.py`.

Staging (`cp`, `lipo`, `xattr -cr`, `touch -t`) runs once, and both
comparisons run in the same job, on the same machine, with the same
toolchain. Only the zip comparison runs under `TZ=UTC`; the plist
comparison runs earlier, in the runner's local timezone. The self-test
therefore shows only that the plist and zip steps repeat byte-identically
within one run. It does not show that a rebuild, a repeated staging pass,
or a run on another machine would produce the same archive. The manifest
checksum does not rely on that: it is computed from the one archive that
is published.

Each self-test output is removed as soon as its comparison succeeds, and
an EXIT trap removes whatever exists when the script exits any other way,
so a run never leaves its self-test outputs behind. The guard checks the
whole output directory rather than just the staged package, and the
directory remains after any run — successful, failed, or interrupted —
so re-running requires removing it in full
(`rm -rf target/apple/release-package`). The removal also deletes archives
of previous runs, so move any archive you keep somewhere else first.

## Recovering from a failed run

- **Key server check failed** (no package was built): the job fails when a
  signing key's public part is not resolvable at `keys.openpgp.org` (the log
  names the fingerprint and the lookup URL; upload the ASCII-armored public
  key at <https://keys.openpgp.org/upload>, wait for the pool to propagate,
  confirm the `200` with the curl check in step 3, and dispatch the run
  again), or when the secret cannot be used for signing as-is — it holds
  only public key material, or a key the check cannot fingerprint
  (unsupported version, truncated or malformed packet); the log names the
  reason. In the latter cases, fix the repository secret
  (`gpg --armor --export-secret-keys <fpr>`) and dispatch the run again.

A publish run moves through these states. Find the state from the failed
job and the evidence it names, then follow its procedure. **Re-run failed
jobs** reuses the run's built artifacts and job outputs. GitHub allows it
for 30 days after the run; after that, a version whose tag exists cannot be
published again, so fix forward with a new patch version.

| State | How to recognize it | Resume |
| --- | --- | --- |
| Before the tag | A job before **Create release commit and tag** failed | Nothing was published. Fix the cause and dispatch the workflow again |
| Tag pushed, no GitHub Release | **Generate checksums and publish assets** failed; `vX.Y.Z` exists, the Release does not | SPM cannot resolve the tag, because its binary asset does not exist. Delete the tag (`git push origin :refs/tags/vX.Y.Z`), then **Re-run failed jobs**, so the same built artifacts are reused |
| GitHub Release created, no upload started | `publish_maven` failed before **Upload to Maven Central**, or `upload` failed before writing the record; the Release has no `maven-central-deployment.json` | Fix the cause (credentials, signing key), then **Re-run failed jobs**. The job adopts the Release AAR again; no rebuild |
| Upload started but not recorded | `decide` fails with "deployment ID was never recorded" | See [below](#upload-started-but-not-recorded) |
| Deployment validating or publishing | `decide` or `upload` fails with "still … after the wait" | **Re-run failed jobs** later. The job resumes from the recorded deployment and uploads nothing |
| Deployment failed | `upload` fails with "failed validation" and the Portal's `errors` | Fix the cause if it is outside the bundle (for example, the key not on the key server), then **Re-run failed jobs**: the job uploads a new deployment. A problem in the bundle itself needs a code change, so fix forward with a new patch version. After three failed attempts the job refuses to upload |
| Published, sync propagating | `decide` fails with "Maven Central still lacks files" | Nothing to fix. **Re-run failed jobs** later, or dispatch `release-consumer.yml` once the files are visible |
| Fully published | `decide` reports `verified` | Done. A version on Maven Central cannot be replaced or deleted. If the AAR on Central differs from the Release asset, the job fails and the version must be fixed forward |

- **Release published with a problem**: do not move or recreate the tag or replace its assets. Consumers may already have resolved it, and SPM caches the checksum. Fix forward with a new patch version.

### Upload started but not recorded

The record says an upload began, but the run stopped before the deployment
ID was written: the bundle may or may not have reached the Portal. The job
fails closed instead of uploading a second deployment. To resume:

1. Look in the failed run's **Upload to Maven Central** log for
   `Uploaded the bundle as Central Portal deployment <id>`, or at
   <https://central.sonatype.com/publishing/deployments> for a deployment
   named `net.project-flora.nativeoidc-native-oidc-android-X.Y.Z`.
2. Download the record (`gh release download vX.Y.Z --pattern maven-central-deployment.json`)
   and edit its last attempt:
   - If a deployment exists, set `"state": "uploaded"` and
     `"deploymentId": "<id>"`.
   - If none exists, set `"state": "failed"`.
3. Upload it back (`gh release upload vX.Y.Z maven-central-deployment.json --clobber`)
   and **Re-run failed jobs**.

- **Consumer validation (`release-consumer.yml`) failed**: the `android_maven`
  job logs four distinct causes. The publication-status check failing means
  the version never reached Maven Central: a release predating #132 or a
  mistyped version has no Maven publication at all, so dispatch a published
  version; a release whose `publish_maven` job failed or was cancelled
  fails the check only when the artifact is also absent from Maven Central
  (the job can fail after the Central Portal deployment was released):
  resume the release run as the state table above says (remove the tag
  first if the tag-pushing job also did not complete); if the deployment
  was published, re-dispatch once the sync completes. The
  availability gate
  404ing means the Central sync is still propagating: re-dispatch once the
  portal deployment is released. A strict-resolution or emulator-smoke
  failure is a real consumer problem in the published artifact; a version
  already on Central cannot be replaced, so fix forward with a new patch
  version.

- **Publish run cancelled (same-version dispatch)**: if you accidentally dispatched another release candidate while a publish was queued, the pending publish may be cancelled without notice. Check the Actions tab for a cancelled run; if so, dispatch the publish again.

## Repository settings

- The repository was renamed from `ProjectFlora/AppAuth-Rust` to
  `ProjectFlora/NativeOIDC` for v0.7.0
  ([#146](https://github.com/ProjectFlora/NativeOIDC/issues/146)). GitHub
  redirects the former URL for git, the web, and release asset downloads,
  and Swift packages pinned to v0.1.0 through v0.6.0 resolve through that
  redirect. **Never create a new repository named `ProjectFlora/AppAuth-Rust`**:
  doing so ends the redirect and breaks those consumers.

- `release-consumer.yml` expects the release asset names and Maven
  coordinates of v0.7.0 and later, and the license notices of v0.7.1 and
  later (see [License notices](#license-notices)). Releases up to v0.6.0
  used the former names, and v0.7.0 lacks the notices, so none of them can
  be revalidated with the current workflow.

- The `publish` job needs `contents: write`. If tag rulesets protect `v*`,
  allow `github-actions[bot]` (the GitHub Actions app) to create those tags.

- Branch protection on `main` is unaffected: the workflow pushes only tags.

- The `publish_maven` job needs `contents: write` to read the Release assets
  and write the deployment record `maven-central-deployment.json`. If GitHub
  immutable releases are enabled, a published Release no longer accepts new
  assets: the record write fails, so the job stops before uploading, and
  the record would have to move elsewhere before the next release.

- The `publish_maven` job, and `dry_run_maven` on a candidate run, read
  these repository secrets:
  - `MAVEN_USER` and `MAVEN_PASSWORD`: a Central Portal user token with
    publishing rights to the verified `net.project-flora` namespace.
  - `GPG_PRIVATE_KEY`: the ASCII-armored private signing key. Its public key
    must be published to `keys.openpgp.org`, where Maven Central's sync
    validation fetches it (its absence rejected v0.6.0's upload,
    [#131](https://github.com/ProjectFlora/NativeOIDC/issues/131)). The
    `keyserver` job in the Release workflow enforces this on every run,
    candidate and publish: it derives the fingerprint of every primary key
    in the secret and requires each to resolve at
    `https://keys.openpgp.org/vks/v1/by-fingerprint/<fingerprint>`,
    retrying three times with 30-second pauses. The key server does not
    resolve subkey fingerprints on that endpoint, so only the primary keys
    are checked; a lookup by primary fingerprint returns the whole key
    block, subkeys included.
  - `GPG_PASSPHRASE`: the passphrase of that key.

- `keys.openpgp.org`'s live lookup endpoints are the `vks/v1/...` ones; the
  older `vkb/v1/...` endpoints answer `404` even for published keys, so do
  not use them to check a key. Upload a public key at
  <https://keys.openpgp.org/upload>.

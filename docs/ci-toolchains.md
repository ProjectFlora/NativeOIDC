# CI toolchains

Where each toolchain and tool version in the CI and release workflows is
pinned, the intent behind each pin, and how to change it. The four shared
composite actions in `.github/actions/` are the single source for most of
these versions; the workflow files only name what a job needs.

The jobs that call the actions:

| Action | `ci.yml` | `release.yml` | `release-consumer.yml` |
|---|---|---|---|
| `setup-rust-toolchain` | `msrv`, `test`, `licenses`, `apple_adapters`, `spm_bindings`, `windows_core`, `android_adapters`, `lint`, `coverage` | `validate_version`, `apple`, `android`, `release_docs`, `windows`, `publish_maven`, `dry_run_maven` | — |
| `setup-android` | `android_adapters` | `android`, `android_consumer`, `publish_maven`, `dry_run_maven` | `android`, `android_maven` |
| `setup-xcode` | `apple_adapters` (`latest` lane) | `apple`, `apple_consumer`, `spm_manifest` | `apple`, `spm`, `legacy_spm` |
| `setup-cargo-tools` | `licenses`, `apple_adapters`, `windows_core`, `android_adapters`, `coverage` | `validate_version`, `apple`, `android`, `windows` | — |

The actions request no secrets and no permissions; they only install tools
and (for `setup-android`) export the Android environment for the steps that
follow in the same job.

## Rust toolchain (`setup-rust-toolchain`)

Installs the rustup toolchain (`dtolnay/rust-toolchain`) and the cargo cache
(`Swatinem/rust-cache`). The `dtolnay` action is pinned to an immutable
commit, not to a version tag: the versioned refs of that action are
toolchain-specific, and with one pinned ref the action's `toolchain` input
is the single source of which channel a job installs.

| Channel | Where it lives | Intent |
|---|---|---|
| `stable` | the action's default input; the `ci.yml` jobs that follow stable call it without a `toolchain` input | CI follows the current stable release: rustup resolves `stable` at run time, so a new stable release needs no workflow change |
| `1.100.0` | spelled out at each package job of `release.yml` | every package of a release links the same rustc, so the archive ties to one toolchain (the rustc version is part of its metadata hash; docs/releasing.md) |
| `1.99.1` | the `msrv` job of `ci.yml` | the minimum supported Rust |

The `components` input carries the rustup components a job needs (for
example `clippy, rustfmt` in `lint`, `llvm-tools-preview` in `coverage`);
empty for none.

**Bumping the release pin:** change the `toolchain: 1.100.0` input in the
`release.yml` package jobs together, then run a release candidate.
**Bumping the MSRV:** change the `toolchain` input of the `ci.yml` `msrv`
job. **Changing the action ref:** only when the action code itself must
change; it does not affect which channel any job installs.

## The cargo cache key

`setup-rust-toolchain` always installs the cargo cache. An empty `cache-key`
input is ignored by `rust-cache`, which then uses its automatic key (the
Rust environment and lockfile hashes, plus the job id) — exactly what a
plain `Swatinem/rust-cache@v2` step does — so call sites that pass no
`cache-key` are unchanged. The Android jobs pass
`android-ndk-<NDK version>-api-<API level>` (read from the `setup-android`
action's exports), because `rust-cache` does not hash the NDK version or the
API level on its own and changing either must start a fresh cache.

The automatic key also hashes the job's Rust environment: the installed
toolchain versions, the lockfiles, and the environment variables that start
with `CARGO`, `CC`, `CFLAGS`, `CXX`, `CMAKE`, or `RUST`. A job whose
environment gains a new exported variable with one of those prefixes gets a
fresh key the next run it runs (a one-time cache miss, then warm again).
`ANDROID_*` variables are not among them, so the `setup-android` exports
never change a `rust-cache` key.

## Xcode (`setup-xcode`)

| Mode | Where it lives | Intent |
|---|---|---|
| `pinned` (default), version `26.6` | the action's `version` input (the default); every Apple job of `release.yml` and `release-consumer.yml` calls the action in the default `pinned` mode | App Store Connect requires apps built with Xcode 26 or later (Issue #135), so the archive is built and validated with a stable Xcode 26.x — the one the `macos-26` image ships as its default, so no extra download |
| `latest` | the `latest` iOS lane of `ci.yml` `apple_adapters` | the `xcode-27` preview runner ships no default selection the lane can rely on, so it selects the newest installed stable Xcode itself |

**Bumping the release Xcode:** change the `version` input default in
`.github/actions/setup-xcode/action.yml` — the single source used by every
pinned-mode call site — and update the runner label together, after checking
the
[runner-images README](https://github.com/actions/runner-images/tree/main/images/macos)
for the image's installed Xcode versions and simulator runtimes. A newer
Swift then reaches the release pipeline and the consumer audit only through
that one change.

## Android (`setup-android`)

Installs Java (Temurin), the SDK platform and build-tools, optionally the
NDK, and Gradle, and exports the Android environment for the steps that
follow.

| Input | Default | Meaning |
|---|---|---|
| `java-version` | `17` | the Gradle Java version |
| `packages` | `platform-tools` | `sdkmanager` packages (provides `adb` for the emulator steps) |
| `compile-sdk` | `android-35` | the SDK platform; the bare level is exported as `ANDROID_COMPILE_SDK` |
| `build-tools` | `35.0.0` | the SDK build-tools |
| `api-level` | `28` | the app's minSdk, exported as `ANDROID_API_LEVEL` |
| `ndk` | (unset) | the NDK version, installed when set and exported as `ANDROID_NDK_VERSION` and `ANDROID_NDK_HOME` |

Call sites pass only the `ndk` input (the jobs that build Rust for Android);
every other value comes from the action's defaults, so it lives in one
place. Every job exports `ANDROID_API_LEVEL` (the minSdk) and
`ANDROID_COMPILE_SDK` (the bare platform level) for its emulator steps, and
a job that installs the NDK additionally exports `ANDROID_NDK_VERSION` and
`ANDROID_NDK_HOME`. The emulator steps of the no-NDK consumer jobs
(`release.yml`, `release-consumer.yml`) use the exported `ANDROID_API_LEVEL`
as their api-level, so a documented minSdk bump updates those consumer tests
too.

**Bumping:** change the input defaults in
`.github/actions/setup-android/action.yml` (the NDK, at its two call
sites). Changing `compile-sdk`, `build-tools`, `api-level`, or `ndk`
changes the Android jobs' `rust-cache` key, so the first Android run after
the bump is a cache miss.

## Cargo tools (`setup-cargo-tools`)

Installs cargo tools from prebuilt release binaries
(`taiki-e/install-action`, checksum-verified upstream). Bare tool names
resolve against the version map in the action; an explicit `name@version`
installs that version as given; a bare name missing from the map fails the
step, so a version changes only when the map is updated deliberately.

| Tool | Version | Why it is pinned |
|---|---|---|
| `cargo-deny` | `0.20.2` | a new release can change the license and advisory results (`deny.toml`) |
| `cargo-about` | `0.9.3` | a new release can change the generated `THIRD_PARTY_LICENSES.txt`; the version must match the one `scripts/generate-third-party-licenses.sh` requires |
| `cargo-llvm-cov` | `0.9.2` | installed from the prebuilt release binary instead of a source-compiled `cargo install`; the release publishes a prebuilt `x86_64-unknown-linux-gnu` for the `ubuntu-latest` coverage job, and the summary step's report-format parsing is verified against this version's output |

**Bumping:** edit the map in `.github/actions/setup-cargo-tools/action.yml`.
When bumping `cargo-about`, update
`scripts/generate-third-party-licenses.sh` together. When bumping
`cargo-llvm-cov`, check that the coverage job's summary step still parses
the report (its table and the PR report percentages come from the
`cargo llvm-cov report` output of the installed version).

## Referencing the actions

The actions are referenced with the self-repo `$/` syntax
(`$/.github/actions/<name>`), not `./`: `$/` resolves the action in the
running repository at the commit being executed, so it works in the
`spm` and `legacy_spm` jobs of `release-consumer.yml`, which deliberately
check nothing out (the consumer must be a foreign package, like a real
SPM consumer).

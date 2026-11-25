# CI Toolchain Management

This document describes where each toolchain and tool version lives, the follow-stable/pinned intent for each, and how to update them.

## Rust Toolchain

**Location**: `.github/actions/setup-rust-toolchain/action.yml`

| Job/Context | Version | Intent |
|---|---|---|
| CI (`ci.yml`) - `test`, `lint`, `spm_bindings`, `apple_adapters`, `windows_core`, `android_adapters` | `stable` (rolling) | CI jobs follow the latest stable release for fresh tooling access |
| Release package jobs (`release.yml` - `api_docs` / `release_docs`) | `1.100.0` (pinned) | Package bytes tie to one rustc version for determinism; published artifacts depend on this specific compiler |
| MSRV job (`ci.yml`) | `1.99.1` (pinned) | Minimum Supported Rust Version is fixed until we decide to bump it |

**Update procedure**:
- **Bump MSRV**: Change the pinned version in `ci.yml` and `release.yml`; update the release notes in `CHANGELOG.md`
- **Bump release pin**: Only for major releases (e.g., v1.0); coordinate with release planning
- **Follow stable**: No action needed; rust-toolchain automatically tracks latest stable

## Xcode

**Location**: `.github/actions/setup-xcode/action.yml`

| Mode | Version Source | Default |
|---|---|---|
| `pinned` (release jobs) | Fixed version specified in input (e.g., "26.6") | `"26.6"` |
| `latest` (CI latest lane) | Newest stable installation on runner | N/A (selects dynamically) |

**Intent**: App Store Connect requires Xcode ≥ 26 (Issue #135). Release jobs use pinned version for reproducibility; CI's latest lane uses newest installed for validation.

**Update procedure**:
- Bump the `version` input default when a new major Xcode is available and all consumers have migrated

## Android SDK/NDK/API/Java

**Location**: `.github/actions/setup-android/action.yml`

| Property | Default Value | Notes |
|---|---|---|
| `ndk` | "" (none) | When set, uses this exact NDK version |
| `compile-sdk` | `"android-35"` | Target API for compilation |
| `build-tools` | `"35.0.0"` | Build tools version |
| `api-level` | `"28"` | Minimum SDK (minSdk) |

**Intent**: Fixed versions ensure reproducible builds across CI runs and releases.

**Update procedure**:
- Change defaults in the action when a new SDK/NDK/API level is required for new platform features

## Rust Cache

**Location**: `.github/actions/setup-rust-toolchain/action.yml` (via `Swatinem/rust-cache@v2`)

| Context | Cache Key | Notes |
|---|---|---|
| Default CI jobs | Empty (rust-cache default) | Uses path hashes for cache |
| Android NDK builds | `android-ndk-{NDK_VERSION}-api-{API_LEVEL}` | Ensures separate caches per NDK/API combo |
| LLVM tools preview | `llvm` | Separate cache from regular toolchain |

**Update procedure**: Cache is managed automatically; no manual action needed unless switching cache providers.

## Cargo Tools

**Location**: `.github/actions/setup-cargo-tools/action.yml`

| Tool | Default Version | Purpose |
|---|---|---|
| `cargo-deny` | `0.20.2` | License/Copyright checks for dependencies; result in `deny.toml` must not drift |
| `cargo-about` | `0.9.3` | Generates `target/licenses/third-party`; required by scripts/generate-third-party-licenses.sh |
| `cargo-llvm-cov` | `0.9.2` | Coverage collection; prebuilt binary from taiki-e/install-action |

**Intent**: Fixed versions prevent result drift between runs and releases.

**Update procedure**:
1. Update version in the action's internal version map
2. Run unit tests: `python3 -m unittest discover -s scripts -p 'test_*.py'`
3. Verify coverage job still works locally
4. Update `CHANGELOG.md` release notes if this is a breaking change

**Known constraints**:
- An explicit `name@version` override in workflow files takes precedence over the map
- Unknown tools without explicit versions fail clearly (no silent fallback)

## Version Matrix Summary

```
┌───────────────────────┬──────────────┬─────────────────┐
│ Context               │ Tool         │ Version          │
├───────────────────────┼──────────────┼─────────────────┤
│ CI test/lint/Apple    │ Rust         │ stable (rolling) │
│ MSRV                  │ Rust         │ 1.99.1 (pinned) │
│ Release package jobs  │ Rust         │ 1.100.0 (pinned)│
│ Xcode (release)       │ Xcode        │ 26.6 (pinned)   │
│ Android               │ NDK/API      │ Fixed defaults  │
│ Cargo tools           │ All          │ Fixed versions  │
└───────────────────────┴──────────────┴─────────────────┘
```

## Follow-Stable vs Pinning

- **CI jobs**: Follow stable Rust (rolling) for fresh tooling access; use pinned versions only when result drift must be prevented (MSRV, release package jobs)
- **Release jobs**: Pin to specific versions to ensure published artifacts are reproducible and tied to exact compiler/tool versions
- **Xcode**: Release pins to 26.x (App Store Connect requirement); CI uses newest installed for validation

## References

- [Rust release notes](https://releases.rust-lang.org)
- [Android SDK manager](https://developer.android.com/studio/command-line/sdkmanager)  
- [Xcode release history](https://www.apple.com/mac/download/feature-updates/)
- [Cargo tools releases](https://github.com/taiki-e/install-action#cargo-tools)

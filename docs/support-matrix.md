# Release support matrix

Tracking: [#60](https://github.com/ProjectFlora/NativeOIDC/issues/60).
OS decisions: [#43](https://github.com/ProjectFlora/NativeOIDC/issues/43),
implemented by [PR #76](https://github.com/ProjectFlora/NativeOIDC/pull/76).

## Status and evidence

Source configuration reviewed on 2026-11-13, based on
`d320722930c179c046f7c2051701389155556d89` (main after PR #76), with the
iOS 18/latest selection and Windows client CI additions accompanying this document.
The Apple CI lanes were re-reviewed on 2026-11-28 for #284 (both stable
Xcode series in regular CI); the accompanying PR is the source of that
change, and its CI run is the execution evidence for it. The OS decision
date is 2026-11-12; this source review does not renew the external
lifecycle or device-reach evidence. The dated rationale is in
[Minimum OS support policy](../README.md#minimum-os-support-policy).

This records the settled development baseline and the procedure for the final
1.0.0 record. Final release verification is pending. Published v0.4.0 assets
predate PR #76; their Android minimum must not be inferred from current main.
This document records CI configuration; successful execution evidence belongs
to the accompanying PR checks, not to the source review itself.

#60 covers this baseline, CI mapping, limitations, and verification procedure.
The release-dependent work is tracked separately:

- [#77: candidate artifact verification](https://github.com/ProjectFlora/NativeOIDC/issues/77).
- [#78: pre-tag OS support recheck and final decisions](https://github.com/ProjectFlora/NativeOIDC/issues/78).

Completing #60 does not satisfy those remaining 1.0.0 readiness gates.

## OS, distribution, and CI mapping

| Platform | Settled minimum / policy | Release targets and package | CI validation and limits |
| --- | --- | --- | --- |
| iOS / iPadOS | 18; recheck Apple's security-update generations before 1.0.0 | `aarch64-apple-ios` device and `aarch64-apple-ios-sim` simulator; XCFramework; deployment target 18.0 | Build both targets and type-check adapters. Regular CI runs three configurations (#284): the stable 26-series Xcode (currently 26.6) on `macos-latest` (currently the macOS 26 Arm64 image) with an installed iOS 18.6 runtime and with the latest 26.x runtime; and the stable 27-series Xcode (currently 27.2) on the `xcode-27` image (macOS 27) with the latest 27.x runtime. Each lane selects its stable Xcode major explicitly, and a missing series fails the job. Xcode 27's `xcodebuild -downloadPlatform` CLI finds no runtime older than iOS 26 (a known issue, reported on Apple's developer forums and reproduced on Xcode 27.2 on 2026-11-28; the same runtimes are installable from Xcode's Settings > Components GUI, but CI runs headless), so the 27-series lane validates the latest generation only in CI; the iOS 18 minimum is validated by the 26-series lane, whose CLI installs 18.6. The legacy 16-series lane was removed from regular CI with this change: the validation targets only the 26- and 27-series Xcode, and the macOS 15 host check is not required (#284 scope correction, 2026-11-28). Release builds use Xcode 26.6 on `macos-26` and run the iOS flow on an iOS 18.6 runtime the release workflow installs, because that image ships no iOS 18 runtime; the release and published-release consumer checks use the same Xcode. Missing requested runtimes fail; actual runtimes are logged. Latest means the newest available runtime on the hosted image, which can lag Apple's release. Package consumer links an iOS simulator library and runs on macOS, not on iOS. No exact 18.0, iPad, or physical-device execution claim. |
| macOS | 15; recheck Apple's security-update generations before 1.0.0 | `aarch64-apple-darwin` and `x86_64-apple-darwin`; universal XCFramework slice; deployment target 15.0 | Apple lanes in regular CI run on the macOS 26/27 hosts their images provide (#284; the `macos-15` host is no longer used in regular CI); release builds and consumer checks run on a `macos-26` host. Build both architectures, run mock loopback flow and package consumer on the host architecture (macOS 26/27 in regular CI). Building the other architecture does not verify execution on it; real system-browser flow is not automated and is checked by the manual [real system-browser verification](real-browser-verification.md). |
| Android | API 28 (Android 9); device-reach decision, not a security-patch guarantee; reassess reach, security bulletin, and dependencies before 1.0.0 | `aarch64-linux-android` / `arm64-v8a` and `x86_64-linux-android` / `x86_64`; AAR; Gradle minSdk and NDK API level 28 | Ubuntu host; regular CI runs the adapter and package-consumer tests on two x86_64 `google_apis_playstore` emulators, API 28 (minSdk) and API 35 (compileSdk and consumer targetSdk), one after the other in the `Android adapters validation` job. Release workflows run the consumer test on API 28 only. Verify both native libraries in AAR. arm64 device execution is unverified. compileSdk 35 is a build requirement, not the runtime minimum. |
| Windows | Microsoft-supported client versions, reviewed each release; concrete edition/version list in README's dated policy | `x86_64-pc-windows-msvc`; DLL, import library, C header, and .NET wrapper ZIP | `windows-latest` Server runner builds/tests and runs C++/.NET consumers. Regular CI also transfers the same package to `windows-11-arm` and runs x64 C++/.NET consumers under emulation, verifying a Windows 11 client host and logging OS/build/image. This adds neither a native ARM64 distribution target nor native x64 client-hardware coverage. Browser callbacks remain simulated in CI; real system-browser flow is checked by the manual [real system-browser verification](real-browser-verification.md). |

Linux is a development/CI host, not a supported distribution platform, and
1.0.0 states this as an explicit constraint
([#252](https://github.com/ProjectFlora/NativeOIDC/issues/252)): the Rust core
tests run on `ubuntu-latest`, but no Linux browser, callback, or application
path is built or run. tvOS is
deferred. Unverified provider and real-device coverage remains documented in
[live IdP interoperability](live-idp-interoperability.md).

Configuration evidence:

- Hosted Apple images: [Xcode 27 image](https://github.com/actions/runner-images/blob/main/images/macos/xcode-27-arm64-Readme.md)
  (generally available [2026-10-15](https://github.blog/changelog/2026-10-15-xcode-27-runner-image-generally-available/);
  the upstream README's pinned "public preview" announcement predates the GA),
  [macOS 26 image](https://github.com/actions/runner-images/blob/main/images/macos/macos-26-arm64-Readme.md)
  (the `macos-latest` label currently supplies it, checked 2026-11-28; when
  GitHub moves the label to a 27-series host image, re-select the runner the
  26-series lane uses — its explicit 26 selection fails first, so the
  re-selection is visible). The Apple lanes select their stable Xcode major
  explicitly ([`setup-xcode`](../.github/actions/setup-xcode/action.yml),
  [CI toolchains](ci-toolchains.md#xcode-setup-xcode)), and the iOS 18
  runtime is installed with
  [`install-ios-simulator-runtime.sh`](../scripts/install-ios-simulator-runtime.sh)
  where the image ships none.
  Xcode 27's `xcodebuild -downloadPlatform` CLI finds no runtime older than
  iOS 26 (reproduced on Xcode 27.2 on 2026-11-28: `-buildVersion 18.6`
  reports "not available for download"; a known issue — the same runtimes
  are installable from the Settings > Components GUI, but CI runs headless),
  so the 27-series lane validates the latest generation only in CI. Apple's
  [Xcode system requirements](https://developer.apple.com/xcode/system-requirements)
  table is the compatibility source (checked 2026-11-28: Simulator iOS 17 or
  later for Xcode 27.x, iOS 15 or later for Xcode 26.x).
  Release builds use a stable Xcode 26.x because App Store Connect requires
  apps built with Xcode 26 or later; the `setup-xcode` action in the release
  workflows selects it.
- Apple lane verification record (#284):
  [PR CI run 46075806384](https://github.com/ProjectFlora/NativeOIDC/actions/runs/46075806384)
  at `07ab8f0` (the first run of the final matrix), validated 2026-11-28.
  All three configurations passed, and each lane asserted the effective
  Xcode major before any build:
  - 26-series (`macos-latest`): image `macos-26-arm64-20261123.0526`,
    macOS 26.7.2 (25G332), Xcode 26.6 (17F113); the iOS 18.6 (installed) and
    iOS 26.5 flows.
  - 27-series (`xcode-27`): image `xcode-27-arm64-20261123.0387`,
    macOS 27.0.1 (26A434), Xcode 27.2 (27B52); the iOS 27.2 flow.
- Apple targets and deployment settings:
  [target builds, XCFramework packaging and slice assertions](../scripts/package-apple-xcframework.sh),
  [simulator selection](../scripts/check-ios-authorization-flow.sh).
- Android minimums: [library Gradle configuration](../native/android/build.gradle.kts),
  [consumer Gradle configuration](../native/consumer-smoke/android/app/build.gradle.kts),
  [NDK target builds](../scripts/check-android-targets.sh).
- CI and package checks: [regular CI](../.github/workflows/ci.yml),
  [release candidate build and consumers](../.github/workflows/release.yml),
  [published release consumers](../.github/workflows/release-consumer.yml).

The features each environment provides, and what the host must do, are
recorded per environment in the
[OIDC platform evaluation](oidc-platform-evaluation.md#feature--environment)
for v0.9.0. In particular, Windows has no standalone discovery or dynamic
registration, the Apple and Android adapters leave the logout browser to the
host, and in v0.9.0 no surface exposes the validated ID Token claims; the
next release adds them on every surface
([#262](https://github.com/ProjectFlora/NativeOIDC/issues/262)).

Alignment means that every declared distribution target is built and packaged,
runtime minimums agree across documentation and build configuration, and tests
and limitations are accurately recorded. It does not imply execution on every
supported OS, device, or architecture. Any decision to accept an execution gap
must be explicit in the final release record; an unrecorded gap is not a pass.

## Final 1.0.0 verification

Before tagging 1.0.0, complete this record against one release-candidate commit:

1. Recheck official Apple security releases, Android security bulletins and
   device-reach data, and Microsoft client lifecycles. Record source links,
   access dates, Windows editions/versions, rationale, and the next review
   trigger in README's support policy. Use the primary sources referenced by
   [FEATURE_MATRIX.md](../FEATURE_MATRIX.md#references) as starting points;
   an earlier PR's findings alone do not constitute a new lifecycle check.
2. Compare the matrix with the candidate's build scripts, both Android minSdk
   values, NDK API level, and all three workflows. Correct any drift and run
   affected checks. Record actual Apple simulator/runtime, host OS and
   architecture, Android emulator API/ABI, and Windows runner image/version.
   Resolve execution gaps by adding coverage or explicitly recording their
   accepted limits; do not describe deployment-target compilation as an
   execution test of the minimum OS.
3. Run regular CI and the release workflow in `workflow_dispatch` mode for
   the candidate version. Inspect the produced XCFramework slice metadata
   and deployment minimums, AAR manifest minimum and native ABI entries, and
   Windows binary architecture/package contents against this table. Record
   the exact asset names and SHA-256 digests, commit SHA, and successful run
   URLs. Do not substitute existing v0.4.0 assets for candidate artifacts.
4. Update README and ROADMAP to reference the completed record and settled
   release-time values. Keep historical changelog entries unchanged. Complete
   #78 after the lifecycle recheck, and record the candidate evidence and
   explicit dispositions for remaining verification limits under #77.
5. After publication, verify checksums and run the published-release consumer
   workflow for that version. Attach its run URL and confirm that the
   published assets match the candidate record; investigate any difference
   before closing #77.

| Final release record | Status |
| --- | --- |
| Candidate version and full commit SHA | Pending (#77) |
| Lifecycle/reach sources, checked date, and final OS decisions | Pending (#78) |
| Actual CI host/runtime versions and successful regular CI run | Pending for release candidate (#77) |
| Candidate release workflow run and consumer results | Pending (#77) |
| Inspected asset names, SHA-256 digests, targets, and minimums | Pending (#77) |
| Execution gaps and explicit release-time dispositions | Pending (#77) |
| Final confirmation date and next review trigger | Pending (#78) |
| Published release checksums and consumer workflow run | Pending publication (#77) |

Recheck Windows client support at every release. Recheck the whole matrix
immediately before 1.0.0, and whenever a lifecycle change, native dependency,
toolchain, or runner change may invalidate a minimum or its verification.

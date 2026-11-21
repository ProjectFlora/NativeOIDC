# Dependency Maintenance

This page records how NativeOIDC decides on UniFFI upgrades and on its JWT
library, and how the dependency graph is audited for security advisories
([Issue #66](https://github.com/ProjectFlora/NativeOIDC/issues/66)).

## UniFFI version

**Decision (2026-11-21): keep UniFFI pinned to 0.32.2.** Re-evaluate when
UniFFI publishes a release newer than 0.32.2.

### Findings

The evaluation compared the pinned version with the UniFFI releases on
crates.io and with the upstream `main` branch
([mozilla/uniffi-rs](https://github.com/mozilla/uniffi-rs), `main` at
`472cfc0`, 2026-11-20).

- **No newer release exists.** 0.32.2 (2026-09-23) is the latest UniFFI
  release on crates.io, so there is no released version to upgrade to. The
  0.32.1 and 0.32.2 changes are Kotlin fixes.
- **Swift 6 `Sendable`.** The checked-in bindings fail to build in the
  Swift 6 language mode with `SendingClosureRisksDataRace`
  ([Spike A](distribution-channels.md#spike-a-spm-binary-target-with-the-static-xcframework)).
  The async foreign-trait helpers `uniffiTraitInterfaceCallAsync` and
  `uniffiTraitInterfaceCallAsyncWithError` in
  `Sources/NativeOIDC/native_oidc.swift` pass non-`@Sendable` closures to
  `Task {}`, the pattern reported upstream in
  [mozilla/uniffi-rs#2929](https://github.com/mozilla/uniffi-rs/issues/2929).
  `NativeHttpTransport` is an async foreign trait, so the NativeOIDC bindings
  contain these helpers. The fix,
  [mozilla/uniffi-rs#2943](https://github.com/mozilla/uniffi-rs/pull/2943),
  is merged into `main` but is not in 0.32.2: the 0.32.2 Swift `Async.swift`
  template still declares the closures without `@Sendable`. The umbrella
  issue [mozilla/uniffi-rs#2458](https://github.com/mozilla/uniffi-rs/issues/2458)
  ("Async code doesn't conform to Swift 6 Sendable") is still open, so
  whether the next release makes the complete NativeOIDC bindings Swift 6
  clean can only be confirmed by building them.
- **Reproducible generation.** The unreleased changes make the
  `uniffi_bindgen` build itself reproducible
  ([mozilla/uniffi-rs#3048](https://github.com/mozilla/uniffi-rs/issues/3048))
  and the detection of recursive types deterministic. Neither changes how
  this repository generates bindings: `scripts/generate-bindings.sh` and the
  CI comparison with the checked-in Swift API stay as they are.
- **Packaging and distribution.** UniFFI still does not package the
  platform libraries, and no release changes how the bindings are
  distributed. The XCFramework, SPM, AAR, and Maven packaging in this
  repository are unaffected.
- **Breaking changes in the unreleased version.** Foreign bindings default
  their module name to the library name instead of the package name; for
  this crate the package name is `native-oidc` and the library name is
  `native_oidc`, so the generated module and file names must be compared
  after an upgrade. The Kotlin `disable_java_cleaner` option is renamed to
  `java_8_compatible` (not used here), and the deprecated library-mode API
  of `uniffi-bindgen` as a crate is removed (not used here: the
  `uniffi-bindgen` binary calls `uniffi::uniffi_bindgen_main`). No change to
  the future or cancellation behavior of async calls is listed; NativeOIDC
  implements cancellation separately from UniFFI.
- **MSRV.** UniFFI does not declare a `rust-version`. Compatibility with the
  MSRV (1.99.1) is checked by the MSRV job in Rust CI when the version
  changes.

### Rationale

| Option | Compatibility | Maintenance cost | Risk |
| --- | --- | --- | --- |
| **Keep 0.32.2 (chosen)** | No change for consumers | None now | The Swift 5 language-mode pin stays ([#133](https://github.com/ProjectFlora/NativeOIDC/issues/133)) |
| Pin to a git revision of upstream `main` | Brings the unreleased breaking changes | A non-crates.io source for the build, the license inventory, and `cargo deny` | Unreleased code in shipped bindings; the full Swift 6 build is still unconfirmed |
| Patch the generated Swift after generation | No change for consumers | A patch step to maintain on every regeneration | Conflicts with the policy of not hand-editing generated files ([ROADMAP.md](../ROADMAP.md)) |

### Re-evaluation

When UniFFI publishes a release newer than 0.32.2, evaluate it as follows.
If every step passes, upgrade in a pull request that updates the pin
(`Cargo.toml`, `Cargo.lock`), regenerates the checked-in bindings, and
updates the version references in [ROADMAP.md](../ROADMAP.md) and
[FEATURE_MATRIX.md](../FEATURE_MATRIX.md).

1. Read the release notes for breaking changes, in particular to the Swift
   and Kotlin generators, async calls, and module naming.
2. Check that the release contains
   [mozilla/uniffi-rs#2943](https://github.com/mozilla/uniffi-rs/pull/2943).
3. Regenerate the bindings and review the diff of the checked-in Swift API
   and the generated Kotlin, including the module and file names.
4. Run Rust CI, including the MSRV job and the Apple and Android adapter
   validations, and the dependency license and audit checks.
5. Build the regenerated Swift API in the Swift 6 language mode on macOS.
   A full build is needed: `swiftc -typecheck` does not run the check that
   reports `SendingClosureRisksDataRace`. If it passes, lifting the Swift 5
   language-mode pin continues in
   [#133](https://github.com/ProjectFlora/NativeOIDC/issues/133).

## JWT library

**Decision (2026-11-22): use `jsonwebtoken` 12 with the `rust_crypto`
backend**
([Issue #197](https://github.com/ProjectFlora/NativeOIDC/issues/197)).

`jsonwebtoken` 9.3.1 was affected by
[GHSA-h395-gr6q-cpjc](https://github.com/advisories/GHSA-h395-gr6q-cpjc)
(CVE-2026-25537, fixed in 10.3.0): an `exp` or `nbf` claim with the wrong
JSON type was treated as absent. NativeOIDC was not exploitable, because
[src/id_token.rs](../src/id_token.rs) deserializes the time claims into typed
fields and checks them itself, but the shipped graph should not contain a
known affected version.

From 10.0 the crate requires a crypto backend. The options were:

| Option | Benefit | Cost |
|---|---|---|
| **`rust_crypto` (chosen)** | Pure Rust, so no new C code is built for the release targets; with 12.0.0 it uses `rsa` 0.10, which fixes RUSTSEC-2023-0071, so no advisory is ignored | Adds RustCrypto crates (`rsa`, `p256`, `p384`, `ed25519-dalek`, `hmac`, `sha2` 0.11, and their dependencies) to the shipped graph, all under licenses `deny.toml` already allows |
| `aws_lc_rs` | One C library, maintained by AWS, for every algorithm | Builds C and assembly for every release target (iOS, Android, macOS, Windows) and changes the license inventory |
| A custom `CryptoProvider` on `ring` | `ring` is already shipped through `rustls` | NativeOIDC maintains its own RS256/ES256 glue, and `CryptoProvider::install_default` is process-wide, so it can conflict with a host that installs its own |

The `rust_crypto` provider is selected from the crate feature, so
NativeOIDC never calls `CryptoProvider::install_default`. Only `RS256` and
`ES256` signatures are verified, always with public keys from the JWKS.

## Security advisory audit

The [Dependency audit](../.github/workflows/dependency-audit.yml) workflow
runs `cargo deny check advisories` against the
[RustSec advisory database](https://rustsec.org/). It runs every Monday, on
manual dispatch, and on pull requests and pushes to `main` that change
`Cargo.toml`, `Cargo.lock`, `deny.toml`, or the workflow. It is not part of
Rust CI, because a new advisory would otherwise fail every unrelated pull
request.

The check covers the same graph as the license check (`[graph]` in
[deny.toml](../deny.toml)): the dependencies linked into the release
libraries for the release targets. It excludes the dev-dependencies and the
`uniffi-cli` feature, which only builds the binding generator. A
vulnerability, unmaintained, or unsound advisory against any crate in that
graph, direct or transitive, fails the check; a yanked crate is a warning.

When the scheduled run fails, update the affected dependency in a pull
request. If the advisory does not affect the release libraries, add it to
`ignore` in `deny.toml` with a comment that says why and when to revisit it.

Limits:

- Only advisories in the RustSec database are checked. An advisory published
  only as a GitHub Security Advisory is not reported. For example,
  GHSA-h395-gr6q-cpjc (see [JWT library](#jwt-library)) was found only by
  checking `Cargo.lock` against [OSV](https://osv.dev/) by hand.
- The Android build tools (Gradle and its plugins) and the Swift and .NET
  tooling are not audited.

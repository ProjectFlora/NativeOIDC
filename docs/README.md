# Documentation

NativeOIDC is an independent project by ProjectFlora. It is not affiliated
with, endorsed by, or sponsored by the OpenID Foundation or the AppAuth
projects (AppAuth-iOS, AppAuth-Android). OpenID is a trademark of the OpenID
Foundation.

Start with the [shared user guide](user-guide.md) for the authentication-state
lifecycle, storage, cancellation, errors, and logout policy. The
[public API coverage matrix](api-coverage.md) identifies the operations exposed
by each language and the examples and checks available today.

| Audience | Installation and current usage | Remaining guide work |
| --- | --- | --- |
| Swift: iOS/iPadOS and macOS | [XCFramework installation](swift-xcframework.md), [Swift guide](swift-guide.md#objective-c-interoperability), [Apple adapters](../README.md#apple-adapters) | [#85](https://github.com/ProjectFlora/NativeOIDC/issues/85) |
| Kotlin: Android | [Kotlin/Android user guide](kotlin-guide.md), [Android adapter](../README.md#android-adapter) | — |
| C++: Windows | [Windows C++ user guide](windows-cpp-guide.md), [Windows usage](../native/windows/README.md), [ABI reference](windows-api.md) | — |
| .NET: Windows | [Windows .NET user guide](windows-dotnet-guide.md), [Windows usage](../native/windows/README.md#net-interop), [wrapper contract](windows-api.md#net-wrapper) | — |
| Rust | [Manual authorization example](../examples/manual_authorization.rs), API reference below | [Coverage matrix](api-coverage.md) |

These are the current entry points, not a claim that all language guides are
complete. [#63](https://github.com/ProjectFlora/NativeOIDC/issues/63) tracks
completion. Each release after v0.8.0 publishes this documentation as a
version-matched set (see [API reference](#api-reference)). Apple and
Android public APIs target Swift and Kotlin only; Objective-C and Java hosts
should use an app-side wrapper, as described in the
[Swift guide](swift-guide.md#objective-c-interoperability) (Objective-C) or the
shared user guide for Java/Kotlin ([user-guide.md](user-guide.md)).

## API reference

Run `cargo doc --locked --no-deps --open` from the repository. The crate index
is `target/doc/native_oidc/index.html`; its `ffi` module documents the UniFFI
types and functions used by Swift and Kotlin. The source API names in that
reference are Rust names; use the generated bindings from the same release
for Swift/Kotlin spelling and signatures.

Each release after v0.8.0 publishes `NativeOIDC-Docs-<version>.zip`, built
from the release's commit: this index and every file it links (the guides,
the coverage matrix, the references, the header, the wrapper, and the
examples, at their repository paths under `repo/`), and the Rust API
reference under `rust-api/`. After unzipping, start at
`NativeOIDC-Docs-<version>/README.md`, which names the version and the full
commit SHA and lists every file; relative links work inside the unzipped
directory. Verify the archive with the release's `SHA256SUMS`. Releases up
to v0.8.0 published the Rust API reference alone, as
`NativeOIDC-API-Docs-<version>.zip`.

The Rust API reference is built on Linux, so it excludes the Windows module.
The [Windows ABI and .NET reference](windows-api.md), the
[C header](../native/windows/include/native_oidc_windows.h), and the
[C# wrapper](../native/windows/dotnet/NativeOidcWindows.cs) cover that public
surface without requiring Windows rustdoc. A local Windows `cargo doc` also
includes the Windows Rust module.

## Version and verification policy

Repository documentation describes the source revision being read. Use the
matching tag when installing a published release: current main's OS minimums,
API additions, and packaging fixes may postdate that release. See the
[support matrix](support-matrix.md) and [release downloads](../README.md#download-and-verify-v080).
Swift Package Manager is available from v0.5.0
([#82](https://github.com/ProjectFlora/NativeOIDC/issues/82)). Maven Central
is available from v0.7.0
([#81](https://github.com/ProjectFlora/NativeOIDC/issues/81)); earlier
versions use the documented GitHub Release installation for Android.

Maintainers publishing a version follow the [release procedure](releasing.md),
which includes the manual
[real system-browser verification](real-browser-verification.md) on macOS
and Windows. Pull request and `main` CI runs only the jobs a change can
affect; the [CI job selection](ci-job-selection.md) lists the rules.
The UniFFI version decision and the dependency security audit are recorded
in [dependency maintenance](dependency-maintenance.md).

Build checks, mock-provider tests, and real-provider/device verification are
different kinds of evidence. Consult the [coverage matrix](api-coverage.md)
and [interoperability limits](live-idp-interoperability.md) before treating an
example or CI fixture as a compatibility guarantee.

## Implementation specifications

- [Loopback callback listener](loopback-callback.md): the shared behavior of
  the Windows (Rust) and macOS (Swift) loopback listeners, where each item is
  implemented and tested, the known differences between them, and the
  iOS/iPadOS and Android redirect matching rules.

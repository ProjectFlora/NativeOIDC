# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org)

NativeOIDC plans to sign its Windows library with a certificate issued to
SignPath Foundation, through the SignPath Foundation program for open-source
projects. The application is in preparation
([#258](https://github.com/ProjectFlora/NativeOIDC/issues/258)). Until it
is approved and the release workflow signs the library, **releases are not
code signed**, including v0.9.0 and earlier. The first signed release will be
recorded in the repository's `CHANGELOG.md`.

## What is signed

| Release asset | File | Signature |
| --- | --- | --- |
| `native-oidc-windows-<version>.zip` | `native_oidc.dll` | Authenticode, timestamped, publisher "SignPath Foundation" (once signing is enabled) |
| `native-oidc-windows-<version>.zip` | `native_oidc.dll.lib`, the header, the .NET wrapper sources, and the notices | None: they are not Windows executables, so they cannot carry an Authenticode signature |
| `NativeOIDC-Apple-<version>.zip` | XCFramework | None from this project: Apple frameworks are signed as part of the app that embeds them |
| `native-oidc-android-<version>.aar` | AAR | None from this project: the app's APK or bundle signature covers it; Maven Central publications carry this project's OpenPGP signatures ([releasing](releasing.md)) |

Only binaries built from this repository's source code are signed. The
signed DLL statically links the Rust crates listed in
`THIRD_PARTY_LICENSES.txt` and the Rust standard library; no separately
distributed third-party binary is signed with this project's certificate.

A signature on `native_oidc.dll` identifies the library only. Applications
that ship it still sign their own executables and installers.

## How releases are built and signed

- Published releases are built by the release workflow on GitHub-hosted
  runners, from a run dispatched from `main` (see [Releasing](releasing.md)).
  Nothing built on a personal machine is published or signed.
- The Windows DLL carries a version resource with product name `NativeOIDC`
  and the crate version as its file and product version, and the packaging
  script fails when they do not match.
- Once signing is enabled, the release workflow submits the unsigned DLL to
  SignPath from the Windows package job, and packages the signed DLL, so the
  release ZIP and `SHA256SUMS` cover the signed bytes.
- Every signing request is approved manually by an approver (below) before
  SignPath signs it.

## Team roles

| Role | Members |
| --- | --- |
| Committers and reviewers | [saitomarch](https://github.com/saitomarch) (project owner) |
| Approvers | [saitomarch](https://github.com/saitomarch) (project owner) |

- Committers may change the source code in this repository without further
  review. Pull requests from anyone else are reviewed by a committer before
  they are merged.
- Approvers decide whether a release may be signed, and approve each signing
  request.

## Verify a signature

On Windows, from the unzipped `native-oidc-windows-<version>` directory:

```powershell
Get-AuthenticodeSignature .\native_oidc.dll | Format-List Status, SignatureType, SignerCertificate, TimeStamperCertificate
```

A signed release reports `Status` `Valid`, `SignatureType` `Authenticode`, a
signer certificate issued to SignPath Foundation, and a timestamp
certificate. From a repository checkout,
`scripts/check-authenticode-signature.ps1 -Path native_oidc.dll -ExpectedSigner "SignPath Foundation"`
checks the same and fails otherwise.

## Privacy

See the [privacy statement](privacy.md). In short: this program will not
transfer any information to other networked systems unless specifically
requested by the user or the person installing or operating it.

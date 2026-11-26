# OIDC platform evaluation (v0.9.0)

Return to the [documentation index](README.md). Tracking:
[#252](https://github.com/ProjectFlora/NativeOIDC/issues/252).

This document records how well NativeOIDC v0.9.0 works as an OpenID Connect
client on each environment, where it differs from the specifications, and
which follow-up work goes into which release. It is an evaluation record, not a
conformance certification. Nothing here was tested against a live identity
provider or a physical device (see [Unverified and uncertain](#unverified-and-uncertain)).

## Baseline and evidence rules

| Item | Value |
| --- | --- |
| Evaluated release | v0.9.0, release commit `b51065e372f06bfcc814409b33831ffb022ef506` (tag `v0.9.0`) |
| Source used for line references | `main` at `7b51351`. `src/`, `Sources/`, `native/`, `tests/`, `examples/`, `scripts/`, and `.github/` are byte-identical to the release commit; only documentation and `Package.swift` differ |
| Evaluation date | 2026-11-27 |
| Regular CI on the release commit's parent `f0a625b` | [run 45894502832](https://github.com/ProjectFlora/NativeOIDC/actions/runs/45894502832): every job succeeded. The release commit itself only changes `Package.swift` and has no CI run |
| Release workflow (publish) | [run 45897065354](https://github.com/ProjectFlora/NativeOIDC/actions/runs/45897065354) on `f0a625b`: every job succeeded except the skipped Maven dry run |
| Published release consumer workflow for 0.9.0 | [run 45900285067](https://github.com/ProjectFlora/NativeOIDC/actions/runs/45900285067): every job succeeded |
| Local runs (macOS 27.2.1 arm64, Rust 1.99.1, Xcode 27.2, Swift 6.4.2) | `cargo test --locked --all-targets` at `7b51351`: 109 library tests, 3 integration tests, all passed. `scripts/check-apple-adapters.sh`: swift-format lint, iOS and macOS adapter type-checks, and the macOS loopback flow smoke against the mock provider, all passed. Local reproduction tests for the findings below (not committed) |

Evidence that counts:

- source locations at the baseline (`path:line`);
- tests that exist in the repository, cited by name or test ID;
- CI and consumer runs listed above, cited by the step or script that ran (see
  the [CI evidence map](#ci-evidence-map)), never by job name alone;
- commands run locally for this evaluation, listed above.

Specifications are cited by section from the primary documents: OIDC Core 1.0
(errata set 2), OIDC Discovery 1.0, OIDC RP-Initiated Logout 1.0, OIDC Dynamic
Client Registration 1.0, RFC 6749, RFC 7009, RFC 7636, RFC 8252, RFC 9207, and
RFC 9700. AI-generated text, including the plan and this document's drafting,
is not evidence of conformance or behavior.

## Summary

- **Feature coverage.** Every in-scope protocol feature is implemented in the
  Rust core and reachable from Swift and Kotlin. Windows (C ABI and .NET) has
  no standalone discovery, configuration inspection, or dynamic registration,
  as already documented. No surface exposes the validated ID Token claims
  (issuer, subject, `auth_time`, `acr`).
- **Interoperability blocker (confirmed).** An issuer identifier without a
  path, such as `https://accounts.google.com`, can never match. The `url`
  crate serializes it as `https://accounts.google.com/`, and both discovery
  and ID Token validation compare that serialization byte for byte
  ([finding S4](#static-findings)). Providers that use such issuers cannot be
  used for OIDC on any surface.
- **Over-lenient checks (confirmed).** Refresh accepts an ID Token for a
  different subject and replaces the stored one. The authorization response
  `iss` parameter is not checked. Callback and registration error text reaches
  `Display`, Kotlin exception messages, and Swift error descriptions with
  newlines and control characters intact.
- **Release direction.** Most fixes change behavior, the saved state JSON, or
  the public API. They fit the v0.10.0 rule (see
  [Release buckets](#release-bucket-proposal)), which confirms that v0.10.0
  comes before the v1.0.0 candidate. The follow-ups are #260–#270.
- **Linux.** It has no browser or loopback integration, no distribution
  channel, and no execution evidence beyond Rust core unit tests on the CI
  host. Linux stays development and CI only for 1.0.0, as an explicit
  constraint (option C under [Linux options](#linux-options), decided
  2026-11-27).

## Requirement classification

Levels: **Required** (MUST for an RP that uses the feature, or needed by the
product's declared flow), **Conditional** (required only when the host uses a
related feature), **Optional extension** (MAY/OPTIONAL in the spec), **Out of
product scope** (a recorded product decision).

| Feature | Level | Spec basis | Notes and host responsibility |
| --- | --- | --- | --- |
| Discovery | Optional extension for the RP; required for OPs | OIDC Discovery §4; Core §15.2 binds dynamic OPs, not RPs | When used, the returned `issuer` MUST be identical to the requested issuer (Discovery §4.3) |
| Explicit endpoint configuration | Required (alternative to discovery) | Core §3.1.3.7 item 2: the issuer must be known to validate `iss` | |
| Authorization Code + PKCE S256 | Required | RFC 8252 §6 ("Public native app clients MUST implement" PKCE); RFC 9700 §2.1.1 (S256 is the only method that does not expose the verifier) | |
| External user agent | Required | RFC 8252 §5 and §8.12 (embedded user agents MUST NOT be used) | Adapters use ASWebAuthenticationSession, the system browser, or Custom Tabs |
| `state` and exact callback matching | Required | RFC 6749 §10.12; RFC 9700 §4.7 | |
| `nonce` | Required when sent | Core §3.1.3.7 item 11 ("a nonce Claim MUST be present") | The library always sends a nonce with `openid` |
| ID Token signature, `iss`, `aud`, `exp` | Required | Core §3.1.3.7 items 2, 3, 6, 9 | |
| ID Token `azp` | Conditional (SHOULD) | Core §3.1.3.7 items 4–5 | Implemented |
| ID Token `acr` | Conditional (SHOULD, when requested) | Core §3.1.3.7 item 12 | Host can request `acr_values`; no check and no claims API (see G3) |
| ID Token `auth_time` with `max_age` | Conditional (SHOULD, when requested) | Core §3.1.2.1 (OP MUST return `auth_time` when `max_age` is used); §3.1.3.7 item 13 | Host can send `max_age`; no check and no claims API (see G4) |
| Validated identity (issuer + subject) for the host | Required by the product's account model; the spec defines `iss`+`sub` as the identifier | Core §2 (`sub` is unique only within the issuer); §5.3.2 (UserInfo `sub` MUST match the ID Token) | Not exposed on any surface (see G3) |
| UserInfo | Optional extension for the RP; out of product scope | Core §5.3 has no RP MUST; §15.2 binds dynamic OPs only | Host calls UserInfo with the access token and MUST compare `sub` with the ID Token (§5.3.2). Today the host must decode the raw ID Token itself to get `sub` (see G3) |
| Refresh | Required by the product scope | RFC 6749 §6; Core §12.2 | |
| ID Token returned by refresh | Required when present | Core §12.2: `iss`, `sub`, `aud` MUST equal the original; `auth_time` MUST be the original. These bind the OP; checking them is the RP's only defense when an OP or a mixed-up configuration violates them | See S1 |
| `token_type` handling | Required (client) | RFC 6749 §7.1 (MUST NOT use a token whose type it does not understand); §5.1 (value is case-insensitive); Core §3.1.3.3 (MUST be Bearer unless negotiated) | The library does not use access tokens itself; the host does (see G8) |
| Save and restore state | Product requirement | None (not a protocol feature) | Host owns encrypted storage |
| Cancellation | Product requirement | None | |
| RP-Initiated Logout | Optional extension | RP-Initiated Logout §2: every RP parameter is OPTIONAL or RECOMMENDED | `logout_hint` and `ui_locales` are not settable (see G9) |
| Token revocation | Optional extension | RFC 7009 §2: "Clients MUST verify that the URL is an HTTPS URL" | See D7 |
| Dynamic registration (public client) | Optional extension | RFC 7591; OIDC Registration 1.0 | Public clients only; responses with `client_secret` are rejected by design |
| Mix-up defense | Conditional: required when one client talks to more than one AS | RFC 9700 §2.1 and §4.4.2; RFC 9207 §2.4 | See D1 |
| Distribution and installation | Product requirement | None | |
| Implicit, ROPC, static client secrets, Device Authorization | Out of product scope | RFC 8252 §8.2; RFC 9700 | Recorded in [FEATURE_MATRIX.md](../FEATURE_MATRIX.md) |

## Feature × environment

Status: **SDK** = provided by the SDK on that surface; **Host** = possible,
with host work described; **Missing** = not possible without changing the
SDK; **Unverified** = provided, but no execution evidence on that environment.

Evidence keys are defined under the table.

| Feature | Windows C ABI | Windows .NET | macOS (Swift) | iOS (Swift) | Android (Kotlin) | Linux (Rust API) |
| --- | --- | --- | --- | --- | --- | --- |
| Discovery | SDK, only inside an operation (`issuer` in the request JSON; `src/windows/ops.rs:59-63`). No standalone call. [W1, W2] | SDK, same JSON [W2] | SDK `FfiServiceConfiguration.discover` (`src/ffi.rs:269`) [U1, A1] | SDK, same [U1, A1, A2] | SDK, same [U1, K1] | SDK `ServiceConfiguration::discover` (`src/authorization/discovery.rs:20`) [R1] |
| Explicit configuration | SDK, request JSON fields [W1, W2] | SDK [W2] | SDK `FfiServiceConfiguration.new` (`src/ffi.rs:257`) [U1] | SDK [U1] | SDK [U1, K1] | SDK `ServiceConfiguration::new` [R1] |
| Code + S256 PKCE | SDK, inside `native_oidc_windows_authorize` (`src/windows/abi.rs:286`) [W1, W2] | SDK `AuthorizeAsync` [W2] | SDK `begin_authorization` (`src/ffi.rs:423`) [R1, A1] | SDK [R1, A2] | SDK [R1, K1] | SDK `AuthorizationRequest::begin` (`src/authorization/session.rs:122`) [R1, R2] |
| state, nonce, callback validation | SDK, loopback 127.0.0.1 only (`src/windows/loopback.rs:22`) [W1, W2] | SDK [W2] | SDK, `MacOSAuthorizationAdapter` loopback (`Sources/NativeOIDC/MacOSAuthorizationAdapter.swift:62`) [A1, A3] | SDK, `IOSAuthorizationAdapter` (`Sources/NativeOIDC/IOSAuthorizationAdapter.swift:99`) [A2] | SDK, `AndroidAuthorizationAdapter` (`native/android/src/main/kotlin/net/projectflora/nativeoidc/AndroidAuthorizationAdapter.kt:90`) [K1, K2] | Host: the host captures the callback and calls `parse_redirect` (`src/authorization/session.rs:210`). No loopback listener exists for Linux; the Windows listener is compiled only on Windows (`src/windows/mod.rs:8-14`). No Linux execution evidence |
| ID Token and JWKS validation | SDK [R1, W1] | SDK [W2] | SDK [R1, U1] | SDK [R1, U1] | SDK [R1, K1] | SDK (`src/id_token.rs:167`) [R1]. Issuers without a path cannot match on any surface (S4) |
| Validated identity (iss + sub, auth_time, acr) | Missing: state JSON contains only the raw ID Token | Missing | Missing: only `FfiTokenResponse.idToken` (raw string) | Missing | Missing | Missing: only `TokenResponse::id_token` (raw string). Host could decode the JWT itself after the SDK has verified it; no evidence or example for that path |
| UserInfo | Host: call UserInfo with the access token; needs a validated `sub` (Missing above) | Host, same | Host, same | Host, same | Host, same | Host, same |
| Refresh and state update | SDK `native_oidc_windows_refresh` (`src/windows/abi.rs:324`) [W1, W2] | SDK `RefreshAsync` [W2] | SDK `FfiAuthenticationState.refresh` (`src/ffi.rs:627`) [U1, A1] | SDK [U1] | SDK [U1, K1] | SDK `AuthenticationState::refresh` (`src/authorization/state.rs:125`) [R1]. Accepts a different subject (S1) |
| Save and restore | SDK, state JSON returned by operations [W1, W2] | SDK, same JSON [W2] | SDK `toJson`/`fromJson` (`src/ffi.rs:596,606`) [U1 S1–S2] | SDK [U1] | SDK [U1, K1] | SDK, Serde on `AuthenticationState` [R1]. Encryption and storage are always the host's |
| Browser launch, finish, cancel | SDK: system browser, loopback, cancellation handle (`src/windows/abi.rs:222-262`) [W1, W2]; real browser checked only manually ([real-browser verification](real-browser-verification.md)) | SDK, `CancellationToken` [W2] | SDK, browser-opening closure and `cancel` (`MacOSAuthorizationAdapter.swift:136`) [A1, A3]; real browser checked only manually | SDK, `cancel` (`IOSAuthorizationAdapter.swift:153`); OS dismissal returns Cancelled [A2] | SDK, Custom Tab and `cancel` (`AndroidAuthorizationAdapter.kt:159`); closing the Custom Tab does not end the wait (#246) [K2] | Host: no browser launch or listener |
| Logout (RP-Initiated) | SDK, integrated browser and loopback `native_oidc_windows_end_session` (`src/windows/abi.rs:365`) [W1; W2 error paths only] | SDK `EndSessionAsync` [W2 error paths only] | Host: SDK builds the URL and parses the callback (`begin_end_session`); the host opens the browser. Adapters have no logout method [U1 E1–E5] | Host, same [U1] | Host, same [U1, K1] | Host, same (`src/authorization/end_session.rs`) [R1] |
| Revocation | SDK `native_oidc_windows_revoke` (`src/windows/abi.rs:401`) [W1, W2] | SDK `RevokeAsync` [W2] | SDK `revoke` (`src/ffi.rs:357`) [U1 R1–R7] | SDK [U1] | SDK [U1, K1] | SDK `revoke_token` [R1] |
| Dynamic registration | Missing (documented Windows limit) | Missing (documented) | SDK `register_public_client` (`src/ffi.rs:679`) [U1 G1–G7] | SDK [U1] | SDK [U1, K1] | SDK `ClientRegistrationRequest::register` [R1, R2] |
| Use from distributed packages | SDK: Windows ZIP from GitHub Releases [W2, P1, P2] | SDK: same ZIP [W2, P1, P2] | SDK: XCFramework ZIP and SPM [A4, P1, P2] | SDK: same; iOS execution only on simulators [A2, A4, P1, P2] | SDK: AAR from GitHub Releases and Maven Central [K3, P1, P2] | Missing: no package; crate is `publish = false` (#64); only a git dependency on the source |

Evidence keys (the steps and scripts are listed in the [CI evidence map](#ci-evidence-map)):

- **R1**: Rust library unit tests (`cargo test`, module tests in `src/`), CI job `test` and local run.
- **R2**: `tests/protocol_integration.rs` (3 tests), same runs.
- **U1**: Swift UniFFI tests in `SwiftTests/NativeOIDCTests/NativeOIDCTests.swift` (IDs D1–D7, R1–R7, G1–G7, E1–E5, S1–S2, C1–C5; see [API coverage](api-coverage.md)), run by `scripts/check-spm-package.sh` (`swift test`).
- **A1**: `scripts/check-apple-adapters.sh`: adapter type-checks and the macOS loopback flow smoke (`native/apple/MacOSAuthorizationFlowSmoke.swift`) against a local mock provider. CI and local run.
- **A2**: `scripts/check-ios-authorization-flow.sh` (`native/apple/IOSAuthorizationFlowSmoke.swift`), run from `scripts/package-apple-xcframework.sh`: iOS simulator flow against a mock provider. Simulators only.
- **A3**: `SwiftTests/NativeOIDCTests/MacOSAuthorizationAdapterTests.swift` (loopback lifecycle), run by `swift test`.
- **A4**: `scripts/check-apple-release-consumer.sh` and the SPM consumer jobs.
- **K1**: Android instrumentation tests `UniFfiOperationCoverageTest`, `AndroidExternalBrowserAuthorizationTest` (`connectedDebugAndroidTest`, API 28 and 35 emulators).
- **K2**: `AndroidAuthorizationCallbackActivityTest`, `AndroidAuthorizationAdapterErrorTest`, plus JVM unit tests (`testDebugUnitTest`).
- **K3**: Android consumer fixture `GuideExamplesTest`, `ReleaseConsumerTest` (direct AAR and Maven Central).
- **W1**: Windows-target Rust tests (`cargo test --lib --target x86_64-pc-windows-msvc`), including loopback tests in `src/windows/loopback.rs`.
- **W2**: `scripts/check-windows-release-consumer.ps1`: C++ and .NET consumers against a mock provider, on `windows-latest` and on `windows-11-arm` under x64 emulation.
- **P1**: release workflow run 45897065354 (package build and release-consumer jobs).
- **P2**: published release consumer run 45900285067.

## Static findings

These are the three points named in #252 plus one found while checking them.
"Confirmed by test" means a local Rust test reproduced the behavior; the test
code goes into the follow-up issue.

| ID | Point | Result | Evidence |
| --- | --- | --- | --- |
| S1 | `sub`/`aud`/`iss` of an ID Token returned by refresh | **Confirmed by test.** Refresh validates the new ID Token against the configuration (signature, `iss` = configured issuer, `aud` contains the client ID, time claims, nonce only if present) but never compares it with the original token. A refreshed token for another subject, with a new `auth_time` and a new `azp`, was accepted and replaced the stored ID Token. The state JSON keeps only `token_response`, `expires_at`, and `oidc_nonce`, so the original claims are not available to compare | `src/authorization/state.rs:184-205`; `src/authorization/state.rs:39-44`. Core §12.2 |
| S2 | `max_age` / `auth_time` | **Confirmed by test.** `max_age`, `acr_values`, `claims`, and `prompt` are accepted as additional authorization parameters, but `IdTokenClaims` has no `auth_time` or `acr` field and nothing checks them. Core §3.1.3.7 items 12–13 are SHOULD checks; the reason for not doing them is not recorded | `src/authorization/validation.rs:7-16` (reserved names); `src/id_token.rs:79-88` |
| S3 | API returning the validated issuer and subject | **Confirmed (static reading of every surface).** No Rust, UniFFI, C ABI, or .NET API returns validated claims. Each surface returns the raw ID Token string only | `src/authorization/state.rs:19-35`; `src/ffi.rs:548`; `docs/windows-api.md` |
| S4 | Issuer identifiers without a path | **Confirmed by test.** `Url::parse("https://issuer.example").as_str()` is `https://issuer.example/`. An ID Token with `iss` `https://issuer.example` is rejected, and the existing test `discovery_validates_every_endpoint_and_exact_issuer` asserts that discovery rejects metadata whose `issuer` lacks the slash. Because every surface takes the issuer as a URL (Rust `Url`, UniFFI and Windows strings parsed with `Url::parse`), no caller can express such an issuer. Discovery §4.3 and Core §3.1.3.7 item 2 require an exact match, so issuers such as `https://accounts.google.com` and `https://appleid.apple.com` cannot be used | `src/id_token.rs:240,252`; `src/authorization/discovery.rs:46`; `src/windows/ops.rs:61,96` |

## Strictness divergence

Direction: **over-strict** (the spec allows it; the library rejects it or has no
way to do it) or **over-lenient** (the spec requires rejection; the library
accepts it). A SHOULD-level difference counts only if no reason is recorded.

| ID | Behavior | Spec and level | Library behavior | Direction | Evidence | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| D1 | Authorization response `iss` (RFC 9207) | RFC 9207 §2.4: clients MUST compare `iss` when present and reject a mismatch; RFC 9700 §4.4.2: mix-up defense MUST when talking to more than one AS | `iss` is kept as an additional parameter and never compared (`src/authorization/session.rs:210-265`); discovery does not read `authorization_response_iss_parameter_supported` | Over-lenient (conditional) | Confirmed by test: a callback with a foreign `iss` was accepted | Gap (G5) |
| D2 | Refresh ID Token consistency | Core §12.2 (OP MUST keep `iss`, `sub`, `aud`, `auth_time`) | Not compared (S1) | Over-lenient | Confirmed by test | Gap (G1) |
| D3 | `auth_time` / `acr` checks | Core §3.1.3.7 items 12–13 (SHOULD) | Not checked, no recorded reason (S2) | Over-lenient | Confirmed by test | Gap (G4) |
| D4 | Issuer without a path | Discovery §4.3, Core §3.1.3.7 item 2 (MUST exact match) | Never matches (S4) | Over-strict | Confirmed by test | Gap (G2) |
| D5 | `token_type` | RFC 6749 §7.1 (client MUST NOT use a token it does not understand); §5.1 case-insensitive | Any non-empty value is stored unchanged (`src/authorization/state.rs:225`); `N_A` and `bearer` are both accepted as is. The library never uses the access token itself | Not a divergence for the library; host responsibility is undocumented | Confirmed by test | Host responsibility (G8) |
| D6 | Untrusted error text | RFC 6749 §4.1.2.1 and §5.2: `error` and `error_description` are limited to printable ASCII (`%x20-21 / %x23-5B / %x5D-7E`) | Token and revocation errors enforce this (`src/authorization/http.rs:127-151`). Authorization callback errors (`src/authorization/session.rs:250-258`) and registration errors (`src/authorization/registration.rs:126-135`) do not: newline and ESC characters pass into the error code and description | Over-lenient | Confirmed by test | Gap (G6) |
| D7 | Loopback HTTP endpoints | RFC 7009 §2: client MUST verify the revocation URL is HTTPS. RFC 6749 §3.1/§3.2 require TLS (binds the AS) | Authorization, token, and revocation endpoints accept `http://` on loopback hosts (`src/authorization/validation.rs:49-69,89-96`); issuer, JWKS, end-session, registration, and discovered endpoints require HTTPS. Used by the local mock providers; the reason is not recorded | Over-lenient (local only) | Static reading only | Gap: decide and record (G10) |
| D8 | End-session `logout_hint`, `ui_locales` | RP-Initiated Logout §2 (OPTIONAL) | Reserved and rejected in the endpoint query, but no setter exists (`src/authorization/validation.rs:40-47`) | Over-strict (optional feature) | Static reading only | Gap (G9) |
| D9 | Loopback IPv6 | RFC 8252 §7.3 (RECOMMENDED to bind IPv4 and IPv6) | Windows and macOS accept only `http://127.0.0.1` redirects (`src/windows/loopback.rs:22`; `MacOSAuthorizationAdapter.swift:148`) | Over-strict (RECOMMENDED) | Static reading only | Gap (G11) |
| D10 | Signing algorithms | Core §3.1.3.7 item 7 (alg SHOULD be RS256 by default); other algorithms allowed | RS256 and ES256 only (`src/id_token.rs:205`) | Over-strict | Static reading only | Intentional restriction ([ROADMAP](../ROADMAP.md#confirmed-decisions) "OIDC algorithms and JWKS") |
| D11 | Registration responses with `client_secret` | RFC 7591 §3.2.1 (server MAY issue a secret) | Rejected (`src/authorization/registration.rs:172`) | Over-strict | Existing test G3 | Intentional restriction ([FEATURE_MATRIX](../FEATURE_MATRIX.md), public clients only) |
| D12 | Fractional NumericDate (`exp` with decimals) | RFC 7519 §2 (NumericDate may be non-integer) | Rejected | Over-strict | Confirmed by test; also covered by the existing time-claim tests | Intentional restriction (recorded in the [security audit](security-edge-case-audit.md) "ID Token time boundary") |
| D13 | `nonce` on refresh | Core §12.2 (SHOULD NOT be present; if present MUST equal the original) | Absent is accepted; present and different is rejected (`src/authorization/state.rs:184-194`, `src/id_token.rs:276-286`) | Not a divergence | Static reading and existing state tests | Not a divergence |
| D14 | JWKS with unsupported extra keys | RFC 7517 §5 (implementations SHOULD ignore keys with a `kty` they do not understand) | A set with an extra `enc` RSA-OAEP-256 key, a PS256 key, an unknown `kty`, an `oct` key, or an unknown `alg` still validates a token signed by the RS256 key | Not a divergence | Confirmed by test | Not a divergence |
| D15 | Discovered endpoints over HTTPS | Discovery §3 (`issuer` uses https); RFC 6749 §3.1–3.2 (TLS for the endpoints) | Enforced (`src/authorization/discovery.rs:110-119`) | Not a divergence | Existing discovery tests | Not a divergence |
| D16 | `jku`/`jwk`/`crit` headers, RSA < 2048 bits | RFC 7515 §4.1.11 (unknown `crit` MUST be rejected); RFC 7518 §3.3 (RSA keys MUST be ≥ 2048 bits) | Rejected (`src/id_token.rs:184-198,230`) | Not a divergence | Existing ID Token tests | Not a divergence |

## Logging and secret exposure

| Item | Finding | Disposition |
| --- | --- | --- |
| Library log output | None found. The Rust core has no `log`/`tracing` dependency and no `println!`/`eprintln!`/`dbg!` outside tests. No `print`/`os_log`/`Logger` in the Swift adapters, no `Log` in the Kotlin adapter, no `Console`/`Debug`/`Trace` in `native/windows/dotnet/NativeOidcWindows.cs` | OK |
| Rust string forms | `TokenResponse` and `AuthenticationState` do not implement `Debug` (`src/authorization/state.rs:17,38`). `WindowsRevocationRequest` and `WindowsEndSessionRequest` derive `Debug` and hold `token` and `id_token_hint` (`src/windows/data.rs:26,58`), but nothing formats them with `{:?}` outside tests | OK (consider removing the derive with G7) |
| Kotlin string forms | UniFFI generates `FfiTokenResponse` as a Kotlin `data class`, so `toString()` prints the access, refresh, and ID tokens. `FfiException.AuthorizationServer.message` is `code=…, description=…` with the server text unchanged | Host responsibility, undocumented (G7) |
| Swift string forms | `FfiTokenResponse` is a struct; `print`/`dump`/string interpolation shows every field, tokens included. `FfiError.errorDescription` is `String(reflecting: self)`, which includes the server's description | Host responsibility, undocumented (G7) |
| C ABI and .NET | Errors are returned as status plus text; .NET throws `InvalidOperationException(value)` with that text. Text from callback and registration errors can carry newlines (D6) | Gap (G6) |
| Error text with outside data | Token and revocation errors are charset-validated. Callback and registration errors are not (D6). `TransportFailure` carries the host transport's message; I/O errors carry OS text | Gap (G6); transport text is the host's own |
| Guide coverage | The guides say to keep tokens and state JSON out of logs (`docs/user-guide.md:315`, `docs/swift-guide.md:375`, `docs/kotlin-guide.md:348`), and the Windows guides call provider descriptions untrusted text. None says that `FfiTokenResponse`, Kotlin exceptions, or Swift `errorDescription` print tokens or untrusted text | Gap (G7) |
| CI and release scripts | Secrets reach `release.yml` only through `secrets.*` (masked by GitHub). No script enables `set -x` or prints credentials; mock providers issue fixed test values | OK |

## CI evidence map

Job names do not always say what a job runs (#253), so this table records the
steps behind the evidence keys. Workflow file and job id are given as
`file:job`.

| Workflow:job (displayed name) | Steps and scripts that back the evidence | What they verify | Keys |
| --- | --- | --- | --- |
| `ci.yml:test` ("Tests") | `cargo test --locked --workspace --all-targets --all-features` on `ubuntu-latest` | Rust core and FFI unit tests and the protocol integration tests on Linux x86_64. Includes the Windows module's platform-independent tests (`src/lib.rs:27`) | R1, R2 |
| `ci.yml:apple_adapters` ("Apple adapters validation (iOS …)") | `scripts/check-apple-adapters.sh`; `scripts/package-apple-xcframework.sh` (runs `scripts/check-ios-authorization-flow.sh`); `scripts/check-apple-release-consumer.sh`; `scripts/check-spm-package.sh` (`swift test`, iOS simulator build) | Adapter type-checks, macOS loopback flow, iOS simulator flow on iOS 18 and latest, XCFramework consumer, Swift UniFFI tests | A1, A2, A3, A4, U1 |
| `ci.yml:android_adapters` ("Android adapters validation") | `gradlew testDebugUnitTest`; `gradlew connectedDebugAndroidTest` on API 28 and API 35 emulators; AAR content checks | Kotlin adapter unit tests, instrumentation flow and operation coverage on x86_64 emulators | K1, K2 |
| `ci.yml:windows_core` ("Windows core target validation") | `cargo test --locked --lib --target x86_64-pc-windows-msvc`; `scripts/package-windows.ps1`; `scripts/check-windows-release-consumer.ps1` | Windows-only Rust tests (C ABI, loopback), package build, C++ and .NET consumers on Windows Server | W1, W2 |
| `ci.yml:windows_client` ("Windows 11 client consumers (x64 emulation)") | `scripts/check-windows-release-consumer.ps1` on `windows-11-arm` | Same consumers on a Windows 11 client under x64 emulation | W2 |
| `release.yml` jobs `apple`, `android`, `windows`, `*_consumer`, `publish`, `publish_maven` | Same scripts as above on the release package, then publication | The published 0.9.0 assets were built and consumed | P1 |
| `release-consumer.yml` jobs `apple`, `spm`, `legacy_spm`, `android`, `android_maven`, `windows`, `docs` | Consumers against the published GitHub Release, SPM tag, and Maven Central artifacts | Installation from every distribution channel | P2 |

No job runs on a Linux desktop session, opens a Linux browser, or uses a
loopback listener on Linux.

## Compatibility assessment

| Boundary | Current shape | What adding or changing it breaks |
| --- | --- | --- |
| Rust public API | `pub` items re-exported from `src/lib.rs`; builder methods take `Url` | Changing `discover(issuer: Url)` or `with_oidc_validation_endpoints(issuer: Url, …)` signatures (a possible fix for G2) breaks Rust callers. Adding methods does not. The crate is not published (#64), so Rust callers are git dependents only |
| UniFFI records (Swift/Kotlin) | Records such as `FfiTokenResponse`, `FfiAuthorizationOptions`, `FfiEndSessionOptions` have no `#[uniffi(default)]` fields | Adding a field changes the Swift memberwise initializer and the Kotlin constructor and `componentN` functions: a source break for hosts that construct or destructure the record. Option records (constructed by hosts) are affected; output records only if hosts construct them in tests. Adding fields with `#[uniffi(default = …)]` keeps Swift and Kotlin source compatibility |
| UniFFI errors | Swift `public enum FfiError`; Kotlin `sealed class FfiException` | Adding a variant breaks exhaustive Swift `switch` and Kotlin `when` expressions. Splitting an existing variant (for example a new mix-up error for G5) is a break |
| C ABI | Ten exported functions, status codes, JSON in and out (`docs/windows-api.md`) | New optional request JSON keys are ignored by older DLLs (no `deny_unknown_fields`). New result keys are additive for JSON readers. New status codes break hosts that switch on all codes |
| Saved state JSON | `{token_response, expires_at, oidc_nonce}`, Serde without `deny_unknown_fields`; `oidc_nonce` uses `#[serde(default)]` | Adding fields with `#[serde(default)]` keeps old JSON readable, and older libraries ignore new keys. But state saved before G1 lacks the original claims, so the refresh comparison needs a defined migration rule (for example: record claims from the first validated ID Token, or require re-authentication). That is a saved-format change that needs migration |
| Callback and cancel behavior | Contract in [loopback callback](loopback-callback.md) and ROADMAP | G5 (reject mismatched `iss`) and #246 (macOS path normalization, D9 in that spec) change which callbacks are accepted |
| Error text | Display strings and status texts | G6 changes the variant and the message for malformed provider errors |
| Minimum OS and toolchain | iOS 18, macOS 15, Android API 28, Microsoft-supported Windows clients, Rust 1.99.1 ([support matrix](support-matrix.md), [CI toolchains](ci-toolchains.md)) | None of the findings changes them. #78 rechecks them before the 1.0.0 tag |
| Distribution | GitHub Releases (all), SPM, Maven Central | None of the findings changes them. Linux option B would add a channel |
| Public names | Core `AuthError::InvalidConfiguration` becomes `FfiError::InvalidRequest` (`src/ffi.rs:68`); `FfiError` has both `Transport` and `TransportFailure`, and only the latter is observable from Swift and Kotlin | A rename is a breaking change. The audit belongs to #254 (already in the v0.10.0 milestone); any public rename it confirms goes in v0.10.0 or Later 0.x |

## Related issues

| Issue | State on 2026-11-27 | How this evaluation uses it |
| --- | --- | --- |
| [#246](https://github.com/ProjectFlora/NativeOIDC/issues/246) adapter callback and cancel contract | Open, milestone v0.10.0 | The browser-closing difference between iOS (cancels) and Android (keeps waiting), and the macOS/Windows loopback path normalization, are left to #246. Not duplicated here |
| [#133](https://github.com/ProjectFlora/NativeOIDC/issues/133) Swift 6 language mode | Open, milestone v1.0.0 | Lifting the Swift 5 pin may change `Sendable` requirements on generated types and on the `NativeHttpTransport` protocol that hosts implement. That would be a source break for Swift hosts, so the evaluation proposes Later 0.x for it (the milestone is the maintainer's decision) |
| [#253](https://github.com/ProjectFlora/NativeOIDC/issues/253) workflow names | Open, milestone v0.10.0 | Reason for citing steps instead of job names |
| [#254](https://github.com/ProjectFlora/NativeOIDC/issues/254) implementation names | Open, milestone v0.10.0 | Public naming mismatches are recorded above and left to #254 |
| [#77](https://github.com/ProjectFlora/NativeOIDC/issues/77), [#78](https://github.com/ProjectFlora/NativeOIDC/issues/78) | Open | Remain the pre-tag gates for the v1.0.0 candidate |

## Unverified and uncertain

- No live identity provider was used. S4 is derived from the published issuer
  identifiers of common providers and from local tests, not from a run
  against those providers.
- iOS runs only on simulators; Android only on x86_64 emulators (API 28 and
  35); Windows browser callbacks are simulated in CI. Real browsers on macOS
  and Windows are checked only by the manual
  [real-browser verification](real-browser-verification.md).
- Linux: only the Rust unit and integration tests run on Linux (`ubuntu-latest`,
  x86_64). No Linux application, browser, or callback path has ever run.
- D7, D8, D9 are from static reading only.
- The Kotlin `toString()` and Swift reflection outputs (G7) were read from the
  generated bindings, not observed in a running app.
- RFC and OIDC requirement levels were checked against the primary documents
  on 2026-11-27. Core §12.2 states its rules as requirements on the OP; the
  RP-side check proposed in G1 is defensive and is not itself a MUST in the
  spec.

### Not evidence

- Rust core tests passing on Linux CI are not evidence of Linux application
  integration.
- Deployment-target compilation is not execution on the minimum OS.
- Mock-provider flows are not interoperability with real providers.
- Job names in CI (#253) are not evidence of what a job verifies.

## Linux options

| Option | What it means | Pros | Cons |
| --- | --- | --- | --- |
| A. Rust core + host implementation | Declare Linux supported through the Rust API, used as a git dependency. The host opens the browser (for example `xdg-open`), runs the loopback listener, and stores state | No SDK work; the core already builds and passes its tests on Linux | Makes the Rust API a supported 1.0.0 surface without a distribution channel (#64 defers crates.io), without a listener or browser code, and without any Linux execution evidence. Hosts must write the security-sensitive loopback listener themselves |
| B. Dedicated Linux integration | Add a Linux loopback listener and browser launch (probably by making the Windows listener portable) and a Linux package or C ABI before 1.0.0 | Real Linux support with the same callback contract as Windows and macOS | New listener port, packaging, CI on a desktop-capable runner, and a distribution decision (conflicts with #64 unless a non-crates.io channel is chosen). It would go into v0.10.0 or Later 0.x and enlarge the 0.x work |
| C. Defer Linux | Linux stays a development and CI host for 1.0.0. The docs state it as an explicit 1.0.0 constraint and say the Rust API can be used on Linux without a support guarantee | Matches the current support matrix and #64; no new 1.0.0 commitment without evidence | Linux desktop apps get no supported path until after 1.0.0 |

**Decision (maintainer, 2026-11-27): C.** This was also the recommendation.
Nothing is in place to back a Linux guarantee:
there is no package, no listener, and no execution evidence. Option A would
promise a Rust API compatibility surface that #64 chose not to publish.
Revisit after 1.0.0 together with #64, or earlier if a Linux consumer appears;
option B is the path then.

## Release bucket proposal

The release direction is v0.10.0 before the v1.0.0 candidate, with later 0.x
releases possible. Bucket rule: `v0.10.0` for breaking changes, saved-format
changes that need migration, or changes worth shipping early for validation;
`Later 0.x` for the same kind of change when it depends on another item or an
unsettled decision; `v1.0.0` for compatible hardening; `v1.0.0後` for work after
1.0.0.

**Status: approved by the maintainer on 2026-11-27.** `Later 0.x` issues
have no milestone; the bucket is written in the issue.

| ID | Follow-up | Bucket | Priority | Effort | Reason for the bucket |
| --- | --- | --- | --- | --- | --- |
| G1 ([#260](https://github.com/ProjectFlora/NativeOIDC/issues/260)) | Record the validated ID Token claims in the state and compare `iss`, `sub`, `aud`, `auth_time`, and `azp` of a refreshed ID Token with them (S1, D2) | v0.10.0 | High | Medium | Changes the saved state JSON (needs a migration rule for older state) and makes refresh reject tokens it accepts today |
| G2 ([#261](https://github.com/ProjectFlora/NativeOIDC/issues/261)) | Accept issuer identifiers without a path: carry the issuer as an exact string for discovery and `iss` comparison (S4, D4) | v0.10.0 | High | Medium | Interoperability blocker on every surface. The fix may change the Rust `issuer: Url` signatures and the value returned by `FfiServiceConfiguration.issuer()`. Worth shipping early to validate |
| G3 ([#262](https://github.com/ProjectFlora/NativeOIDC/issues/262)) | Expose the validated identity (issuer, subject, and selected claims such as `auth_time`, `acr`, `amr`) on every surface: Rust, UniFFI, C ABI JSON, .NET (S3) | v0.10.0 | High | High | New public API on all surfaces, built on G1's stored claims. Needed for account binding and for the UserInfo `sub` check. Shipping it in 0.x lets the record shape be adjusted before 1.0.0 |
| G4 ([#263](https://github.com/ProjectFlora/NativeOIDC/issues/263)) | Support `max_age` (and requested `acr`) with `auth_time`/`acr` validation (S2, D3) | Later 0.x | Medium | Medium | Depends on G1 and G3. Adding an option field is a record change |
| G5 ([#264](https://github.com/ProjectFlora/NativeOIDC/issues/264)) | Compare the authorization response `iss` with the configured issuer when present, and honor `authorization_response_iss_parameter_supported` from discovery (D1) | v0.10.0 | Medium | Medium | Changes which callbacks are accepted and may add an error variant |
| G6 ([#265](https://github.com/ProjectFlora/NativeOIDC/issues/265)) | Validate callback and registration error text the same way as token errors, so control characters cannot reach error messages (D6) | v0.10.0 | Medium | Low | Changes error classification and messages |
| G7 ([#266](https://github.com/ProjectFlora/NativeOIDC/issues/266)) | Document that `FfiTokenResponse` and error objects print tokens and untrusted provider text on Swift and Kotlin; drop the unneeded `Debug` derives on Windows request types (logging review) | v1.0.0 | Medium | Low | Documentation plus an internal change; no API change |
| G8 ([#267](https://github.com/ProjectFlora/NativeOIDC/issues/267)) | Document the host's `token_type` check (case-insensitive `Bearer`) and the UserInfo `sub` comparison (D5) | v1.0.0 | Low | Very Low | Documentation only |
| G9 ([#268](https://github.com/ProjectFlora/NativeOIDC/issues/268)) | Allow `logout_hint` and `ui_locales` on end-session requests (D8) | v1.0.0後 | Low | Low | Additive API for an optional feature |
| G10 ([#269](https://github.com/ProjectFlora/NativeOIDC/issues/269)) | Decide and record the loopback-HTTP endpoint policy (D7): keep it for local development with a recorded reason, or require HTTPS for revocation (RFC 7009 §2) | v0.10.0 | Low | Low | If HTTPS is required, configurations that work today will be rejected |
| G11 ([#270](https://github.com/ProjectFlora/NativeOIDC/issues/270)) | Accept IPv6 loopback (`http://[::1]`) redirects on macOS and Windows (D9) | v1.0.0後 | Low | Medium | Additive; RECOMMENDED only |
| — | Linux | Option C (decided) | — | — | No work item; revisit after 1.0.0 together with #64 |
| — | #133 (Swift 6) | Proposed: Later 0.x (recorded only) | — | — | May change `Sendable` requirements on host-implemented protocols (see Related issues). Its milestone is unchanged |

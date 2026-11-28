# Naming Audit

This audit checks implementation and public API names against what they do,
and against the matching name on the other surfaces
([#254](https://github.com/ProjectFlora/NativeOIDC/issues/254)). Renaming a
public name after 1.0.0 is a breaking change, so each public mismatch is
settled in a 0.x release.

- **Baseline:** `main` at `007053b` (2026-11-28). Every `path:line` below
  refers to that commit
- **Surfaces:** Rust exports (`src/lib.rs`); UniFFI (`src/ffi.rs`, which
  gives the Swift and Kotlin names); the Swift adapters (`Sources/NativeOIDC/`);
  the Kotlin adapter (`native/android/src/main/kotlin/`); the Windows C ABI
  (`native/windows/include/native_oidc_windows.h`, `src/windows/abi.rs`,
  [windows-api.md](windows-api.md)); the .NET wrapper
  (`native/windows/dotnet/NativeOidcWindows.cs`); the JSON keys of the saved
  state and of the Windows operation requests; internal modules, functions,
  types, and test names; and the scripts under `scripts/` that no workflow
  runs
- **Not covered:** workflow, job, and step names, and the scripts that a
  workflow runs, directly or through another script. Those belong to
  [#253](https://github.com/ProjectFlora/NativeOIDC/issues/253)

## Classifications

| Classification | Meaning | Where the work goes |
| --- | --- | --- |
| Public rename | A breaking change to a public name | Its own issue, in a release bucket from the [release bucket proposal](oidc-platform-evaluation.md#release-bucket-proposal) |
| Internal rename | No compatibility impact | Done with this audit |
| Documentation only | The name is acceptable, but its documentation is wrong or incomplete | Done with this audit |
| No change | Recorded with the reason | — |

Every public rename below is a breaking change that depends on no other open
item, so the bucket rule puts each in `v0.10.0`.

## Public renames

| ID | Location | Name | What it does | Issue |
| --- | --- | --- | --- | --- |
| P1 | `src/error.rs:9`, `src/ffi.rs:19`, `src/ffi.rs:68` | `AuthError::InvalidConfiguration`, exposed to Swift and Kotlin as `FfiError::InvalidRequest` | Reports invalid caller input of any kind, including call arguments and state ("authentication state has no refresh token"). The two surfaces use different names, and `InvalidRequest` reads like the OAuth `invalid_request` error code | [#291](https://github.com/ProjectFlora/NativeOIDC/issues/291), `v0.10.0` |
| P2 | `src/ffi.rs:47`, `src/ffi.rs:209` | `FfiError::Transport` next to `FfiError::TransportFailure` | Swift and Kotlin can never receive `Transport`: the `NativeTransport` wrapper turns every non-cancel transport error into `TransportFailure` ([api-coverage.md](api-coverage.md), "Variants these fixtures cannot produce") | [#292](https://github.com/ProjectFlora/NativeOIDC/issues/292), `v0.10.0` |
| P3 | `native/windows/include/native_oidc_windows.h:18,24,102,107,111`, `src/windows/abi.rs:27,459,477,492` | `NativeOidcWindowsAuthorizationResult`, `NativeOidcWindowsAuthorizationStatus`, `NATIVE_OIDC_WINDOWS_AUTHORIZATION_*`, `native_oidc_windows_authorization_result_{status,value,free}` | The result handle, status values, and accessors of every operation: refresh, end-session, revocation, and ID Token claims return the same handle | [#293](https://github.com/ProjectFlora/NativeOIDC/issues/293), `v0.10.0` |
| P4 | `src/authorization/config.rs:72`, `src/ffi.rs:282` | `with_oidc_validation_endpoints(issuer, jwks_uri)` | Configures ID Token validation. The issuer is an identifier compared with `iss`, not an endpoint | [#294](https://github.com/ProjectFlora/NativeOIDC/issues/294), `v0.10.0` |

## Internal renames

All of these were done with this audit.

| ID | Location | Old name | New name and reason |
| --- | --- | --- | --- |
| I1 | `src/authorization/validation.rs:71`, `src/authorization/config.rs:50` | `validate_logout_endpoint` | `validate_end_session_endpoint`: the API, the docs, and the C ABI call it end session |
| I2 | `src/authorization/validation.rs:98`, `src/authorization/config.rs:79` | `validate_oidc_endpoint` | `validate_jwks_endpoint`: it checks only the JWKS URI |
| I3 | `src/authorization/validation.rs:248`, `src/authorization/session.rs:124` | `reject_reserved_endpoint_parameters` | `reject_reserved_authorization_endpoint_parameters`: it checks only the authorization endpoint query; the token and revocation endpoints have their own checks |
| I4 | `src/windows/data.rs:88` and its uses in `src/windows/` | `WindowsAuthorizationError` | `WindowsOperationError`: refresh, end-session, and revocation return it too |
| I5 | `src/windows/data.rs:93`, `src/windows/ops.rs:50,67,115` | Message "authorization or token endpoint URL is invalid" | "an endpoint URL is missing or invalid": the same error reports a missing or malformed revocation, end-session, or JWKS endpoint, and a request with only one of the authorization and token endpoints. No test or document matched the old text |
| I6 | `src/authorization/end_session.rs:287` | Test `logout_callback_rejects_malformed_encoding_and_encoded_duplicate_state` | `end_session_callback_rejects_...`, as in I1 |
| I7 | `native/windows/dotnet/NativeOidcWindows.cs:291` | Private `NativeMethods.AuthorizationResultHandle` | `ResultHandle`: every operation's result uses it. The class is private, so consumers cannot reach it |

## Documentation only

All of these were fixed with this audit. The committed Swift bindings
(`Sources/NativeOIDC/native_oidc.swift`) carry the UniFFI doc comments and
were updated with them.

| ID | Location | Name | Correction |
| --- | --- | --- | --- |
| D1 | `src/ffi.rs:306` | `FfiServiceConfiguration.with_revocation_endpoint` | The doc said HTTPS; HTTP on a loopback host is also accepted, as in the Rust method |
| D2 | `src/windows/abi.rs:21,247` and the cancel and free docs | `NativeOidcWindowsCancellationHandle` | The docs said "authorization call"; the handle serves every blocking operation |
| D3 | `src/windows/abi.rs:26` and the result accessor safety docs | `NativeOidcWindowsAuthorizationResult` | The docs said the handle comes from `native_oidc_windows_authorize`; every operation's result is accepted. The names themselves are P3 |
| D4 | `native/windows/dotnet/NativeOidcWindows.cs:9` | `NativeOidcWindows` | The class summary named authorization only; it also refreshes, ends sessions, revokes, and reads ID Token claims |
| D5 | `src/ffi.rs:397` | `FfiJsonParameter` | The doc said "token response parameter"; `FfiIdTokenClaims.additional_claims` also uses it |
| D6 | `src/ffi.rs:549`, `src/authorization/state.rs:21` | `access_token` | The doc said "bearer access token"; the library does not check that `token_type` is `Bearer` |

## No change

| Location | Name | Reason |
| --- | --- | --- |
| `src/authorization/revocation.rs:29`, `src/ffi.rs:357`, `native/windows/include/native_oidc_windows.h:83`, `native/windows/dotnet/NativeOidcWindows.cs:145` | `revoke_token` (Rust) vs `revoke`, `native_oidc_windows_revoke`, `RevokeAsync` | Every surface says "revoke". The Rust function sits at the crate root next to other operations, so it names its object |
| Prose in doc comments and guides | "logout" next to "end session" | Every API name says end session. Prose uses "logout" where the spec does (OpenID Connect RP-Initiated Logout, `post_logout_redirect_uri`). The one internal helper named "logout" is I1 |
| Generated bindings | `FfiError` (Swift) vs `FfiException` (Kotlin) | The UniFFI naming convention for each language; [api-coverage.md](api-coverage.md) lists both |
| `Sources/NativeOIDC/IOSAuthorizationAdapter.swift:8`, `Sources/NativeOIDC/MacOSAuthorizationAdapter.swift:11`, `native/android/src/main/kotlin/net/projectflora/nativeoidc/AndroidAuthorizationAdapter.kt:53,61` | `authorizationSessionAlreadyRunning` vs `AuthorizationAlreadyRunning`; `browserCouldNotBeOpened` vs `BrowserUnavailable` | Each follows its platform's error idiom and names its own failure: Android reports that no browser can handle the intent, macOS that the opener failed. The adapter callback and cancellation contract belongs to [#246](https://github.com/ProjectFlora/NativeOIDC/issues/246) |
| `src/ffi.rs:697`, `src/authorization/state.rs:328` | `FfiAuthenticationState.refresh` vs `refresh_with_additional_parameters` | The UniFFI surface has one refresh method, which takes the list of additional parameters |
| `src/error.rs:33` | `AuthError::AuthorizationServer` for token and revocation endpoint errors | Those endpoints belong to the authorization server (RFC 6749, section 1.1) |
| `src/authorization/state.rs:53`, `src/windows/data.rs:8` | Saved state keys (`token_response`, `expires_at`, `oidc_nonce`, `id_token_claims`) and Windows request keys | They use the OAuth and OpenID Connect parameter and metadata names (`issuer`, `jwks_uri`, `end_session_endpoint`, `token_type_hint`, ...). `scopes` is a list, unlike the space-separated `scope` parameter |
| `src/windows/data.rs:113` | `ConfigurationKind::Oidc` | Its doc comment says it attaches the OIDC validation endpoints (issuer and JWKS URI), which authorization and refresh need |
| `scripts/agent-precheck.sh` | — | Run by the Orca automations, not a workflow; the name says what it is |
| `scripts/check-macos-real-browser.sh`, `scripts/check-windows-real-browser.ps1`, `scripts/real_browser_mock_provider.py` | — | Run by hand ([real-browser-verification.md](real-browser-verification.md)). The `check-*` scripts generate bindings or unpack a package only to set up the check whose result they report. #253 decides the `check-` prefix convention for the scripts that workflows run; these follow its decision |

Every other script under `scripts/` is run by a workflow, directly or through
another script, and belongs to #253.

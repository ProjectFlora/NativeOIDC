# Real system-browser verification (macOS and Windows)

CI exercises the macOS and Windows loopback flows with a simulated browser:
the tests replace the browser-opening step with a closure that sends the
callback request directly. The code that opens the real browser
(`NSWorkspace.shared.open` in `MacOSAuthorizationAdapter`, `ShellExecuteW`
in the Windows C ABI) and the browser following the provider's redirect to
the loopback listener are not run in CI
([#65](https://github.com/ProjectFlora/NativeOIDC/issues/65)).

This procedure runs that path on a developer machine. Run it for each
release and record the result (see [Record the result](#record-the-result)).

## What it checks

Each run performs one authorization code flow with PKCE against a local
mock provider, [`scripts/real_browser_mock_provider.py`](../scripts/real_browser_mock_provider.py):

1. The library binds the loopback listener on `127.0.0.1` with an
   OS-chosen port and opens the authorization URL in the **default system
   browser**, through the same public entry point an application uses: the
   default `MacOSAuthorizationAdapter()` on macOS, and
   `NativeOidcWindows.AuthorizeAsync` (which calls
   `native_oidc_windows_authorize`) on Windows.
2. The browser loads `/authorize` from the provider. The provider checks the
   request and records the browser's `User-Agent`.
3. The provider redirects the browser to the loopback callback with a code
   and the request's state. The browser shows
   `Authorization complete. You may close this window.`
4. The library validates the callback, exchanges the code (the provider
   checks the code, redirect URI, client ID, and PKCE verifier), and returns
   the authentication state.

The run passes when the script reports success and the provider result
records the request from the browser. No user interaction is needed in the
browser.

It does not check:

- a real identity provider's sign-in pages or consent (see
  [live identity-provider interoperability](live-idp-interoperability.md));
- end-session with a loopback post-logout redirect, because the end-session
  endpoint must use HTTPS, which the local provider does not serve;
- browsers other than the current default (change the default browser and
  run again to cover another one).

## Prerequisites

Both platforms:

- A checkout of the commit or release tag being verified.
- Python 3 (standard library only).
- A desktop session where a browser window can open. A remote session works;
  a session without a desktop (an SSH session on macOS, a service account on
  Windows) does not.
- The default browser has completed its first-run setup. A first-run or
  "set as default" prompt can stop the browser from loading the URL.

macOS:

- macOS 15 or later, Xcode command line tools, and a stable Rust toolchain
  (at least the `rust-version` in [`Cargo.toml`](../Cargo.toml)).

Windows:

- Windows 10 or 11 with the .NET 8 SDK.
- A Windows package ZIP: the `native-oidc-windows-X.Y.Z.zip` release asset,
  or the output of `scripts/package-windows.ps1` for an unreleased commit.

## Run on macOS

From the repository root, in Terminal:

```sh
scripts/check-macos-real-browser.sh
```

The script builds the Rust library and Swift bindings, builds
`target/apple/mac-real-browser-check` from
[`native/apple/MacOSRealBrowserCheck.swift`](../native/apple/MacOSRealBrowserCheck.swift),
starts the provider, and runs the check. The default browser opens a tab with
the completion message. Set `REAL_BROWSER_TIMEOUT` (seconds, default 120) to
change how long the check waits for the callback.

Results are written to `target/real-browser/macos/` (or the directory given
as the first argument).

## Run on Windows

From the repository root, in PowerShell 7:

```powershell
scripts/check-windows-real-browser.ps1 -Package native-oidc-windows-X.Y.Z.zip
```

The script builds
[`native/consumer-smoke/windows/real-browser`](../native/consumer-smoke/windows/real-browser/Program.cs)
against the package, starts the provider, and runs the check. The default
browser opens a tab with the completion message. Options:
`-ResultDirectory` (default `target/real-browser/windows`),
`-TimeoutSeconds` (default 120), and `-Python` (default `python`; use `py`
if that is how Python is installed).

## Check the result

Each run writes these files to the result directory:

| File | Content |
| --- | --- |
| `environment.txt` | Date, commit, OS version, and architecture (and the package on Windows) |
| `check-output.txt` | The default browser (macOS: the application `NSWorkspace` opens for `http` URLs; Windows: the `http` `UserChoice` ProgId) and the outcome |
| `provider-result.json` | `completed`, the browser's `authorization_user_agent`, the `redirect_uri` it was given, and any `errors` |
| `provider-log.txt` | The provider's request log |

A run passes when all of the following hold:

- The script ends with `Real system-browser check passed`.
- `provider-result.json` has `"completed": true` and an empty `errors` list.
- `authorization_user_agent` is the default browser's user agent (for
  example one that names Safari, Edg, Chrome, or Firefox), not a
  non-browser HTTP client such as `CFNetwork` or `curl`.
- `redirect_uri` is `http://127.0.0.1:<port>/oauth/callback`.

If the check times out, look at the browser window: a first-run page, a
prompt to choose a default browser, or a blocked redirect leaves the
authorization URL unloaded or the callback unsent. `provider-log.txt` shows
how far the browser got (`GET /authorize` reached the provider, but no
`POST /token` means the callback did not reach the library).

## Record the result

For each release, run both platforms at the release tag (on Windows, with
the published `native-oidc-windows-X.Y.Z.zip`), and add a comment to the
release tracking issue:

```text
Real system-browser verification for vX.Y.Z (docs/real-browser-verification.md)

| Platform | OS version / architecture | Default browser and version | Commit or package | Result | Date |
| --- | --- | --- | --- | --- | --- |
| macOS |  |  |  | pass / fail |  |
| Windows |  |  |  | pass / fail |  |

Notes: (failures, prompts that had to be dismissed, other browsers tried)
```

Attach or paste `provider-result.json` from each run. Record a failure as a
failure, together with what was observed, rather than rerunning until it
passes.

## Automation status

A feasibility trial in [#65](https://github.com/ProjectFlora/NativeOIDC/issues/65)
ran these scripts unchanged on GitHub-hosted runners in November 2026. The
runners have a logged-in desktop session, and the default browser loaded
the authorization URL and followed the redirect without first-run prompts:
Safari on `macos-15` (macOS 15.8) and `macos-26` (macOS 26.7), and
Microsoft Edge on `windows-latest` (Windows Server 2025) and
`windows-11-arm` (Windows 11, x64 emulation). Whether to add a CI job is decided in #65. Until one exists,
this manual procedure is the only regular check of the real system-browser
path, and a CI job would not replace a run on a client machine with a
user-chosen default browser.

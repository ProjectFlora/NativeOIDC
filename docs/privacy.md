# Privacy statement

This program will not transfer any information to other networked systems
unless specifically requested by the user or the person installing or
operating it.

NativeOIDC is a library. It runs inside the application that embeds it and
acts only when that application calls it. In detail:

- **Network requests go only where the host application points them.** The
  library sends HTTP requests to the identity provider endpoints that the
  application configures: the issuer's discovery document, and the
  authorization, token, revocation, registration, end-session, and JWKS
  endpoints that the application sets or that the configured issuer's
  discovery document names. With a custom HTTP transport supplied by the
  application, the application sends these requests itself.
- **The browser opens only for the requests the application starts.** For
  sign-in and sign-out, the platform adapters open the system browser (or
  the platform's authentication session) at the identity provider's URL.
  For a loopback redirect, as on Windows, the library listens on
  `127.0.0.1` for the browser's redirect only until that operation ends.
- **No telemetry.** The library contains no analytics, crash reporting,
  usage statistics, update checks, or advertising, and contacts no server
  operated by ProjectFlora or any other party on its own.
- **No storage.** The library keeps tokens and authentication state in
  memory and returns them to the application. It writes no files, logs, or
  settings; storing the state securely is the application's responsibility
  (see [Save, restore, and refresh](user-guide.md#save-restore-and-refresh)
  in the user guide).

What the identity provider does with the requests it receives is governed by
that provider's own privacy policy, which the application's operator
chooses. An application that embeds NativeOIDC is responsible for its own
privacy policy towards its users.

Questions about this statement can be raised in the
[issue tracker](https://github.com/ProjectFlora/NativeOIDC/issues).

#if os(macOS)
  import AppKit
  import Foundation

  /// Runs one authorization through the default `MacOSAuthorizationAdapter`,
  /// which opens the authorization URL with `NSWorkspace` in the user's
  /// default browser, against `scripts/real_browser_mock_provider.py`.
  ///
  /// Usage: `mac-real-browser-check <provider base URL> [timeout seconds]`.
  /// See docs/real-browser-verification.md.
  @main
  struct MacOSRealBrowserCheck {
    static func main() async {
      let arguments = CommandLine.arguments
      guard arguments.count == 2 || arguments.count == 3,
        let timeout = arguments.count == 3 ? Double(arguments[2]) : 120,
        timeout > 0
      else {
        FileHandle.standardError.write(
          Data("usage: mac-real-browser-check <provider base URL> [timeout seconds]\n".utf8))
        exit(2)
      }
      do {
        try await run(providerBaseURL: arguments[1], timeout: timeout)
        print("real system-browser authorization completed")
      } catch {
        FileHandle.standardError.write(
          Data("real system-browser authorization failed: \(error)\n".utf8))
        exit(1)
      }
    }

    @MainActor
    private static func run(providerBaseURL: String, timeout: Double) async throws {
      let configuration = try FfiServiceConfiguration(
        authorizationEndpoint: "\(providerBaseURL)/authorize",
        tokenEndpoint: "\(providerBaseURL)/token"
      )
      let options = FfiAuthorizationOptions(
        clientId: "real-browser-check",
        redirectUri: "http://127.0.0.1/oauth/callback",
        scopes: ["profile"],
        additionalParameters: [],
        additionalTokenParameters: []
      )
      // NSWorkspace opens http URLs with this application.
      let browser = NSWorkspace.shared.urlForApplication(toOpen: URL(string: "http://127.0.0.1/")!)
      print("default browser: \(browser?.path ?? "none")")
      let adapter = MacOSAuthorizationAdapter()
      let timeoutTask = Task { @MainActor in
        try await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
        FileHandle.standardError.write(
          Data("no callback within \(timeout) seconds; cancelling\n".utf8))
        adapter.cancel()
      }
      defer { timeoutTask.cancel() }
      let state = try await adapter.authorize(
        configuration: configuration,
        options: options,
        transport: URLSessionNativeHttpTransport()
      )
      let tokenResponse = await state.tokenResponse()
      guard tokenResponse.accessToken == "real-browser-access-token" else {
        throw URLError(.badServerResponse)
      }
    }
  }
#endif

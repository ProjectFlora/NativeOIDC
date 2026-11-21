#if os(macOS)
  import Foundation

  /// Runs the complete macOS authorization flow against the mock provider,
  /// linked directly against the Rust library. The loopback listener,
  /// transport, and adapter error checks are XCTest cases in
  /// `SwiftTests/NativeOIDCTests`.
  @main
  struct MacOSAuthorizationFlowSmoke {
    static func main() async throws {
      let provider = try MacOSMockOAuthProvider()
      let providerPort = try await provider.start()
      do {
        let configuration = try FfiServiceConfiguration(
          authorizationEndpoint: "http://127.0.0.1:\(providerPort)/authorize",
          tokenEndpoint: "http://127.0.0.1:\(providerPort)/token"
        )
        let options = FfiAuthorizationOptions(
          clientId: "macos-smoke-client",
          redirectUri: "http://127.0.0.1/oauth/callback",
          scopes: ["profile"],
          additionalParameters: [],
          additionalTokenParameters: []
        )
        let adapter = MacOSAuthorizationAdapter { authorizationURL in
          let (_, response) = try await URLSession.shared.data(from: authorizationURL)
          guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw URLError(.badServerResponse)
          }
        }
        let authenticationState = try await adapter.authorize(
          configuration: configuration,
          options: options,
          transport: URLSessionNativeHttpTransport()
        )
        let tokenResponse = await authenticationState.tokenResponse()
        precondition(tokenResponse.accessToken == "mock-access-token")
        precondition(tokenResponse.scope == "profile")
        try await provider.verifyCompletedFlow()
      } catch {
        await provider.stop()
        throw error
      }
      await provider.stop()
    }
  }
#endif

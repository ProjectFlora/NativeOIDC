import AuthenticationServices
import Foundation

// Compile-time mirror of the iOS/iPadOS examples in docs/swift-guide.md: the
// presentation-context provider, the `AuthService`, and the Objective-C
// wrapper. Compiled by scripts/check-apple-release-consumer.sh for the iOS
// simulator against the release archive's generated bindings and shipped
// sources (there is no `import NativeOIDC` here: the symbols are already in
// scope).

/// Supplies the presentation anchor for the authentication session.
final class GuideIOSPresentationProvider: NSObject,
  ASWebAuthenticationPresentationContextProviding {
  func presentationAnchor(
    for session: ASWebAuthenticationSession
  ) -> ASPresentationAnchor {
    UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .flatMap(\.windows)
      .first(where: \.isKeyWindow) ?? ASPresentationAnchor()
  }
}

/// The guide's iOS/iPadOS authorization example: the authentication session
/// and the callback code exchange.
@MainActor
final class GuideIOSAuthService {
  private let adapter = IOSAuthorizationAdapter(
    // Set true to avoid sharing cookies with the user's regular browsing
    // session.
    presentationContextProvider: GuideIOSPresentationProvider(),
    prefersEphemeralWebBrowserSession: true
  )

  /// Opens the authentication session and completes code exchange.
  func signIn(
    configuration: FfiServiceConfiguration,
    options: FfiAuthorizationOptions,
    transport: URLSessionNativeHttpTransport
  ) async throws -> FfiAuthenticationState {
    try await adapter.authorize(
      configuration: configuration,
      options: options,
      transport: transport
    )
  }

  /// Cancels the browser session and any in-flight core request.
  func cancel() {
    adapter.cancel()
  }
}

/// The guide's iOS/iPadOS Objective-C wrapper.
@MainActor
@objc public final class GuideIOSObjCService: NSObject {
  private let adapter: IOSAuthorizationAdapter
  private var activeCancellation: FfiCancellationHandle?
  private var activeTask: Task<Void, Never>?

  @objc public override init() {
    self.adapter = IOSAuthorizationAdapter(
      presentationContextProvider: GuideIOSPresentationProvider(),
      prefersEphemeralWebBrowserSession: false
    )
    super.init()
  }

  /// Runs authorization in the system browser session. The completion
  /// handler reports the serialized state JSON (or `nil`) and any error.
  @objc public func authorize(
    issuer: String,
    clientId: String,
    redirectUri: String,
    scopes: [String],
    completion: @escaping @Sendable (String?, Error?) -> Void
  ) {
    let transport = URLSessionNativeHttpTransport()
    let cancellation = FfiCancellationHandle()
    let task = Task { @MainActor in
      do {
        // Discovery is a core request, not an adapter operation: signal the
        // shared handle so a cancel while it is in flight ends it instead of
        // letting the task proceed to open the browser.
        let configuration = try await FfiServiceConfiguration.discover(
          issuer: issuer,
          transport: transport,
          cancellation: cancellation
        )
        // A cancel that lands between discovery and the adapter must not
        // start the session either.
        guard !cancellation.isCancelled() else { throw FfiError.Cancelled }
        let options = FfiAuthorizationOptions(
          clientId: clientId,
          redirectUri: redirectUri,
          scopes: scopes,
          additionalParameters: [],
          additionalTokenParameters: []
        )
        let state = try await adapter.authorize(
          configuration: configuration,
          options: options,
          transport: transport
        )
        completion(try await state.toJson(), nil)
      } catch {
        completion(nil, error)
      }
    }
    activeCancellation = cancellation
    activeTask = task
  }

  /// Cancels the active browser session, any in-flight core request, and a
  /// suspended discovery request alike.
  @objc public func cancel() {
    adapter.cancel()
    activeCancellation?.cancel()
    activeTask?.cancel()
    activeCancellation = nil
    activeTask = nil
  }
}

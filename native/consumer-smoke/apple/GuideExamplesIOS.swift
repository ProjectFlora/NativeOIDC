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

/// An `authorize` call was rejected because an operation is already in
/// flight.
struct AuthorizationInProgress: Error {}

/// The guide's iOS/iPadOS Objective-C wrapper.
@MainActor
@objc public final class GuideIOSObjCService: NSObject {
  private let adapter: IOSAuthorizationAdapter
  // The guide's example creates this inside `authorize`; the wrapper stores
  // it so checks can substitute a scripted transport (see the seam below)
  // while keeping the same claim, cancellation, and completion path.
  private let transport: any NativeHttpTransport
  private var activeCancellation: FfiCancellationHandle?
  private var activeTask: Task<Void, Never>?
  // Bumped for every claimed operation, so a superseded task cannot release
  // a slot that a newer operation owns.
  private var inFlightGeneration = 0

  @objc public override init() {
    self.adapter = IOSAuthorizationAdapter(
      presentationContextProvider: GuideIOSPresentationProvider(),
      prefersEphemeralWebBrowserSession: false
    )
    self.transport = URLSessionNativeHttpTransport()
    super.init()
  }

  // Internal seam: the checks substitute a scripted transport.
  init(
    adapter: IOSAuthorizationAdapter,
    transport: any NativeHttpTransport
  ) {
    self.adapter = adapter
    self.transport = transport
  }

  /// Runs authorization in the system browser session. The completion
  /// handler reports the serialized state JSON (or `nil`) and any error.
  /// A call that arrives while an operation is in flight reports
  /// `AuthorizationInProgress` — including one that arrives right after a
  /// `cancel()`, while the cancelled operation is still unwinding.
  @objc public func authorize(
    issuer: String,
    clientId: String,
    redirectUri: String,
    scopes: [String],
    completion: @escaping @Sendable (String?, Error?) -> Void
  ) {
    // Claim the single in-flight slot before discovery starts: a second
    // call must be rejected here, not replace the first operation's task
    // and handle, which would leave its discovery un-cancellable.
    guard activeTask == nil else {
      completion(nil, AuthorizationInProgress())
      return
    }
    inFlightGeneration += 1
    let generation = inFlightGeneration
    let cancellation = FfiCancellationHandle()
    let task = Task { @MainActor in
      // Release the slot before calling the completion handler, so a host
      // that retries directly from the completion can claim it. The
      // generation gate keeps a superseded task from releasing a slot a
      // newer operation owns.
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
        let stateJson = try await state.toJson()
        if inFlightGeneration == generation {
          activeTask = nil
          activeCancellation = nil
        }
        completion(stateJson, nil)
      } catch {
        if inFlightGeneration == generation {
          activeTask = nil
          activeCancellation = nil
        }
        completion(nil, error)
      }
    }
    activeCancellation = cancellation
    activeTask = task
  }

  /// Cancels the active browser session, any in-flight core request, and a
  /// suspended discovery request alike. The in-flight slot stays claimed
  /// until the cancelled operation finishes unwinding — the adapter may
  /// still be busy — so a retry right after a `cancel()` is rejected rather
  /// than racing it; the slot is released when the operation ends.
  @objc public func cancel() {
    adapter.cancel()
    activeCancellation?.cancel()
    activeTask?.cancel()
  }
}

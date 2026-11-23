import AppKit
import Foundation

// Compile-time mirror of the macOS examples in docs/swift-guide.md: the
// loopback `AuthService`, the "using the result" flow, and the Objective-C
// wrapper. Compiled by scripts/check-apple-release-consumer.sh against the
// release archive's generated bindings and shipped sources (there is no
// `import NativeOIDC` here: the symbols are already in scope).

/// The guide's macOS authorization example: the loopback listener, the
/// external browser, and the code exchange.
@MainActor
final class GuideMacOSAuthService {
  // The default closure opens the URL in the default browser via NSWorkspace;
  // hosts that manage browser presentation themselves can supply their own.
  private let adapter = MacOSAuthorizationAdapter()

  /// Binds the loopback listener, opens the browser, and exchanges the
  /// callback code.
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

  /// Stops the listener and any in-flight core request. It cannot dismiss
  /// the external browser.
  func cancel() {
    adapter.cancel()
  }
}

/// The guide's "Using the result" flow.
@MainActor
final class GuideMacOSSignInHost {
  func performSignIn(
    configuration: FfiServiceConfiguration,
    options: FfiAuthorizationOptions,
    transport: URLSessionNativeHttpTransport
  ) async throws -> FfiAuthenticationState {
    let authService = GuideMacOSAuthService()
    // Return the state to the host: it holds the tokens, the OIDC nonce,
    // and the expiry that the persistence, refresh, and end-session
    // operations in GuideExamplesCommon.swift require.
    return try await authService.signIn(
      configuration: configuration,
      options: options,
      transport: transport
    )
  }
}

/// An `authorize` call was rejected because an operation is already in
/// flight.
struct AuthorizationInProgress: Error {}

/// The guide's macOS Objective-C wrapper.
@MainActor
@objc public final class GuideMacOSObjCService: NSObject {
  private let adapter: MacOSAuthorizationAdapter
  // The guide's example creates this inside `authorize`; the wrapper stores
  // it so the lifecycle check can substitute a scripted transport (see the
  // seam below) while keeping the same claim, cancellation, and completion
  // path.
  private let transport: any NativeHttpTransport
  private var activeCancellation: FfiCancellationHandle?
  private var activeTask: Task<Void, Never>?
  // Bumped for every claimed operation, so a superseded task cannot release
  // a slot that a newer operation owns.
  private var inFlightGeneration = 0

  @objc public override init() {
    // The default closure opens the URL in the default browser.
    self.adapter = MacOSAuthorizationAdapter()
    self.transport = URLSessionNativeHttpTransport()
    super.init()
  }

  // Internal seam: the checks substitute a scripted transport.
  init(
    adapter: MacOSAuthorizationAdapter,
    transport: any NativeHttpTransport
  ) {
    self.adapter = adapter
    self.transport = transport
  }

  /// Binds the loopback listener and opens the external browser. The
  /// completion handler reports the serialized state JSON (or `nil`) and
  /// any error. A call that arrives while an operation is in flight reports
  /// `AuthorizationInProgress`.
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
        // start the browser either.
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
      // Release the slot once this operation has fully ended — success,
      // failure, or cancellation — only if it is still the claimed one.
      if inFlightGeneration == generation {
        activeTask = nil
        activeCancellation = nil
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

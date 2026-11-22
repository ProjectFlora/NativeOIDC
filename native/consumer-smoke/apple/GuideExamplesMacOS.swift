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
  ) async throws {
    let authService = GuideMacOSAuthService()
    let state = try await authService.signIn(
      configuration: configuration,
      options: options,
      transport: transport
    )
    // `state` is ready for the operations in GuideExamplesCommon.swift.
    _ = state
  }
}

/// The guide's macOS Objective-C wrapper.
@MainActor
@objc public final class GuideMacOSObjCService: NSObject {
  private let adapter: MacOSAuthorizationAdapter
  private var activeCancellation: FfiCancellationHandle?
  private var activeTask: Task<Void, Never>?

  @objc public override init() {
    // The default closure opens the URL in the default browser.
    self.adapter = MacOSAuthorizationAdapter()
    super.init()
  }

  /// Binds the loopback listener and opens the external browser. The
  /// completion handler reports the serialized state JSON (or `nil`) and
  /// any error.
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

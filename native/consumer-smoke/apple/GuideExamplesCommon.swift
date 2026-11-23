import Foundation
import Security

// Compile-time mirror of the platform-neutral examples in
// docs/swift-guide.md: transport and cancellation, service configuration
// (discovered and explicit), authorization options, secure state persistence,
// refresh, end-session, revocation, dynamic registration, and error handling.
// scripts/check-apple-release-consumer.sh compiles the release archive's
// generated bindings and shipped sources as files alongside this one, so the
// guide examples are exercised against the distributed artifacts without an
// `import NativeOIDC` (SPM users keep that import in the guide itself).
//
// The per-platform halves (the authorization adapters, the Objective-C
// wrappers) live in GuideExamplesMacOS.swift and GuideExamplesIOS.swift.

// MARK: - Guide examples

/// The guide's "Configuration, transport, and callbacks" snippet.
func guideTransportAndCancellation()
  -> (transport: URLSessionNativeHttpTransport,
      cancellation: FfiCancellationHandle)
{
  // URLSession-based transport: returns HTTP redirects without following them
  // and disables HTTP caching so the Rust core controls JWKS freshness.
  let transport = URLSessionNativeHttpTransport()

  // Shared cancellation signal; create one per operation.
  let cancellation = FfiCancellationHandle()
  return (transport, cancellation)
}

/// The guide's discovered-configuration example.
func guideDiscoveredConfiguration(
  transport: URLSessionNativeHttpTransport,
  cancellation: FfiCancellationHandle
) async throws -> FfiServiceConfiguration {
  // Fetches and validates the OIDC discovery document.
  let configuration = try await FfiServiceConfiguration.discover(
    issuer: "https://your-idp.example",
    transport: transport,
    cancellation: cancellation
  )
  return configuration
}

/// The guide's explicit-configuration example, including the optional
/// endpoints that the end-session, revocation, and registration examples
/// require.
func guideExplicitConfiguration() throws -> FfiServiceConfiguration {
  let configuration = try FfiServiceConfiguration(
    authorizationEndpoint: "https://your-idp.example/authorize",
    tokenEndpoint: "https://your-idp.example/token"
  )
  .withOidcValidationEndpoints(
    issuer: "https://your-idp.example",
    jwksUri: "https://your-idp.example/jwks"
  )
  // Optional endpoints are added the same way; each must be supported by the
  // provider before the operation that uses it.
  return try configuration
    .withEndSessionEndpoint(endpoint: "https://your-idp.example/session/end")
    .withRevocationEndpoint(endpoint: "https://your-idp.example/revoke")
    .withRegistrationEndpoint(endpoint: "https://your-idp.example/register")
}

/// The guide's authorization-options example.
func guideAuthorizationOptions() -> FfiAuthorizationOptions {
  FfiAuthorizationOptions(
    clientId: "native-public-client",
    // Must exactly match the redirect registered with the provider.
    redirectUri: "https://your-app.example/oidc/callback",
    scopes: ["openid", "profile"],
    // Optional pass-through for the authorization request and token exchange.
    additionalParameters: [FfiParameter(name: "prompt", value: "consent")],
    additionalTokenParameters: []
  )
}

/// The guide's Keychain persistence sketch.
enum GuideStateStorageError: Error {
  case add(OSStatus)
  case missing
}

func guideSaveState(_ json: String, account: String) throws {
  let delete: [CFString: Any] = [
    kSecClass: kSecClassGenericPassword,
    kSecAttrService: "com.example.app",
    kSecAttrAccount: account,
  ]
  SecItemDelete(delete as CFDictionary)
  var add = delete
  add[kSecValueData] = Data(json.utf8)
  let status = SecItemAdd(add as CFDictionary, nil)
  guard status == errSecSuccess else { throw GuideStateStorageError.add(status) }
}

func guideLoadState(account: String) throws -> String {
  let query: [CFString: Any] = [
    kSecClass: kSecClassGenericPassword,
    kSecAttrService: "com.example.app",
    kSecAttrAccount: account,
    kSecReturnData: true,
    kSecMatchLimit: kSecMatchLimitOne,
  ]
  var item: CFTypeRef?
  let status = SecItemCopyMatching(query as CFDictionary, &item)
  guard status == errSecSuccess, let data = item as? Data else {
    throw GuideStateStorageError.missing
  }
  return String(decoding: data, as: UTF8.self)
}

/// The guide's "Secure state persistence" round trip.
func guidePersistState(_ state: FfiAuthenticationState) async throws -> FfiAuthenticationState {
  // After authorization: serialize the complete state, then store it.
  let stateJson = try await state.toJson()
  try guideSaveState(stateJson, account: "native-oidc/state")

  // On the next launch: restore it before any operation that needs the state.
  return try FfiAuthenticationState.fromJson(
    serialized: try guideLoadState(account: "native-oidc/state")
  )
}

/// The guide's refresh example, including re-persistence of the updated
/// complete state, guarded against local sign-out with the guide's lock and
/// flag.
func guideRefresh(
  _ state: FfiAuthenticationState,
  configuration: FfiServiceConfiguration,
  transport: URLSessionNativeHttpTransport
) async throws {
  // Shared with the guide's local sign-out: a refresh that outlives
  // sign-out must not write the deleted state back.
  let sessionLock = NSLock()
  var signedOut = false

  try await state.refresh(
    configuration: configuration,
    clientId: "native-public-client",
    additionalParameters: [],
    transport: transport,
    cancellation: FfiCancellationHandle()
  )

  // Refresh tokens may rotate; persist the updated complete state. Serialize
  // the state outside the lock — `toJson` is a core request, and the lock
  // must not be held across it — then check the flag under the lock.
  let stateJson = try await state.toJson()
  sessionLock.lock()
  if !signedOut {
    try guideSaveState(stateJson, account: "native-oidc/state")
  }
  sessionLock.unlock()
}

/// The guide's end-session example: the prepared session retained across
/// browser presentation. Returns the logout URL together with the session
/// that generated its state: validating the callback needs the original
/// session, and recreating it would generate a different state.
struct GuidePreparedEndSession {
  let logoutURL: URL
  let endSession: FfiEndSessionSession
}

func guidePrepareEndSession(
  _ state: FfiAuthenticationState,
  configuration: FfiServiceConfiguration
) async throws -> GuidePreparedEndSession {
  // Register the app's post-logout redirect with the provider; a random
  // state is then generated and validated on return.
  let endSession = try beginEndSession(
    configuration: configuration, // requires an end-session endpoint
    options: FfiEndSessionOptions(
      clientId: "native-public-client",
      idTokenHint: await state.tokenResponse().idToken,
      // Must exactly match the post-logout redirect registered with the
      // provider; a random state is validated on return.
      postLogoutRedirectUri: "https://your-app.example/oidc/post-logout"
    )
  )

  let logoutURL = URL(string: endSession.endSessionUrl())!
  precondition(
    logoutURL.scheme == "https",
    "the end-session URL must be the HTTPS end-session endpoint"
  )

  // Open `logoutURL` in the external browser, and retain the prepared
  // session until the callback returns.
  return GuidePreparedEndSession(logoutURL: logoutURL, endSession: endSession)
}

/// The guide's callback validation: when the provider returns the app to the
/// registered redirect, validate the callback it carried with the retained
/// session.
func guideValidateEndSessionRedirect(
  _ prepared: GuidePreparedEndSession,
  callbackURL: URL
) throws -> FfiEndSessionResponse {
  try prepared.endSession.parseRedirect(redirectUri: callbackURL.absoluteString)
}

/// The guide's revocation example.
func guideRevoke(
  _ state: FfiAuthenticationState,
  configuration: FfiServiceConfiguration,
  transport: URLSessionNativeHttpTransport
) async throws {
  let tokens = await state.tokenResponse()
  try await revoke(
    configuration: configuration, // requires a revocation endpoint
    clientId: "native-public-client",
    token: tokens.refreshToken ?? tokens.accessToken,
    tokenTypeHint: tokens.refreshToken != nil ? "refresh_token" : "access_token",
    transport: transport,
    cancellation: FfiCancellationHandle()
  )
}

/// The guide's dynamic-registration example.
func guideRegisterClient(
  configuration: FfiServiceConfiguration,
  transport: URLSessionNativeHttpTransport
) async throws -> FfiClientRegistrationResponse {
  try await registerPublicClient(
    configuration: configuration, // requires a registration endpoint
    options: FfiRegistrationOptions(
      redirectUris: ["https://your-app.example/oidc/callback"],
      clientName: "Example App",
      initialAccessToken: nil // only for a protected registration endpoint
    ),
    transport: transport,
    cancellation: FfiCancellationHandle()
  )
}

/// The guide's error-handling shape: `FfiError` carries the classification
/// the host branches on, and provider descriptions are untrusted diagnostics.
func guideCategorizeError(_ error: FfiError) -> String {
  switch error {
  case .InvalidRequest: return "caller configuration"
  case .InvalidResponse: return "invalid protocol response"
  case .AuthorizationServer(let code, _): return "provider rejection (\(code))"
  case .ClientRegistrationServer(let code, _): return "registration rejected (\(code))"
  case .HttpStatus(let status): return "unexpected HTTP status (\(status))"
  case .Transport: return "transport failure"
  case .TransportFailure: return "transport failure"
  case .Cancelled: return "cancelled"
  case .Clock, .Randomness: return "system failure"
  case .Internal: return "internal error"
  }
}

// MARK: - Representative runtime checks
//
// The guide's "Example verification" section records these as the
// representative state-update, cancellation, error, and authorization
// lifecycle branches (#85). They run inside the compiled macOS consumer
// binary against the release archive's binary and bindings, without a
// network connection.

private let guideStateJson =
  #"{"token_response":{"access_token":"old","token_type":"Bearer","refresh_token":"refresh"}}"#

/// A transport that returns a fixed response or throws a fixed error.
private final class GuideScriptedTransport: NativeHttpTransport, @unchecked Sendable {
  private let lock = NSLock()
  private var _result: Result<FfiHttpResponse, any Error>?
  private var _count = 0

  var result: Result<FfiHttpResponse, any Error>? {
    get {
      lock.lock()
      defer { lock.unlock() }
      return _result
    }
    set {
      lock.lock()
      _result = newValue
      lock.unlock()
    }
  }

  var requestCount: Int {
    lock.lock()
    defer { lock.unlock() }
    return _count
  }

  private func countRequest() {
    lock.lock()
    _count += 1
    lock.unlock()
  }

  func execute(
    request: FfiHttpRequest,
    cancellation: FfiCancellationHandle
  ) async throws -> FfiHttpResponse {
    countRequest()
    guard let result = self.result else {
      fatalError("guide check: no scripted response configured")
    }
    switch result {
    case .success(let response):
      return response
    case .failure(let error):
      throw error
    }
  }
}

/// A transport that suspends until the cancellation handle fires, then
/// reports `.Cancelled` like the Rust core does for a cancelled request.
private final class GuideBlockingTransport: NativeHttpTransport, @unchecked Sendable {
  private let lock = NSLock()
  private var _entered = false
  private var _requestCount = 0

  var entered: Bool {
    lock.lock()
    defer { lock.unlock() }
    return _entered
  }

  var requestCount: Int {
    lock.lock()
    defer { lock.unlock() }
    return _requestCount
  }

  func execute(
    request: FfiHttpRequest,
    cancellation: FfiCancellationHandle
  ) async throws -> FfiHttpResponse {
    lock.lock()
    _entered = true
    _requestCount += 1
    lock.unlock()
    await cancellation.cancelled()
    throw FfiError.Cancelled
  }
}

private func guideJSONResponse(_ status: UInt16, _ json: String) -> FfiHttpResponse {
  FfiHttpResponse(
    status: status,
    headers: [FfiHttpHeader(
      name: "content-type",
      value: Data("application/json".utf8)
    )],
    body: Data(json.utf8)
  )
}

/// First-wins storage for the completion of a background operation, so the
/// checks can bound their waits the way the Swift test suite does.
private final class GuideOutcome: @unchecked Sendable {
  enum Value: Equatable {
    case success(String)
    case failure(FfiError)
  }

  private let lock = NSLock()
  private var _value: Value?

  func record(_ value: Value) {
    lock.lock()
    if _value == nil { _value = value }
    lock.unlock()
  }

  var value: Value? {
    lock.lock()
    defer { lock.unlock() }
    return _value
  }
}

/// Polls `isReady` every 5 ms until it returns true or the deadline passes.
private func guideWait(
  _ seconds: Double,
  _ isReady: () -> Bool
) async -> Bool {
  let clock = ContinuousClock()
  let deadline = clock.now + .seconds(Int(seconds))
  while true {
    if isReady() { return true }
    if clock.now >= deadline { return false }
    do {
      try await Task.sleep(for: .milliseconds(5))
    } catch {
      return false
    }
  }
}

private func checkStateUpdate() async throws {
  // State update: restore a state, refresh it against a scripted token
  // endpoint that rotates the refresh token, and re-serialize it.
  let state = try FfiAuthenticationState.fromJson(serialized: guideStateJson)
  let configuration = try FfiServiceConfiguration(
    authorizationEndpoint: "https://idp.example/authorize",
    tokenEndpoint: "https://idp.example/token"
  )
  let transport = GuideScriptedTransport()
  transport.result = .success(
    guideJSONResponse(
      200,
      #"{"access_token":"new","token_type":"Bearer","expires_in":3600,"refresh_token":"rotated","scope":"profile"}"#
    )
  )

  try await state.refresh(
    configuration: configuration,
    clientId: "native-public-client",
    additionalParameters: [],
    transport: transport,
    cancellation: FfiCancellationHandle()
  )

  let tokens = await state.tokenResponse()
  guard tokens.accessToken == "new", tokens.refreshToken == "rotated" else {
    fatalError("guide check: refresh did not update the state in place")
  }
  guard transport.requestCount == 1 else {
    fatalError("guide check: expected exactly one token request")
  }
  // The updated complete state round-trips through JSON.
  let restored = try FfiAuthenticationState.fromJson(
    serialized: try await state.toJson()
  )
  let restoredTokens = await restored.tokenResponse()
  guard restoredTokens.refreshToken == "rotated" else {
    fatalError("guide check: re-serialized state lost the rotated refresh token")
  }
  print("guide check ok: state update (refresh rotation + JSON round trip)")
}

private func checkCancellation() async throws {
  // Cancellation: a handle signalled while discovery is in flight must end
  // the operation with `.Cancelled`, so the host never starts the browser
  // after a cancel.
  let transport = GuideBlockingTransport()
  let cancellation = FfiCancellationHandle()
  let outcome = GuideOutcome()

  let task = Task {
    do {
      _ = try await FfiServiceConfiguration.discover(
        issuer: "https://idp.example/",
        transport: transport,
        cancellation: cancellation
      )
      outcome.record(.success("discover"))
    } catch let error as FfiError {
      outcome.record(.failure(error))
    } catch {
      outcome.record(.failure(FfiError.Internal("unexpected error: \(error)")))
    }
  }

  guard await guideWait(10, { transport.entered }) else {
    task.cancel()
    fatalError("guide check: discovery request never entered the transport")
  }
  cancellation.cancel()

  guard await guideWait(10, { outcome.value != nil }) else {
    task.cancel()
    fatalError("guide check: discovery did not complete after cancellation")
  }
  guard case .failure(.Cancelled)? = outcome.value else {
    fatalError("guide check: expected .Cancelled, got \(String(describing: outcome.value))")
  }
  print("guide check ok: in-flight cancellation (discover -> .Cancelled)")
}

private func checkErrorBranches() async throws {
  // Error branches: provider rejection on refresh, malformed state JSON, and
  // the end-session callback validation, in both directions.
  let state = try FfiAuthenticationState.fromJson(serialized: guideStateJson)
  let configuration = try FfiServiceConfiguration(
    authorizationEndpoint: "https://idp.example/authorize",
    tokenEndpoint: "https://idp.example/token"
  )
  .withEndSessionEndpoint(endpoint: "https://idp.example/logout")

  // Provider rejection surfaces as .AuthorizationServer with the OAuth code.
  let transport = GuideScriptedTransport()
  transport.result = .success(
    guideJSONResponse(400, #"{"error":"invalid_grant","error_description":"stale"}"#)
  )
  do {
    try await state.refresh(
      configuration: configuration,
      clientId: "native-public-client",
      additionalParameters: [],
      transport: transport,
      cancellation: FfiCancellationHandle()
    )
    fatalError("guide check: refresh unexpectedly succeeded")
  } catch let error as FfiError {
    guard case .AuthorizationServer(let code, _) = error, code == "invalid_grant" else {
      fatalError("guide check: expected .AuthorizationServer(invalid_grant), got \(error)")
    }
  }
  print("guide check ok: provider rejection (.AuthorizationServer)")

  // Malformed stored JSON surfaces as .InvalidRequest.
  do {
    _ = try FfiAuthenticationState.fromJson(serialized: "{")
    fatalError("guide check: malformed state JSON unexpectedly parsed")
  } catch let error as FfiError {
    guard case .InvalidRequest = error else {
      fatalError("guide check: expected .InvalidRequest, got \(error)")
    }
  }
  print("guide check ok: malformed state JSON (.InvalidRequest)")

  // A registered post-logout redirect validates the returned state, through
  // the lifecycle entry points: prepare the session (which generates a
  // random state), derive the scripted callback from the generated URL, and
  // validate it with the retained session.
  let prepared = try await guidePrepareEndSession(state, configuration: configuration)
  let stateParameter = URLComponents(string: prepared.logoutURL.absoluteString)?
    .queryItems?
    .first { $0.name == "state" }?
    .value
  guard let stateParameter else {
    fatalError("guide check: end-session URL carries no state parameter")
  }
  let okResponse = try guideValidateEndSessionRedirect(
    prepared,
    callbackURL: URL(string: "https://your-app.example/oidc/post-logout?state=\(stateParameter)")!
  )
  guard okResponse.state == stateParameter else {
    fatalError("guide check: validated state was not echoed back")
  }
  print("guide check ok: end-session callback (state validated)")

  // A wrong state fails validation with .InvalidResponse.
  do {
    _ = try guideValidateEndSessionRedirect(
      prepared,
      callbackURL: URL(string: "https://your-app.example/oidc/post-logout?state=wrong_state")!
    )
    fatalError("guide check: mismatched state unexpectedly validated")
  } catch let error as FfiError {
    guard case .InvalidResponse = error else {
      fatalError("guide check: expected .InvalidResponse, got \(error)")
    }
  }
  print("guide check ok: end-session callback (mismatched state rejected)")
}

#if os(macOS)
/// Overlapping authorizations on the Objective-C wrapper: the second is
/// rejected before discovery, a cancel while the first discovery is in
/// flight ends the first operation without opening the browser, and the
/// single-flight slot is released once the operation ends.
@MainActor
private func checkAuthorizationLifecycle() async throws {
  let transport = GuideBlockingTransport()
  let firstOutcome = GuideOutcome()
  let secondOutcome = GuideOutcome()
  let thirdOutcome = GuideOutcome()

  let wrapper = GuideMacOSObjCService(
    adapter: MacOSAuthorizationAdapter(),
    transport: transport
  )

  // First call: suspended in discovery on the blocking transport.
  wrapper.authorize(
    issuer: "https://idp.example",
    clientId: "native-public-client",
    redirectUri: "http://127.0.0.1/oauth/callback",
    scopes: ["openid"]
  ) { stateJson, error in
    if let stateJson {
      firstOutcome.record(.success(stateJson))
    } else if let error, let ffiError = error as? FfiError {
      firstOutcome.record(.failure(ffiError))
    } else if let error {
      firstOutcome.record(.failure(FfiError.Internal("unexpected error: \(error)")))
    }
  }

  // A second call while the first is in flight must be rejected, not
  // replace the first operation's task and handle.
  wrapper.authorize(
    issuer: "https://idp.example",
    clientId: "native-public-client",
    redirectUri: "http://127.0.0.1/oauth/callback",
    scopes: ["openid"]
  ) { stateJson, error in
    if let stateJson {
      secondOutcome.record(.success(stateJson))
    } else if let error {
      secondOutcome.record(.failure(FfiError.Internal("\(error)")))
    }
  }

  // The first discovery is suspended in the transport; cancel it through
  // the wrapper.
  guard await guideWait(10, { transport.entered }) else {
    wrapper.cancel()
    fatalError("guide check: the first discovery never entered the transport")
  }
  wrapper.cancel()
  guard await guideWait(10, { firstOutcome.value != nil }) else {
    fatalError("guide check: the first authorization did not end after cancellation")
  }
  guard case .failure(.Cancelled)? = firstOutcome.value else {
    fatalError(
      "guide check: the first authorization did not end with .Cancelled, got \(String(describing: firstOutcome.value))"
    )
  }
  guard case .failure(.Internal(let message))? = secondOutcome.value,
    message.contains("AuthorizationInProgress")
  else {
    fatalError(
      "guide check: the second authorization was not rejected with AuthorizationInProgress, got \(String(describing: secondOutcome.value))"
    )
  }

  // The slot is released once the operation has ended: a third call must
  // be accepted (it enters discovery) rather than rejected.
  wrapper.authorize(
    issuer: "https://idp.example",
    clientId: "native-public-client",
    redirectUri: "http://127.0.0.1/oauth/callback",
    scopes: ["openid"]
  ) { stateJson, error in
    if let stateJson {
      thirdOutcome.record(.success(stateJson))
    } else if let error, let ffiError = error as? FfiError {
      thirdOutcome.record(.failure(ffiError))
    } else if let error {
      thirdOutcome.record(.failure(FfiError.Internal("unexpected error: \(error)")))
    }
  }
  guard await guideWait(10, { transport.requestCount >= 2 }) else {
    wrapper.cancel()
    fatalError("guide check: the third authorization never entered discovery")
  }
  wrapper.cancel()
  guard await guideWait(10, { thirdOutcome.value != nil }) else {
    fatalError("guide check: the third authorization did not end after cancellation")
  }
  guard transport.requestCount == 2 else {
    fatalError(
      "guide check: expected two discovery requests, got \(transport.requestCount)"
    )
  }
  print(
    "guide check ok: overlapping authorization (second rejected, first cancelled during discovery, slot released)"
  )
}
#endif

/// Runs the representative guide-example branches; the release consumer
/// binary calls this after the archive sanity checks.
func runGuideExampleChecks() async {
  do {
    try await checkStateUpdate()
    try await checkCancellation()
    try await checkErrorBranches()
    #if os(macOS)
      try await checkAuthorizationLifecycle()
    #endif
  } catch {
    fatalError("guide example check failed: \(error)")
  }
}

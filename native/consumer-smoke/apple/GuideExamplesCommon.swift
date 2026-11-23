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
  // The redirect must be the one for the platform the adapter runs on: the
  // macOS adapter requires the loopback template and rejects anything else,
  // while iOS/iPadOS uses the HTTPS associated domain.
  #if os(macOS)
    let redirectUri = "http://127.0.0.1/oauth/callback"
  #else
    let redirectUri = "https://your-app.example/oidc/callback"
  #endif
  return FfiAuthorizationOptions(
    clientId: "native-public-client",
    // Must exactly match the redirect registered with the provider.
    redirectUri: redirectUri,
    scopes: ["openid", "profile"],
    // Optional pass-through for the authorization request and token exchange.
    additionalParameters: [FfiParameter(name: "prompt", value: "consent")],
    additionalTokenParameters: []
  )
}

/// The guide's Keychain persistence sketch.
enum GuideStateStorageError: Error {
  case update(OSStatus)
  case add(OSStatus)
  case delete(OSStatus)
  case read(OSStatus)
  case missing
}

func guideSaveState(_ json: String, account: String) throws {
  let query: [CFString: Any] = [
    kSecClass: kSecClassGenericPassword,
    kSecAttrService: "com.example.app",
    kSecAttrAccount: account,
  ]

  // Update the existing item in place, and only add when it does not exist:
  // deleting first would lose the previous session if the write fails
  // partway through. The update dictionary carries only the value being
  // replaced — `SecItemUpdate` copies the attributes it is given, so the
  // identifying attributes stay out of it.
  let update: [CFString: Any] = [kSecValueData: Data(json.utf8)]
  switch SecItemUpdate(query as CFDictionary, update as CFDictionary) {
  case errSecSuccess:
    return
  case errSecItemNotFound:
    break // No item yet: fall through to add.
  case let status:
    throw GuideStateStorageError.update(status)
  }
  var attributes = query
  attributes[kSecValueData] = Data(json.utf8)
  let status = SecItemAdd(attributes as CFDictionary, nil)
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
    // Distinguish an absent record (first launch, or after sign-out) from a
    // storage failure: only errSecItemNotFound means there is no state to
    // restore.
    if status == errSecItemNotFound {
      throw GuideStateStorageError.missing
    }
    throw GuideStateStorageError.read(status)
  }
  return String(decoding: data, as: UTF8.self)
}

/// The guide's local sign-out deletion of the stored state.
func guideDeleteState(account: String) throws {
  let query: [CFString: Any] = [
    kSecClass: kSecClassGenericPassword,
    kSecAttrService: "com.example.app",
    kSecAttrAccount: account,
  ]
  // Accept only the two expected outcomes — the record deleted, or already
  // gone — and surface any other status: a failed deletion must not be
  // reported as a completed sign-out.
  switch SecItemDelete(query as CFDictionary) {
  case errSecSuccess, errSecItemNotFound:
    return
  case let status:
    throw GuideStateStorageError.delete(status)
  }
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

/// The session guard the guide's refresh write and local sign-out share:
/// one lock and one flag, so the two cannot drift apart.
final class GuideSessionGuard {
  private let lock = NSLock()
  private var signedOut = false

  /// A synchronous critical section that releases the lock even when
  /// `body` throws, so a failed Keychain write cannot deadlock sign-out.
  func withLock<T>(_ body: () throws -> T) rethrows -> T {
    lock.lock()
    defer { lock.unlock() }
    return try body()
  }

  var isSignedOut: Bool {
    withLock { signedOut }
  }

  // Internal seam: the guide's guard writes through the Keychain sketch; the
  // checks substitute a scripted write while keeping the same flag check and
  // lock path.
  var save: ((_ stateJson: String, _ account: String) throws -> Void)?

  /// Persists the state unless the session has been signed out: the flag
  /// check and the write run under one lock hold.
  func persistState(_ stateJson: String, account: String) throws {
    try withLock {
      guard !signedOut else { return }
      if let save {
        try save(stateJson, account)
      } else {
        try guideSaveState(stateJson, account: account)
      }
    }
  }

  /// Local sign-out sets the flag under the same lock, so a suspended
  /// refresh write cannot write the deleted state back.
  func markSignedOut() {
    withLock { signedOut = true }
  }
}

/// The guide's local sign-out: mark the session signed out in the shared
/// guard, then clear the stored state, checking the deletion status.
func guideSignOut(session: GuideSessionGuard) throws {
  session.markSignedOut()
  try guideDeleteState(account: "native-oidc/state")
}

/// The guide's refresh example, including re-persistence of the updated
/// complete state through the session guard shared with the guide's local
/// sign-out.
///
/// Internal seam (the guide's snippet uses `URLSessionNativeHttpTransport`):
/// the checks substitute a scripted transport while exercising the same
/// refresh-and-guard path.
func guideRefresh(
  _ state: FfiAuthenticationState,
  configuration: FfiServiceConfiguration,
  transport: any NativeHttpTransport,
  session: GuideSessionGuard
) async throws {
  try await state.refresh(
    configuration: configuration,
    clientId: "native-public-client",
    additionalParameters: [],
    transport: transport,
    cancellation: FfiCancellationHandle()
  )

  // Refresh tokens may rotate; persist the updated complete state. Serialize
  // the state outside the lock — `toJson` is a core request, and the lock
  // must not be held across it.
  let stateJson = try await state.toJson()
  try session.persistState(stateJson, account: "native-oidc/state")
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
      // The platform redirects from the authorization-options example above:
      // the HTTPS associated domain for iOS/iPadOS and the loopback template
      // for macOS. Register the ones the client supports.
      redirectUris: [
        "https://your-app.example/oidc/callback",
        "http://127.0.0.1/oauth/callback",
      ],
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

/// A transport that returns queued responses in request order or throws a
/// queued error, and can suspend a request until the check releases it.
private final class GuideScriptedTransport: NativeHttpTransport, @unchecked Sendable {
  private let lock = NSLock()
  private var _responses: [Result<FfiHttpResponse, any Error>] = []
  private var _holdNext = false
  private var _held: CheckedContinuation<Void, Never>?
  private var _count = 0

  /// The responses `execute` returns, in request order.
  var responses: [Result<FfiHttpResponse, any Error>] {
    get {
      lock.lock()
      defer { lock.unlock() }
      return _responses
    }
    set {
      lock.lock()
      _responses = newValue
      lock.unlock()
    }
  }

  var requestCount: Int {
    lock.lock()
    defer { lock.unlock() }
    return _count
  }

  /// The check installs an outcome here; `execute` records a success once a
  /// held request is suspended and its continuation installed, so the check
  /// can interpose work without racing the suspension.
  var holdSignal: GuideOutcome?

  /// Suspends the next request it receives until `releaseHeldRequest()`
  /// runs, so the check can interleave other work with an in-flight request.
  func holdNextRequest() {
    lock.lock()
    defer { lock.unlock() }
    _holdNext = true
  }

  /// Releases a request suspended by `holdNextRequest()`.
  func releaseHeldRequest() {
    lock.lock()
    let held = _held
    _held = nil
    lock.unlock()
    held?.resume()
  }

  private func countRequest() {
    lock.lock()
    _count += 1
    lock.unlock()
  }

  private func takeNextResponse() -> (hold: Bool, response: Result<FfiHttpResponse, any Error>?) {
    lock.lock()
    defer { lock.unlock() }
    let hold = _holdNext
    if hold {
      _holdNext = false
    }
    let response = _responses.first
    if !_responses.isEmpty {
      _responses.removeFirst()
    }
    return (hold, response)
  }

  func execute(
    request: FfiHttpRequest,
    cancellation: FfiCancellationHandle
  ) async throws -> FfiHttpResponse {
    countRequest()
    let (hold, result) = takeNextResponse()
    if hold {
      await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
        lock.lock()
        _held = continuation
        lock.unlock()
        holdSignal?.record(.success("held"))
      }
    }

    guard let result else {
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

  private func noteEntry() {
    lock.lock()
    _entered = true
    _requestCount += 1
    lock.unlock()
  }

  func execute(
    request: FfiHttpRequest,
    cancellation: FfiCancellationHandle
  ) async throws -> FfiHttpResponse {
    noteEntry()
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
  transport.responses = [
    .success(
      guideJSONResponse(
        200,
        #"{"access_token":"new","token_type":"Bearer","expires_in":3600,"refresh_token":"rotated","scope":"profile"}"#
      )
    )
  ]

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
  transport.responses = [
    .success(guideJSONResponse(400, #"{"error":"invalid_grant","error_description":"stale"}"#))
  ]
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

private func checkRefreshSignOutInterleaving() async throws {
  // The guide's Keychain sketch, refresh write, and local sign-out share one
  // session guard: sign-out invalidates a pending refresh write, and a
  // failing write releases the guard's lock.
  let state = try FfiAuthenticationState.fromJson(serialized: guideStateJson)
  let configuration = try FfiServiceConfiguration(
    authorizationEndpoint: "https://idp.example/authorize",
    tokenEndpoint: "https://idp.example/token"
  )
  let transport = GuideScriptedTransport()
  transport.responses = [
    .success(
      guideJSONResponse(
        200,
        #"{"access_token":"new","token_type":"Bearer","refresh_token":"rotated"}"#
      )
    ),
    .success(
      guideJSONResponse(
        200,
        #"{"access_token":"new","token_type":"Bearer","refresh_token":"rotated"}"#
      )
    ),
  ]
  let session = GuideSessionGuard()
  try guideDeleteState(account: "native-oidc/state")

  // Save twice under one account and confirm the second value is restored:
  // the first save creates the record, the second must replace the existing
  // record in place through `SecItemUpdate`.
  try guideSaveState("first-save", account: "native-oidc/state")
  try guideSaveState("second-save", account: "native-oidc/state")
  guard (try? guideLoadState(account: "native-oidc/state")) == "second-save" else {
    fatalError("guide check: a second save did not replace the existing Keychain record")
  }
  print("guide check ok: Keychain save (second save replaces the existing record)")

  // Refresh succeeds and persists the updated state through the guard.
  try await guideRefresh(
    state,
    configuration: configuration,
    transport: transport,
    session: session
  )
  _ = try guideLoadState(account: "native-oidc/state")
  guard !session.isSignedOut else {
    fatalError("guide check: the session was signed out before sign-out ran")
  }

  // Sign-out: the flag and the checked deletion, through the same entry
  // points.
  try guideSignOut(session: session)
  guard session.isSignedOut else {
    fatalError("guide check: sign-out did not set the flag")
  }
  do {
    _ = try guideLoadState(account: "native-oidc/state")
    fatalError("guide check: sign-out left the stored state in place")
  } catch let error as GuideStateStorageError {
    guard case .missing = error else {
      fatalError("guide check: unexpected load error: \(error)")
    }
  }

  // A refresh that outlives sign-out must not write the deleted state back.
  // This runs on a fresh, active session: hold a real refresh before its
  // persistence, sign the session out while the refresh is suspended, and
  // only then release the refresh through the same persistState path — the
  // suppressed write must come from this single sign-out. The check waits
  // for the transport's held signal, which is published after the
  // continuation is installed, so the release cannot be lost.
  let heldSession = GuideSessionGuard()
  try guideDeleteState(account: "native-oidc/state")
  let heldSignal = GuideOutcome()
  transport.holdSignal = heldSignal
  transport.holdNextRequest()
  let heldRefreshDone = GuideOutcome()
  let heldRefresh = Task {
    try await guideRefresh(
      state,
      configuration: configuration,
      transport: transport,
      session: heldSession
    )
    heldRefreshDone.record(.success("released"))
  }
  guard await guideWait(10, { heldSignal.value != nil }) else {
    transport.releaseHeldRequest()
    heldRefresh.cancel()
    fatalError("guide check: the held refresh never suspended before its persistence")
  }
  try guideSignOut(session: heldSession)
  transport.releaseHeldRequest()
  guard await guideWait(30, { heldRefreshDone.value != nil }) else {
    heldRefresh.cancel()
    fatalError("guide check: the released refresh did not finish after sign-out")
  }
  do {
    try await heldRefresh.value
  } catch {
    fatalError("guide check: the released refresh threw: \(error)")
  }
  do {
    _ = try guideLoadState(account: "native-oidc/state")
    fatalError("guide check: a post-sign-out refresh wrote the state back")
  } catch let error as GuideStateStorageError {
    guard case .missing = error else {
      fatalError("guide check: unexpected load error: \(error)")
    }
  }
  print("guide check ok: refresh/sign-out interleaving (post-sign-out write suppressed)")

  // A failing write through persistState must release the guard's lock, so a
  // sign-out through its normal entry point completes instead of blocking on
  // the failed write. This runs on a fresh session that has not signed out,
  // so the write is attempted (and fails) before the sign-out.
  let freshSession = GuideSessionGuard()
  freshSession.save = { _, _ in throw GuideStateStorageError.add(errSecAuthFailed) }
  do {
    try freshSession.persistState("failing-save", account: "native-oidc/state")
    fatalError("guide check: the failing write unexpectedly succeeded")
  } catch {}
  let signOutDone = GuideOutcome()
  let signOutTask = Task {
    try guideSignOut(session: freshSession)
    signOutDone.record(.success("signed out"))
  }
  guard await guideWait(10, { signOutDone.value != nil }) else {
    signOutTask.cancel()
    fatalError("guide check: sign-out did not complete after a failing write")
  }
  print("guide check ok: failing write through persistState left sign-out free to complete")
}

#if os(macOS)
/// Overlapping authorizations on the Objective-C wrapper: the second is
/// rejected before discovery; a cancel while the first discovery is in
/// flight ends the first operation without opening the browser, and the
/// single-flight slot stays claimed until the operation finishes — an
/// immediate retry after `cancel()` is rejected, a retry issued from the
/// completion is accepted, and the slot is released once the operation
/// ends.
@MainActor
private func checkAuthorizationLifecycle() async throws {
  let transport = GuideBlockingTransport()
  let firstOutcome = GuideOutcome()
  let secondOutcome = GuideOutcome()
  let heldRetryOutcome = GuideOutcome()
  let completionRetryOutcome = GuideOutcome()
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
    // A retry issued directly from the completion must be accepted: the
    // wrapper releases the slot before calling the completion handler. The
    // completion is invoked on the main actor, so retry synchronously
    // through `MainActor.assumeIsolated` — a `Task { @MainActor in }` hop
    // would yield the main-actor job first, and the check would pass even if
    // the slot were released only after the completion call.
    MainActor.assumeIsolated {
      wrapper.authorize(
        issuer: "https://idp.example",
        clientId: "native-public-client",
        redirectUri: "http://127.0.0.1/oauth/callback",
        scopes: ["openid"]
      ) { stateJson, error in
        if let stateJson {
          completionRetryOutcome.record(.success(stateJson))
        } else if let error, let ffiError = error as? FfiError {
          completionRetryOutcome.record(.failure(ffiError))
        } else if let error {
          completionRetryOutcome.record(.failure(FfiError.Internal("unexpected error: \(error)")))
        }
      }
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

  // An immediate retry while the cancelled operation is still unwinding
  // must be rejected: the slot stays claimed during cancellation, so the
  // new operation cannot race the still-busy adapter.
  wrapper.authorize(
    issuer: "https://idp.example",
    clientId: "native-public-client",
    redirectUri: "http://127.0.0.1/oauth/callback",
    scopes: ["openid"]
  ) { stateJson, error in
    if let stateJson {
      heldRetryOutcome.record(.success(stateJson))
    } else if let error {
      heldRetryOutcome.record(.failure(FfiError.Internal("\(error)")))
    }
  }
  guard case .failure(.Internal(let message))? = heldRetryOutcome.value,
    message.contains("AuthorizationInProgress")
  else {
    wrapper.cancel()
    fatalError(
      "guide check: a retry right after cancel() was not rejected while the slot stays claimed, got \(String(describing: heldRetryOutcome.value))"
    )
  }

  // The cancelled operation ends with .Cancelled and releases the slot.
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

  // The retry issued from the completion must have been accepted and must
  // be suspended in its own discovery now: a rejected retry would never
  // enter the transport, so a second discovery request proves acceptance.
  guard await guideWait(10, { transport.requestCount >= 2 }) else {
    wrapper.cancel()
    fatalError(
      "guide check: the retry from the completion never entered discovery (rejected?), got \(String(describing: completionRetryOutcome.value))"
    )
  }
  wrapper.cancel()
  guard await guideWait(10, { completionRetryOutcome.value != nil }) else {
    fatalError("guide check: the completion retry did not end after cancellation")
  }
  guard case .failure(.Cancelled)? = completionRetryOutcome.value else {
    fatalError(
      "guide check: the completion retry did not end with .Cancelled, got \(String(describing: completionRetryOutcome.value))"
    )
  }

  // The slot is released once the operation has ended: an external call
  // must be accepted (it enters discovery) rather than rejected.
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
  guard await guideWait(10, { transport.requestCount >= 3 }) else {
    wrapper.cancel()
    fatalError("guide check: the third authorization never entered discovery")
  }
  wrapper.cancel()
  guard await guideWait(10, { thirdOutcome.value != nil }) else {
    fatalError("guide check: the third authorization did not end after cancellation")
  }
  guard transport.requestCount == 3 else {
    fatalError(
      "guide check: expected three discovery requests, got \(transport.requestCount)"
    )
  }
  print(
    "guide check ok: authorization lifecycle (overlap and post-cancel retries rejected, completion retry accepted, slot released when the operation ends)"
  )

  // A scripted success flow: discovery, the loopback callback, and the code
  // exchange all complete, and the cancellation lands as the task enters the
  // final serialization — after the adapter has returned, before the success
  // report. The generated `toJson()` has no task-cancellation handler, so
  // the operation still reports the serialized state.
  let scriptedTransport = GuideScriptedTransport()
  scriptedTransport.responses = [
    .success(
      guideJSONResponse(
        200,
        #"{"issuer":"https://idp.example/","authorization_endpoint":"https://idp.example/authorize","token_endpoint":"https://idp.example/token"}"#
      )
    ),
    .success(
      guideJSONResponse(
        200,
        #"{"access_token":"new","token_type":"Bearer","refresh_token":"rotated"}"#
      )
    ),
  ]
  let successWrapper = GuideMacOSObjCService(
    adapter: MacOSAuthorizationAdapter { authorizationURL in
      // Simulates the external browser: the authorization URL it would open
      // names the loopback callback and the generated state; visit the
      // callback with them, as the provider's redirect would.
      let components = URLComponents(url: authorizationURL, resolvingAgainstBaseURL: false)
      let queryItems = components?.queryItems ?? []
      guard let redirect = queryItems.first(where: { $0.name == "redirect_uri" })?.value,
        let stateParameter = queryItems.first(where: { $0.name == "state" })?.value,
        var callback = URLComponents(string: redirect)
      else {
        throw URLError(.badURL)
      }
      callback.queryItems = [
        URLQueryItem(name: "code", value: "scripted-authorization-code"),
        URLQueryItem(name: "state", value: stateParameter),
      ]
      let (_, response) = try await URLSession.shared.data(from: callback.url!)
      guard (response as? HTTPURLResponse)?.statusCode == 200 else {
        throw URLError(.badServerResponse)
      }
    },
    transport: scriptedTransport
  )
  let successOutcome = GuideOutcome()
  successWrapper.onStateSerialized = {
    // The cancellation lands as the task enters the final serialization.
    successWrapper.cancel()
  }
  successWrapper.authorize(
    issuer: "https://idp.example/",
    clientId: "native-public-client",
    redirectUri: "http://127.0.0.1/oauth/callback",
    scopes: ["profile"]
  ) { stateJson, error in
    if let stateJson {
      successOutcome.record(.success(stateJson))
    } else if let error, let ffiError = error as? FfiError {
      successOutcome.record(.failure(ffiError))
    } else if let error {
      successOutcome.record(.failure(FfiError.Internal("unexpected error: \(error)")))
    }
  }
  guard await guideWait(30, { successOutcome.value != nil }) else {
    successWrapper.cancel()
    fatalError("guide check: the scripted authorization flow did not complete")
  }
  guard case .success(let stateJson)? = successOutcome.value else {
    fatalError(
      "guide check: a cancellation during serialization should still deliver the state, got \(String(describing: successOutcome.value))"
    )
  }
  // The delivered JSON is the exchanged state.
  let delivered = try FfiAuthenticationState.fromJson(serialized: stateJson)
  let deliveredTokens = await delivered.tokenResponse()
  guard deliveredTokens.accessToken == "new" else {
    fatalError("guide check: the delivered state was not the exchanged state")
  }
  print(
    "guide check ok: cancellation during serialization still reports the state (toJson has no task-cancellation handler)"
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
    try await checkRefreshSignOutInterleaving()
    #if os(macOS)
      try await checkAuthorizationLifecycle()
    #endif
  } catch {
    fatalError("guide example check failed: \(error)")
  }
}

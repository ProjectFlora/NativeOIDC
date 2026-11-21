#if os(macOS)
  import Darwin
  import Foundation
  @testable import NativeOIDC
  import XCTest

  /// Loopback listener, transport, and adapter checks that need neither a
  /// browser nor a simulator. The complete authorization flow still runs in
  /// `native/apple/MacOSAuthorizationFlowSmoke.swift`.
  final class MacOSAuthorizationAdapterTests: XCTestCase {
    private var callbackSession: URLSession!

    override func setUp() {
      super.setUp()
      let configuration = URLSessionConfiguration.ephemeral
      configuration.timeoutIntervalForRequest = 2
      configuration.timeoutIntervalForResource = 3
      callbackSession = URLSession(configuration: configuration)
    }

    override func tearDown() {
      callbackSession.invalidateAndCancel()
      callbackSession = nil
      super.tearDown()
    }

    // MARK: - Loopback listener

    func testListenerDeliversCallbackDespiteStalledConnectionAndWrongPath() async throws {
      let listener = try LoopbackHTTPListener(callbackPath: "/oauth/callback")
      let port = try await listener.start()
      let callbackTask = Task {
        try await listener.waitForCallback()
      }

      let stalled = try Self.openSocket(port: port, request: "GET /oauth/callback HTTP/1.1\r\n")
      defer { Darwin.close(stalled) }

      let wrongPath = URL(string: "http://127.0.0.1:\(port)/favicon.ico")!
      let (_, wrongResponse) = try await callbackSession.data(from: wrongPath)
      XCTAssertEqual((wrongResponse as? HTTPURLResponse)?.statusCode, 404)

      let callbackURL = URL(
        string: "http://127.0.0.1:\(port)/oauth/callback?code=auth-code&state=state"
      )!
      let (_, callbackResponse) = try await callbackSession.data(from: callbackURL)
      XCTAssertEqual((callbackResponse as? HTTPURLResponse)?.statusCode, 200)

      let receivedComponents = URLComponents(string: try await callbackTask.value)
      XCTAssertEqual(receivedComponents?.path, "/oauth/callback")
      XCTAssertEqual(
        receivedComponents?.queryItems?.first { $0.name == "state" }?.value, "state")
    }

    func testCompletedListenerRefusesAnotherCallback() async throws {
      let listener = try LoopbackHTTPListener(callbackPath: "/oauth/callback")
      let port = try await listener.start()
      let callbackURL = URL(string: "http://127.0.0.1:\(port)/oauth/callback?code=c&state=s")!
      _ = try await callbackSession.data(from: callbackURL)
      _ = try await listener.waitForCallback()

      do {
        _ = try await callbackSession.data(from: callbackURL)
        XCTFail("completed listener accepted another callback")
      } catch is URLError {
      }
    }

    func testIndependentListenersBindDifferentPortsAndCancelSeparately() async throws {
      let first = try LoopbackHTTPListener(callbackPath: "/oauth/callback")
      let second = try LoopbackHTTPListener(callbackPath: "/oauth/callback")
      let firstPort = try await first.start()
      let secondPort = try await second.start()
      XCTAssertNotEqual(firstPort, secondPort)

      // Cancelling one listener while the other is waiting leaves the other
      // able to deliver its callback.
      let secondCallbackTask = Task {
        try await second.waitForCallback()
      }
      // Creating the task does not run it, so wait until the second listener
      // has registered its pending wait before cancelling the first.
      let deadline = Date().addingTimeInterval(5)
      while !(await second.isWaitingForCallback) {
        guard Date() < deadline else {
          await second.cancel()
          XCTFail("second listener did not start waiting for its callback")
          return
        }
        try await Task.sleep(nanoseconds: 10_000_000)
      }
      await first.cancel()
      let callbackURL = URL(string: "http://127.0.0.1:\(secondPort)/oauth/callback?code=c&state=s")!
      let (_, response) = try await callbackSession.data(from: callbackURL)
      XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
      let received = try await secondCallbackTask.value
      XCTAssertEqual(URLComponents(string: received)?.port, Int(secondPort))
    }

    func testCancelledListenerRefusesToStart() async throws {
      let listener = try LoopbackHTTPListener(callbackPath: "/oauth/callback")
      await listener.cancel()
      do {
        _ = try await listener.start()
        XCTFail("cancelled listener started")
      } catch FfiError.Cancelled {
      }
    }

    func testCancellingListenerFailsPendingCallbackWait() async throws {
      let listener = try LoopbackHTTPListener(callbackPath: "/oauth/callback")
      _ = try await listener.start()
      let callbackTask = Task {
        try await listener.waitForCallback()
      }
      await listener.cancel()
      do {
        _ = try await callbackTask.value
        XCTFail("cancelled listener returned a callback")
      } catch FfiError.Cancelled {
      }
    }

    // MARK: - URLSessionNativeHttpTransport

    func testTransportDoesNotFollowRedirects() async throws {
      try await withProvider { provider, base in
        let response = try await URLSessionNativeHttpTransport().execute(
          request: FfiHttpRequest(
            method: "GET", url: "\(base)/transport-redirect", headers: [], body: Data()),
          cancellation: FfiCancellationHandle())
        XCTAssertEqual(response.status, 302)
        let counts = await provider.transportRequestCounts()
        XCTAssertEqual(counts.redirectTargets, 0)
      }
    }

    func testTransportBypassesSuppliedCacheWithoutChangingConfiguration() async throws {
      try await withProvider { provider, base in
        let supplied = URLSessionConfiguration.ephemeral
        let cache = URLCache(memoryCapacity: 1024 * 1024, diskCapacity: 0)
        supplied.urlCache = cache
        supplied.requestCachePolicy = .returnCacheDataElseLoad
        let transport = URLSessionNativeHttpTransport(configuration: supplied)
        XCTAssertTrue(supplied.urlCache === cache)

        let url = URL(string: "\(base)/transport-cache")!
        let cached = HTTPURLResponse(
          url: url, statusCode: 200, httpVersion: "HTTP/1.1",
          headerFields: ["Cache-Control": "max-age=300"])!
        cache.storeCachedResponse(
          CachedURLResponse(response: cached, data: Data("stale".utf8)),
          for: URLRequest(url: url))
        for version in 1...2 {
          let response = try await transport.execute(
            request: FfiHttpRequest(
              method: "GET", url: url.absoluteString, headers: [], body: Data()),
            cancellation: FfiCancellationHandle())
          XCTAssertEqual(String(data: response.body, encoding: .utf8), "version-\(version)")
        }
        let counts = await provider.transportRequestCounts()
        XCTAssertEqual(counts.cacheRequests, 2)
      }
    }

    func testTransportRejectsCancelledRequest() async throws {
      try await withProvider { _, base in
        let cancelled = FfiCancellationHandle()
        cancelled.cancel()
        do {
          _ = try await URLSessionNativeHttpTransport().execute(
            request: FfiHttpRequest(
              method: "GET", url: "\(base)/transport-redirect", headers: [], body: Data()),
            cancellation: cancelled)
          XCTFail("cancelled transport request succeeded")
        } catch FfiError.Cancelled {
        }
      }
    }

    // MARK: - MacOSAuthorizationAdapter

    @MainActor
    func testStateMismatchIsRejectedAndAdapterCanBeReused() async throws {
      try await withProvider { _, base in
        let adapter = MacOSAuthorizationAdapter { url in
          var callback = URLComponents(string: try Self.redirectURI(in: url))!
          callback.queryItems = [
            URLQueryItem(name: "code", value: "code"),
            URLQueryItem(name: "state", value: "another-operation"),
          ]
          let (_, response) = try await URLSession.shared.data(from: callback.url!)
          XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        }
        for attempt in 1...2 {
          do {
            _ = try await adapter.authorize(
              configuration: try Self.configuration(base: base), options: Self.options,
              transport: URLSessionNativeHttpTransport())
            XCTFail("attempt \(attempt): state mismatch accepted")
          } catch FfiError.InvalidResponse {
          }
        }
      }
    }

    @MainActor
    func testBrowserOpenFailureReturnsErrorAndReleasesListener() async throws {
      let opened = OpenedRedirects()
      let adapter = MacOSAuthorizationAdapter { url in
        opened.uris.append(try Self.redirectURI(in: url))
        throw MacOSAuthorizationAdapterError.browserCouldNotBeOpened
      }
      let configuration = try FfiServiceConfiguration(
        authorizationEndpoint: "https://idp.example/authorize",
        tokenEndpoint: "https://idp.example/token"
      )

      // The second attempt proves the adapter cleared its running state.
      for attempt in 1...2 {
        do {
          _ = try await adapter.authorize(
            configuration: configuration, options: Self.options,
            transport: URLSessionNativeHttpTransport())
          XCTFail("attempt \(attempt): authorization succeeded without a browser")
        } catch MacOSAuthorizationAdapterError.browserCouldNotBeOpened {
        }
      }

      XCTAssertEqual(opened.uris.count, 2)
      for redirectURI in opened.uris {
        let port = try XCTUnwrap(URLComponents(string: redirectURI)?.port)
        try await Self.assertPortReleased(UInt16(port))
      }
    }

    // MARK: - Helpers

    @MainActor
    private final class OpenedRedirects {
      var uris: [String] = []
    }

    private static let options = FfiAuthorizationOptions(
      clientId: "macos-smoke-client",
      redirectUri: "http://127.0.0.1/oauth/callback",
      scopes: ["profile"],
      additionalParameters: [],
      additionalTokenParameters: []
    )

    private static func configuration(base: String) throws -> FfiServiceConfiguration {
      try FfiServiceConfiguration(
        authorizationEndpoint: "\(base)/authorize",
        tokenEndpoint: "\(base)/token"
      )
    }

    private func withProvider(
      _ body: (MacOSMockOAuthProvider, String) async throws -> Void
    ) async throws {
      let provider = try MacOSMockOAuthProvider()
      let port = try await provider.start()
      do {
        try await body(provider, "http://127.0.0.1:\(port)")
      } catch {
        await provider.stop()
        throw error
      }
      await provider.stop()
    }

    private static func redirectURI(in authorizationURL: URL) throws -> String {
      let components = URLComponents(url: authorizationURL, resolvingAgainstBaseURL: false)
      return try XCTUnwrap(components?.queryItems?.first { $0.name == "redirect_uri" }?.value)
    }

    /// Waits until nothing accepts connections on `port`. The listener is
    /// cancelled before `authorize` returns, but the socket closes on the
    /// listener's queue shortly after.
    private static func assertPortReleased(
      _ port: UInt16, file: StaticString = #filePath, line: UInt = #line
    ) async throws {
      let deadline = Date().addingTimeInterval(5)
      while Date() < deadline {
        guard let socket = try? openSocket(port: port, request: "") else {
          return
        }
        Darwin.close(socket)
        try await Task.sleep(nanoseconds: 50_000_000)
      }
      XCTFail("loopback listener on port \(port) still accepts connections", file: file, line: line)
    }

    private static func openSocket(port: UInt16, request: String) throws -> Int32 {
      let socket = Darwin.socket(AF_INET, SOCK_STREAM, 0)
      guard socket >= 0 else { throw URLError(.cannotConnectToHost) }
      var address = sockaddr_in()
      address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
      address.sin_family = sa_family_t(AF_INET)
      address.sin_port = port.bigEndian
      address.sin_addr.s_addr = inet_addr("127.0.0.1")
      let connected = withUnsafePointer(to: &address) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
          Darwin.connect(socket, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
      }
      guard connected == 0 else {
        Darwin.close(socket)
        throw URLError(.cannotConnectToHost)
      }
      let bytes = Array(request.utf8)
      let sent = bytes.withUnsafeBytes { Darwin.send(socket, $0.baseAddress, $0.count, 0) }
      guard sent == bytes.count else {
        Darwin.close(socket)
        throw URLError(.networkConnectionLost)
      }
      return socket
    }
  }
#endif

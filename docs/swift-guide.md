# Swift Guide

This guide covers the public API usage for iOS/iPadOS and macOS applications.

## Supported platforms

- iOS/iPadOS 18+
- macOS 15+
- Xcode 16+
- Swift 6.4+

The library is delivered as a static `NativeOIDC.xcframework` or via Swift Package Manager (v0.5.0+).
See [Swift XCFramework installation](swift-xcframework.md) for manual integration details.

## Objective-C interoperability

Objective-C applications have no library-provided entry point. UniFFI generates Swift bindings only,
so hosts must provide a wrapper:

- **Wrap the Swift API in an `@objc` class** with completion-handler methods that run the async work
  in a Swift task. The wrapper is where main-thread hops and lifecycle handling belong.

Example pattern (conceptual):

```swift
@objc class NativeOIDCWrapper: NSObject {
    private let authorizationAdapter = IOSAuthorizationAdapter() // or MacOSAuthorizationAdapter
    
    @objc func authorizeWithCompletion(state: String, callbackURLScheme: String?, 
                                      completion: @escaping (Error?) -> Void) {
        Task { @MainActor in
            do {
                let result = try await authorizationAdapter.authorize(
                    state: state,
                    callbackURLScheme: callbackURLScheme
                )
                // Handle success...
                completion(nil)
            } catch {
                completion(error as NSError?)
            }
        }
    }
    
    @objc func cancel() {
        authorizationAdapter.cancel()
    }
}
```

This pattern allows legacy Objective-C code to use the library's async API with traditional
completion handlers. The wrapper is where timeout policy, cancellations, and lifecycle
events are managed.

## Configuration

See [user-guide.md](user-guide.md) for configuration, transport setup, authorization flow,
state management, and refresh operations.

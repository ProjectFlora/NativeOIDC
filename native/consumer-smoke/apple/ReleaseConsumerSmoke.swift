import Foundation

@main
struct ReleaseConsumerSmoke {
  static func main() async throws {
    let configuration = try FfiServiceConfiguration(
      authorizationEndpoint: "https://issuer.example/authorize",
      tokenEndpoint: "https://issuer.example/token"
    )
    precondition(configuration.authorizationEndpoint() == "https://issuer.example/authorize")
    precondition(configuration.tokenEndpoint() == "https://issuer.example/token")

    // Representative guide-example branches against the release archive:
    // state update (refresh rotation + JSON round trip), in-flight
    // cancellation, and the error branches.
    await runGuideExampleChecks()
  }
}

#!/usr/bin/env bash
set -euo pipefail

# Build and run the macOS authorization-flow smoke test against the staged
# static macOS library, so the package's headers, module map and universal
# library link without the Swift package. scripts/package-apple-xcframework.sh
# runs it after staging; run it on its own against an existing staging in
# target/apple/release-package.

repo_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

if [[ "$(uname -s)" != Darwin ]]; then
    echo "The macOS static-link smoke test requires macOS and Xcode" >&2
    exit 1
fi

case "$(uname -m)" in
    arm64) host_macos_target=arm64-apple-macosx15.0 ;;
    x86_64) host_macos_target=x86_64-apple-macosx15.0 ;;
    *)
        echo "unsupported macOS architecture for package smoke test" >&2
        exit 1
        ;;
esac

generated="$repo_root/target/uniffi"
headers="$repo_root/target/apple/release-package/headers"
macos_library="$repo_root/target/apple/libnative_oidc_macos.a"

for input in "$headers/module.modulemap" "$macos_library" "$generated/swift/native_oidc.swift"; do
    if [[ ! -f "$input" ]]; then
        echo "Staged input is missing: $input (run scripts/package-apple-xcframework.sh first)" >&2
        exit 1
    fi
done

xcrun swiftc \
    -parse-as-library \
    -target "$host_macos_target" \
    -sdk "$(xcrun --sdk macosx --show-sdk-path)" \
    -Xcc "-fmodule-map-file=$headers/module.modulemap" \
    -L "$repo_root/target/apple" \
    -lnative_oidc_macos \
    -o "$repo_root/target/apple/mac-authorization-flow-smoke-static" \
    "$generated/swift/native_oidc.swift" \
    "$repo_root/SwiftTests/NativeOIDCTests/MacOSMockOAuthProvider.swift" \
    "$repo_root/Sources/NativeOIDC/MacOSAuthorizationAdapter.swift" \
    "$repo_root/Sources/NativeOIDC/URLSessionNativeHttpTransport.swift" \
    "$repo_root/native/apple/MacOSAuthorizationFlowSmoke.swift"
"$repo_root/target/apple/mac-authorization-flow-smoke-static"

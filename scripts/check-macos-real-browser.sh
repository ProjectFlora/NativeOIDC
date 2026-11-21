#!/usr/bin/env bash
# Runs one authorization through the real macOS system browser: the default
# MacOSAuthorizationAdapter opens the authorization URL with NSWorkspace,
# scripts/real_browser_mock_provider.py redirects the browser to the loopback
# callback, and the adapter exchanges the code. Run it in a logged-in GUI
# session. See docs/real-browser-verification.md.
#
# Usage: scripts/check-macos-real-browser.sh [result directory]
# Environment: REAL_BROWSER_TIMEOUT (seconds, default 120).
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

result_dir=${1:-target/real-browser/macos}
timeout=${REAL_BROWSER_TIMEOUT:-120}
mkdir -p "$result_dir"
rm -f "$result_dir"/provider-result.json "$result_dir"/check-output.txt

output_dir=target/uniffi
scripts/generate-bindings.sh "$output_dir"
swift_file="$output_dir/swift/native_oidc.swift"
module_map="$output_dir/swift/native_oidcFFI.modulemap"
case "$(uname -m)" in
    arm64) host_macos_target=arm64-apple-macosx15.0 ;;
    x86_64) host_macos_target=x86_64-apple-macosx15.0 ;;
    *)
        echo "unsupported macOS architecture for the real browser check" >&2
        exit 1
        ;;
esac
mkdir -p target/apple
swiftc \
    -parse-as-library \
    -target "$host_macos_target" \
    -sdk "$(xcrun --sdk macosx --show-sdk-path)" \
    -Xcc "-fmodule-map-file=$module_map" \
    -L target/debug \
    -lnative_oidc \
    -Xlinker -rpath \
    -Xlinker "$repo_root/target/debug" \
    -o target/apple/mac-real-browser-check \
    "$swift_file" \
    Sources/NativeOIDC/MacOSAuthorizationAdapter.swift \
    Sources/NativeOIDC/URLSessionNativeHttpTransport.swift \
    native/apple/MacOSRealBrowserCheck.swift

{
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "commit: $(git rev-parse HEAD 2>/dev/null || echo unknown)"
    sw_vers
    echo "architecture: $(uname -m)"
} > "$result_dir/environment.txt"
cat "$result_dir/environment.txt"

provider_output="$result_dir/provider-stdout.txt"
python3 scripts/real_browser_mock_provider.py \
    --result-file "$result_dir/provider-result.json" \
    --timeout "$((timeout + 30))" \
    > "$provider_output" 2> "$result_dir/provider-log.txt" &
provider_pid=$!
trap 'kill "$provider_pid" 2>/dev/null || true' EXIT

base_url=
for _ in $(seq 1 100); do
    base_url=$(head -n 1 "$provider_output" 2>/dev/null || true)
    [[ -n "$base_url" ]] && break
    sleep 0.1
done
if [[ -z "$base_url" ]]; then
    echo "the mock provider did not report its address" >&2
    exit 1
fi

check_status=0
target/apple/mac-real-browser-check "$base_url" "$timeout" 2>&1 | tee "$result_dir/check-output.txt" || check_status=$?
if [[ "$check_status" -ne 0 ]]; then
    # The provider would otherwise wait for its own timeout.
    kill "$provider_pid" 2>/dev/null || true
fi
provider_status=0
wait "$provider_pid" || provider_status=$?
trap - EXIT

cat "$result_dir/provider-result.json" 2>/dev/null || cat "$result_dir/provider-log.txt"
if [[ "$check_status" -ne 0 || "$provider_status" -ne 0 ]]; then
    echo "Real system-browser check failed (adapter: $check_status, provider: $provider_status)" >&2
    exit 1
fi
echo "Real system-browser check passed; results in $result_dir"

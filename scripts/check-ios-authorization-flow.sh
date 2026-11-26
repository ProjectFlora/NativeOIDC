#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
# shellcheck source=scripts/apple-phase-timing.sh
source "$repo_root/scripts/apple-phase-timing.sh"

bundle_id=net.project-flora.nativeoidc.ios-flow-smoke
app_bundle="$repo_root/target/apple/ios-authorization-flow-smoke.app"
executable=NativeOIDCIOSFlowSmoke
generated="$repo_root/target/uniffi/swift"
xcframework="$repo_root/target/apple/release-package/NativeOIDC-Apple/NativeOIDC.xcframework"
simulator_slice="$xcframework/ios-arm64-simulator"
provider_port_file="$repo_root/target/apple/ios-mock-provider.port"
provider_log="$repo_root/target/apple/ios-mock-provider.log"
provider_pid=""
simulator_id=""
booted_by_script=false
settle_log="$repo_root/target/apple/ios-flow-settle-sampler.log"
settle_dir=""
settle_pids=""
settle_started=0

# Installed before any phase or resource, so every exit path records the
# phase that stopped the script (timing writes are best-effort and cannot
# interrupt the rest) and then stops the app, the mock provider and a
# simulator this script booted.
cleanup() {
    phase_fail_open
    if [[ -n "$settle_pids" ]]; then
        settle_kill
    fi
    if [[ -n "$simulator_id" ]]; then
        xcrun simctl terminate "$simulator_id" "$bundle_id" >/dev/null 2>&1 || true
    fi
    if [[ -n "$provider_pid" ]]; then
        kill "$provider_pid" >/dev/null 2>&1 || true
        wait "$provider_pid" 2>/dev/null || true
    fi
    if [[ "$booted_by_script" == true ]]; then
        xcrun simctl shutdown "$simulator_id" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

# Settle diagnostics (#235). The first command after the boot status wait
# (the uninstall below) can wait minutes while the simulator is still
# settling. While it runs, background probes record when each candidate
# readiness signal first answers, and a sampler records the host load and the
# busiest processes, all relative to the end of the boot status wait. The
# diagnostics are best-effort, like the phase timing: nothing here can stop
# the flow or change its result, and every probe attempt is time-limited.
settle_now() {
    date +%s
}

# Run a command with a time limit in its own process group (macOS has no
# timeout(1)); on expiry or on SIGTERM the whole group is stopped.
settle_bounded() {
    perl -e '
        my $limit = shift;
        my $pid = fork() // exit 125;
        if ($pid == 0) { setpgrp(0, 0); exec @ARGV or exit 127; }
        $SIG{ALRM} = sub { kill "TERM", -$pid; waitpid($pid, 0); exit 124; };
        $SIG{TERM} = sub { kill "TERM", -$pid; waitpid($pid, 0); exit 143; };
        alarm $limit;
        waitpid($pid, 0);
        exit($? & 127 ? 128 + ($? & 127) : $? >> 8);
    ' "$@"
}

# settle_probe NAME COMMAND: retry COMMAND (run by bash, with the device in
# $SETTLE_UDID) until it succeeds, then record the elapsed seconds.
settle_probe() {
    local name=$1 command=$2 attempts=0
    set +e
    while :; do
        attempts=$((attempts + 1))
        if settle_bounded 30 bash -c "$command" >/dev/null 2>&1; then
            printf '%s\t%s\n' $(($(settle_now) - settle_started)) "$attempts" > "$settle_dir/$name"
            return 0
        fi
        sleep 1
    done
}

settle_sampler() {
    set +e
    while :; do
        {
            echo "--- +$(($(settle_now) - settle_started))s loadavg $(sysctl -n vm.loadavg)"
            top -l 2 -n 0 -s 1 | grep 'CPU usage' | tail -n 1
            ps -Aceo pcpu=,rss=,comm= -r | head -n 8
        } >> "$settle_log" 2>&1
        sleep 9
    done
}

# Stop each probe and the sampler before its current command (a time-limited
# probe attempt stops its own process group), so no shell reports the
# command's termination; the job notices for the stopped loops are discarded.
settle_kill() {
    local pid children=""
    for pid in $settle_pids; do
        children+=" $(pgrep -P "$pid" | tr '\n' ' ' || true)"
    done
    # Word splitting is intended: both lists hold process ids.
    # shellcheck disable=SC2086
    kill $settle_pids || true
    # shellcheck disable=SC2086
    kill $children || true
    for pid in $settle_pids; do
        wait "$pid" || true
    done
    settle_pids=""
} >/dev/null 2>&1

settle_start() {
    settle_started=$(settle_now)
    if ! settle_dir=$(mktemp -d "${TMPDIR:-/tmp}/ios-flow-settle.XXXXXX" 2>/dev/null) ||
        ! { mkdir -p "$(dirname -- "$settle_log")" && : > "$settle_log"; } 2>/dev/null; then
        echo "warning: could not start the simulator settle diagnostics" >&2
        settle_dir=""
        return 0
    fi
    {
        echo "runner: $(uname -a)"
        echo "hw.ncpu=$(sysctl -n hw.ncpu) hw.perflevel0.physicalcpu=$(sysctl -n hw.perflevel0.physicalcpu 2>/dev/null) hw.memsize=$(sysctl -n hw.memsize)"
        echo "loadavg at boot status end: $(sysctl -n vm.loadavg)"
    } >> "$settle_log" 2>&1 || true
    export SETTLE_UDID=$simulator_id
    settle_probe spawn 'xcrun simctl spawn "$SETTLE_UDID" launchctl list' 2>/dev/null &
    settle_pids+=" $!"
    settle_probe springboard 'xcrun simctl spawn "$SETTLE_UDID" launchctl list | grep -Eq "^[0-9]+[[:space:]].*[[:space:]]com\.apple\.SpringBoard$"' 2>/dev/null &
    settle_pids+=" $!"
    settle_probe screenshot "xcrun simctl io \"\$SETTLE_UDID\" screenshot --type=png \"$settle_dir/screenshot.png\"" 2>/dev/null &
    settle_pids+=" $!"
    settle_probe listapps 'xcrun simctl listapps "$SETTLE_UDID"' 2>/dev/null &
    settle_pids+=" $!"
    settle_sampler 2>/dev/null &
    settle_pids+=" $!"
}

# Stop the probes when the uninstall returns, and report which signals
# answered before it: as [settle] log lines, the sampler log, and the summary.
settle_report() {
    local returned name seconds attempts report
    if [[ -z "$settle_dir" ]]; then
        return 0
    fi
    returned=$(($(settle_now) - settle_started))
    settle_kill
    report="iOS flow settle (#235): uninstall returned at +${returned}s after boot status"
    echo "[settle] uninstall returned: ${returned}s" >&2
    for name in spawn springboard screenshot listapps; do
        if [[ -s "$settle_dir/$name" ]]; then
            IFS=$'\t' read -r seconds attempts < "$settle_dir/$name"
            echo "[settle] probe $name: ${seconds}s (attempt $attempts)" >&2
            report+="; $name +${seconds}s"
        else
            echo "[settle] probe $name: not ready by uninstall return" >&2
            report+="; $name not ready"
        fi
    done
    if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
        echo "::group::iOS simulator settle sampler"
    else
        echo "--- iOS simulator settle sampler ---"
    fi
    cat "$settle_log" 2>/dev/null || true
    if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
        echo "::endgroup::"
    fi
    if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]] &&
        ! { echo "$report" >> "$GITHUB_STEP_SUMMARY"; } 2>/dev/null; then
        echo "warning: could not write the settle report to $GITHUB_STEP_SUMMARY" >&2
    fi
    rm -rf "$settle_dir" 2>/dev/null || true
    settle_dir=""
}

phase_begin "iOS flow: check host, SDK and simulator slice"
if [[ "$(uname -m)" != arm64 ]]; then
    echo "iOS simulator smoke test requires an arm64 Apple host" >&2
    exit 1
fi

simulator_sdk=$(xcrun --sdk iphonesimulator --show-sdk-path)

if [[ ! -f "$simulator_slice/libnative_oidc.a" ]]; then
    echo "iOS simulator library is missing: $simulator_slice/libnative_oidc.a" >&2
    exit 1
fi
phase_end

phase_begin "iOS flow: build and sign app"
rm -rf "$app_bundle"
mkdir -p "$app_bundle"
cp native/apple/IOSAuthorizationFlowSmoke-Info.plist "$app_bundle/Info.plist"

xcrun --sdk iphonesimulator swiftc \
    -parse-as-library \
    -target arm64-apple-ios18.0-simulator \
    -sdk "$simulator_sdk" \
    -Xcc "-fmodule-map-file=$simulator_slice/Headers/module.modulemap" \
    -L "$simulator_slice" \
    -lnative_oidc \
    -framework AuthenticationServices \
    -framework Network \
    -framework Security \
    -framework UIKit \
    -o "$app_bundle/$executable" \
    "$generated/native_oidc.swift" \
    Sources/NativeOIDC/IOSAuthorizationAdapter.swift \
    native/apple/IOSAuthorizationFlowSmoke.swift \
    Sources/NativeOIDC/URLSessionNativeHttpTransport.swift

codesign --force --sign - --timestamp=none "$app_bundle"
phase_end

phase_begin "iOS flow: select simulator"
simulator_selection=$(
    xcrun simctl list devices available -j |
        python3 scripts/select_ios_simulator.py "${IOS_SIMULATOR_MAJOR_VERSION:-}"
)
IFS=$'\t' read -r simulator_id simulator_state simulator_runtime <<< "$simulator_selection"
echo "iOS flow simulator: $simulator_id ($simulator_runtime; $simulator_state)"
sw_vers
xcodebuild -version
# The summary line is diagnostic, like the timing rows: a failed write warns
# and does not stop the flow.
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]] &&
    ! { echo "iOS authorization: $simulator_runtime on $(uname -m), simulator $simulator_id" >> "$GITHUB_STEP_SUMMARY"; } 2>/dev/null; then
    echo "warning: could not write the iOS authorization summary to $GITHUB_STEP_SUMMARY" >&2
fi
phase_end
if [[ "$simulator_state" != Booted ]]; then
    phase_begin "iOS flow: simctl boot"
    # Set before the boot, so cleanup also shuts down a device whose boot
    # started but did not complete.
    booted_by_script=true
    xcrun simctl boot "$simulator_id"
    phase_end
fi

# Record what the app and the simulator saw when the flow did not pass. The
# app logs its scene and window state on failure and carries it in the result
# file; the simulator state is read back from simctl. The app log starts at
# launch time, not a fixed recent window, so an app that traps early (for
# example in a lifecycle precondition) still leaves its failure context.
diagnostics() {
    echo "--- simulator state ---" >&2
    xcrun simctl list devices available -j 2>/dev/null |
        python3 -c '
import json, sys
target = sys.argv[1]
for runtime, devices in json.load(sys.stdin).get("devices", {}).items():
    for device in devices:
        if device.get("udid") == target:
            print(device["name"] + " (" + runtime + "): " + str(device.get("state")))
' "$simulator_id" || echo "simulator state unavailable" >&2
    echo "--- app log (since launch) ---" >&2
    xcrun simctl spawn "$simulator_id" log show \
        --start "$log_start" \
        --style compact \
        --predicate "process == \"$executable\"" |
        tail -n 40 || true
    echo "--- mock provider log ---" >&2
    cat "$provider_log" >&2
}

phase_begin "iOS flow: simctl bootstatus"
xcrun simctl bootstatus "$simulator_id" -b
settle_start || true
phase_begin "iOS flow: uninstall previous app"
xcrun simctl uninstall "$simulator_id" "$bundle_id" >/dev/null 2>&1 || true
settle_report || true
phase_begin "iOS flow: start mock provider"
rm -f "$provider_port_file"
python3 scripts/ios-mock-oauth-provider.py --port-file "$provider_port_file" \
    > "$provider_log" 2>&1 &
provider_pid=$!
for ((attempt = 0; attempt < 600; attempt++)); do
    if [[ -s "$provider_port_file" ]]; then
        break
    fi
    if ! kill -0 "$provider_pid" 2>/dev/null; then
        cat "$provider_log" >&2
        echo "mock OAuth provider exited before becoming ready" >&2
        exit 1
    fi
    sleep 0.1
done
if [[ ! -s "$provider_port_file" ]]; then
    cat "$provider_log" >&2
    echo "mock OAuth provider did not start" >&2
    exit 1
fi
provider_port=$(<"$provider_port_file")
phase_begin "iOS flow: install app"
xcrun simctl install "$simulator_id" "$app_bundle"
phase_begin "iOS flow: launch app"
# Anchor the diagnostics log window at launch, not a fixed recent span, so an
# app that dies early still leaves its failure context (see diagnostics). The
# in-simulator `log` tool does not parse ISO 8601, so use a local timestamp.
log_start=$(date '+%Y-%m-%d %H:%M:%S')
if ! xcrun simctl launch "$simulator_id" "$bundle_id" --oauth-provider-port "$provider_port"; then
    diagnostics
    exit 1
fi

phase_begin "iOS flow: wait for result"
data_container=$(xcrun simctl get_app_container "$simulator_id" "$bundle_id" data)
result_file="$data_container/Documents/authorization-flow-result"
# The app waits up to 90s for its scene to be foreground active before
# starting the session (#203), on top of a slow simulator boot and launch, so
# the window must cover that before the result file can appear: 150 x 2s = 300s.
for ((attempt = 0; attempt < 150; attempt++)); do
    if [[ -f "$result_file" ]]; then
        result=$(<"$result_file")
        if [[ "$result" == success ]]; then
            phase_end
            echo "iOS ASWebAuthenticationSession mock-provider flow passed"
            exit 0
        fi
        echo "iOS authorization flow failed: $result" >&2
        diagnostics
        exit 1
    fi
    sleep 2
done

diagnostics
echo "timed out waiting for iOS authorization flow result" >&2
exit 1

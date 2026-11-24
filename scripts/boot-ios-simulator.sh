#!/usr/bin/env bash
set -euo pipefail

# Select an iPhone simulator for IOS_SIMULATOR_MAJOR_VERSION (an iOS major or
# `latest`; empty means any) with scripts/select_ios_simulator.py, boot it,
# and wait until it has finished booting.
#
#   scripts/boot-ios-simulator.sh SELECTION_FILE
#
# SELECTION_FILE receives `udid<TAB>initial state<TAB>runtime` before the boot
# starts, so a caller that stops this script partway (for example the cleanup
# of scripts/package-apple-xcframework.sh, which runs it in the background
# while the Rust slices build) still knows which device to shut down: one
# whose initial state was not Booted. This script never shuts the device down
# itself; the caller owns it, and passes it to
# scripts/check-ios-authorization-flow.sh as IOS_SIMULATOR_UDID.

repo_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
# shellcheck source=scripts/apple-phase-timing.sh
source "$repo_root/scripts/apple-phase-timing.sh"

if [[ $# -ne 1 || -z "$1" ]]; then
    echo "Usage: $0 SELECTION_FILE" >&2
    exit 2
fi
selection_file=$1

trap phase_fail_open EXIT

phase_begin "iOS simulator: select"
simulator_selection=$(
    xcrun simctl list devices available -j |
        python3 scripts/select_ios_simulator.py "${IOS_SIMULATOR_MAJOR_VERSION:-}"
)
IFS=$'\t' read -r simulator_id simulator_state simulator_runtime <<< "$simulator_selection"
mkdir -p "$(dirname -- "$selection_file")"
printf '%s\t%s\t%s\n' "$simulator_id" "$simulator_state" "$simulator_runtime" > "$selection_file.tmp"
mv "$selection_file.tmp" "$selection_file"
phase_end
echo "iOS simulator: $simulator_id ($simulator_runtime; $simulator_state)"

if [[ "$simulator_state" != Booted ]]; then
    phase_begin "iOS simulator: simctl boot"
    xcrun simctl boot "$simulator_id"
    phase_end
fi

phase_begin "iOS simulator: simctl bootstatus"
xcrun simctl bootstatus "$simulator_id" -b
phase_end

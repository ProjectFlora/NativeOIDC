#!/usr/bin/env bash
set -euo pipefail

# Build, verify and archive the Apple release package. CI, the release
# workflow and local runs all use this one entry point; it runs the phases in
# order and records how long each took (scripts/apple-phase-timing.sh) in the
# step summary, including a failed run's last phase:
#
# 1. Build: UniFFI bindings and the four Rust release slices.
# 2. Stage: Swift sources, notices, README, headers and the universal macOS
#    library in target/apple/release-package.
# 3. XCFramework: create it, and check that its canonical Info.plist is the
#    same across two create-xcframework runs.
# 4. Verify: the macOS static-link smoke test
#    (scripts/check-apple-static-link-smoke.sh) and the iOS simulator
#    authorization flow (scripts/check-ios-authorization-flow.sh), which
#    records its simulator phases separately.
# 5. Archive: the release zip and its determinism self-test.
#
# Each verification script also runs on its own against an existing staging.

repo_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
# shellcheck source=scripts/apple-phase-timing.sh
source "$repo_root/scripts/apple-phase-timing.sh"

if [[ "$(uname -s)" != Darwin ]]; then
    echo "Apple XCFramework packaging requires macOS and Xcode" >&2
    exit 1
fi

version=${1:-0.0.0-ci}
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][A-Za-z0-9.]+)?$ ]]; then
    echo "Usage: $0 [version]" >&2
    exit 2
fi

output_root="$repo_root/target/apple/release-package"
package_root="$output_root/NativeOIDC-Apple"
xcframework="$package_root/NativeOIDC.xcframework"
generated="$repo_root/target/uniffi"
headers="$output_root/headers"
macos_library="$repo_root/target/apple/libnative_oidc_macos.a"

# Require the whole output directory to be absent: the CI workflows remove
# it in full before each run, and a local re-run must not stage over a
# previous run's contents (a partial staging, self-test outputs, or older
# archives).
if [[ -e "$output_root" ]]; then
    echo "Package output directory already exists: $output_root" >&2
    echo "Remove it before re-running (this also deletes previously generated archives; move any you need elsewhere first):" >&2
    echo "  rm -rf \"$output_root\"" >&2
    exit 1
fi

# The self-test outputs below. The guard above requires $output_root to be
# absent, so none exists from an earlier run; cleanup removes them on every
# exit path, so a run never leaves them behind. Both stay in $output_root,
# outside the staged $package_root, so a crashed run can never leave them in
# the release tree that the archive embeds.
scratch_xcframework="$output_root/NativeOIDC-Apple-scratch.xcframework"
recheck="$output_root/NativeOIDC-Apple-$version-determinism.zip"

# A fresh timing file for this run, shared with the scripts it calls. Timing
# is diagnostic only: if the file cannot be created, the phases are still
# logged as [timing] lines and packaging continues.
if APPLE_PHASE_TIMING_FILE=$(mktemp "${TMPDIR:-/tmp}/package-phase-timings.XXXXXX" 2>/dev/null); then
    export APPLE_PHASE_TIMING_FILE
else
    echo "warning: could not create a phase timing file; the phase table is not written" >&2
    unset APPLE_PHASE_TIMING_FILE
fi
package_started=$SECONDS

# A failure to remove the self-test outputs must not skip the phase table or
# replace the packaging error, so it is held until the table is written. It
# fails a run that otherwise succeeded.
cleanup() {
    local status=$? cleanup_status=0
    rm -rf "$scratch_xcframework" "$recheck" || cleanup_status=$?
    phase_fail_open
    if ((status == 0 && cleanup_status == 0)); then
        phase_record "Total" ok $((SECONDS - package_started))
    else
        phase_record "Total" failed $((SECONDS - package_started))
    fi
    phase_write_summary "Apple packaging phases"
    if [[ -n "${APPLE_PHASE_TIMING_FILE:-}" ]]; then
        rm -f "$APPLE_PHASE_TIMING_FILE" 2>/dev/null || true
    fi
    if ((cleanup_status != 0)); then
        echo "Could not remove the packaging self-test outputs: $scratch_xcframework $recheck" >&2
        if ((status == 0)); then
            exit "$cleanup_status"
        fi
    fi
}
trap cleanup EXIT

export IPHONEOS_DEPLOYMENT_TARGET=18.0
export MACOSX_DEPLOYMENT_TARGET=15.0

phase_begin "Build: UniFFI bindings"
scripts/generate-bindings.sh "$generated"
phase_end

for target in \
    aarch64-apple-ios \
    aarch64-apple-ios-sim \
    aarch64-apple-darwin \
    x86_64-apple-darwin; do
    phase_begin "Build: Rust $target"
    rustup target add "$target"
    cargo build --locked --release --lib --target "$target"
    phase_end
done

phase_begin "Stage: sources, notices, headers, macOS library"
mkdir -p "$repo_root/target/apple" "$package_root/Swift" "$headers"
# The Swift package sources (the UniFFI-generated API plus the platform
# adapters) ship in the archive so manual Xcode users get the same content as
# the Swift Package Manager target. The generated file is copied from the
# freshly generated output, which the SPM bindings check verifies matches the
# checked-in copy.
cp "$generated/swift/native_oidc.swift" "$package_root/Swift/"
cp "$repo_root"/Sources/NativeOIDC/IOSAuthorizationAdapter.swift \
    "$repo_root"/Sources/NativeOIDC/MacOSAuthorizationAdapter.swift \
    "$repo_root"/Sources/NativeOIDC/URLSessionNativeHttpTransport.swift \
    "$package_root/Swift/"
cp "$repo_root/LICENSE" "$package_root/"
scripts/generate-third-party-licenses.sh "$package_root/THIRD_PARTY_LICENSES.txt"
cp "$repo_root/docs/swift-xcframework.md" "$package_root/README.md"
cp "$generated/swift/native_oidcFFI.h" "$headers/"
cp "$generated/swift/native_oidcFFI.modulemap" "$headers/module.modulemap"

xcrun lipo -create \
    target/aarch64-apple-darwin/release/libnative_oidc.a \
    target/x86_64-apple-darwin/release/libnative_oidc.a \
    -output "$macos_library"
phase_end

phase_begin "XCFramework: create and check Info.plist stability"
# The slice inputs are shared by both create-xcframework calls below; the
# xcframework names derive from the libraries, so only -output differs.
xcframework_args=(
    -library target/aarch64-apple-ios/release/libnative_oidc.a
    -headers "$headers"
    -library target/aarch64-apple-ios-sim/release/libnative_oidc.a
    -headers "$headers"
    -library "$macos_library"
    -headers "$headers"
)

xcodebuild -create-xcframework \
    "${xcframework_args[@]}" \
    -output "$xcframework"

# xcodebuild writes AvailableLibraries in a run-varying order (its internal
# slice enumeration, not the input order), so the same package yields a
# different Info.plist — and therefore a different release archive — from
# run to run. The in-run determinism self-test below therefore creates the
# framework a second time, canonicalizes both plists
# (scripts/normalize_xcframework_plist.py: validate the slice set, sort the
# array and each architecture list, rewrite in a fixed format), and requires
# identical bytes before the archive embeds the framework. The scratch copy
# ($scratch_xcframework, defined with the guard) is removed by cleanup on
# every exit path.
xcodebuild -create-xcframework \
    "${xcframework_args[@]}" \
    -output "$scratch_xcframework"

python3 "$repo_root/scripts/normalize_xcframework_plist.py" \
    "$xcframework/Info.plist"
python3 "$repo_root/scripts/normalize_xcframework_plist.py" \
    "$scratch_xcframework/Info.plist"
if ! cmp -s "$xcframework/Info.plist" "$scratch_xcframework/Info.plist"; then
    echo "XCFramework Info.plist is not stable across create-xcframework runs (run-varying order survived canonicalization)" >&2
    exit 1
fi
rm -rf "$scratch_xcframework"
phase_end

phase_begin "Verify: macOS static-link smoke"
scripts/check-apple-static-link-smoke.sh
phase_end

# The iOS flow records its own phases (app build, simulator selection, boot,
# install, launch, result) in the same table.
scripts/check-ios-authorization-flow.sh

phase_begin "Archive: zip and determinism self-test"

# Keep the archive bytes free of build-time and file-state inputs that
# `ditto` would otherwise record. The in-run self-test below zips the same
# staged tree a second time and requires an identical SHA-256. It runs on
# one machine in one job, so it checks only that this zip step repeats; it
# cannot detect differences between machines or between staging passes
# (see docs/releasing.md: "What the packaging self-test checks"). Three
# layers remove those inputs:
#
# 1. Fixed timestamps. `ditto` records each file's modification time in the
#    zip DOS time fields and as atime + mtime words in the 0x5855 (Info-ZIP
#    "UX") extra field; unpinned build outputs would carry their build time.
#    `touch -h -t` pins both to 2026-01-01 00:00:00; `-h` stamps symlinks
#    themselves (ditto records their lstat times) instead of their targets.
# 2. Empty attribute sets. `xattr -cr` drops extended attributes so `ditto`
#    does not embed build-machine metadata in the archive.
# 3. Post-zip normalization (scripts/normalize_zip.py). Three things escape
#    filesystem-level pinning. APFS advances atimes when `ditto` reads the
#    tree, so the UX atime word drifts between zips even though the files
#    were pinned; the normalizer aligns every UX atime word with the pinned
#    mtime word. The local-header UX field also records the staging user's
#    uid/gid; the normalizer drops it, rewriting the 12-byte shape to the
#    8-byte shape, so the archive records no owner and its bytes do not
#    depend on who ran the packaging. And the `com.apple.provenance`
#    attribute cannot be removed (`xattr -d` is a silent no-op), so `ditto`
#    always emits a constant `._*` AppleDouble sidecar per entry; the
#    normalizer strips them.
#    `--sequesterRsrc` is deliberately omitted: the `__MACOSX/` shadow tree
#    it creates inside the archive is stamped with the zip run time.
#
# The timezone is fixed for the metadata and zip steps: `touch -t` interprets
# the pin in the local timezone and `ditto` derives the DOS time fields from
# it, so the same wall-clock pin would otherwise yield different archive
# bytes on machines in different time zones.
export TZ=UTC
xattr -cr "$package_root"
find "$package_root" -print0 | xargs -0 touch -h -t 202601010000.00

archive="$output_root/NativeOIDC-Apple-$version.zip"
ditto -c -k --keepParent "$package_root" "$archive"
python3 "$repo_root/scripts/normalize_zip.py" "$archive"

# The in-run determinism self-test: zip the same staged tree a second time,
# normalize both, and demand identical hashes. Staging is not repeated.
# (The raw double-ditto hashes would differ: the first zip's read of the
# tree advances file atimes, which the second zip records in the UX extra
# field; the normalizer is what makes the comparison meaningful.) The recheck
# zip ($recheck, defined with the guard) is removed by cleanup on every exit
# path.
ditto -c -k --keepParent "$package_root" "$recheck"
python3 "$repo_root/scripts/normalize_zip.py" "$recheck"
first_hash=$(shasum -a 256 "$archive" | awk '{print $1}')
second_hash=$(shasum -a 256 "$recheck" | awk '{print $1}')
rm -f "$recheck"
if [[ "$first_hash" != "$second_hash" ]]; then
    echo "Apple release archive packaging is not deterministic: $first_hash != $second_hash" >&2
    exit 1
fi
phase_end
echo "Apple release archive packaging is deterministic ($first_hash)"

#!/usr/bin/env bash
# Publishes the Android library to a throwaway local Maven repository and
# checks the files Maven Central requires: the AAR with both Rust ABIs, a POM
# with complete metadata and the transitive dependencies, and the sources and
# javadoc jars. Signatures are checked when signing is configured.
#
# Optional environment, used by the release workflow's Maven publication
# (Issue #180):
#   ADOPTED_AAR      Publish this AAR (the GitHub Release asset) instead of
#                    building one. The AAR build, including the Rust NDK
#                    builds, is excluded from the Gradle run.
#   EXPECTED_SHA256  The published AAR, and the AAR entry of the module
#                    metadata, must have this SHA-256.
#   BUNDLE_OUT       Stage the publication in a file repository instead, which
#                    adds the .md5 and .sha1 checksums Central requires, and
#                    zip it to this path as the Central Portal upload bundle
#                    (without maven-metadata files, like the Gradle plugin).
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <version>" >&2
    exit 2
fi

version=$1
repo_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
local_repo=$(mktemp -d)
trap 'rm -rf "$local_repo"' EXIT

gradle_args=(-PreleaseVersion="$version")
if [[ -n "${ADOPTED_AAR:-}" ]]; then
    test -s "$ADOPTED_AAR" || { echo "ADOPTED_AAR $ADOPTED_AAR is missing or empty" >&2; exit 1; }
    gradle_args+=(-PadoptedAar="$(cd -- "$(dirname -- "$ADOPTED_AAR")" && pwd)/$(basename -- "$ADOPTED_AAR")" -x bundleReleaseAar)
fi
if [[ -n "${BUNDLE_OUT:-}" ]]; then
    gradle_args+=(-PcentralBundleRepository="$local_repo" publishAllPublicationsToCentralBundleRepository)
else
    gradle_args+=(publishToMavenLocal -Dmaven.repo.local="$local_repo")
fi

"$repo_root/native/android/gradlew" -p "$repo_root/native/android" "${gradle_args[@]}"

dir="$local_repo/net/project-flora/nativeoidc/native-oidc-android/$version"
base="$dir/native-oidc-android-$version"
files=("$base.aar" "$base.pom" "$base.module" "$base-sources.jar" "$base-javadoc.jar")
for file in "${files[@]}"; do
    test -s "$file" || { echo "missing $file" >&2; ls -l "$dir" >&2; exit 1; }
done

unzip -Z1 "$base.aar" | grep -Fx jni/arm64-v8a/libnative_oidc.so
unzip -Z1 "$base.aar" | grep -Fx jni/x86_64/libnative_oidc.so
# The license notices are java resources, so they live in classes.jar.
unzip -p "$base.aar" classes.jar > "$local_repo/classes.jar"
unzip -Z1 "$local_repo/classes.jar" | grep -Fx META-INF/native-oidc/LICENSE
unzip -Z1 "$local_repo/classes.jar" | grep -Fx META-INF/native-oidc/THIRD_PARTY_LICENSES.txt
unzip -Z1 "$local_repo/classes.jar" | grep -Fx META-INF/native-oidc/RUST_STANDARD_LIBRARY_COPYRIGHT.html
rm "$local_repo/classes.jar"
unzip -Z1 "$base-sources.jar" | grep -E '(^|/)uniffi/native_oidc/native_oidc\.kt$'
unzip -Z1 "$base-sources.jar" | grep -E '(^|/)net/projectflora/nativeoidc/AndroidAuthorizationAdapter\.kt$'

pom="$base.pom"
grep -F '<groupId>net.project-flora.nativeoidc</groupId>' "$pom"
grep -F "<version>$version</version>" "$pom"
grep -F '<packaging>aar</packaging>' "$pom"
for element in name description url licenses developers scm; do
    grep -q "<$element>" "$pom" || { echo "POM lacks <$element>" >&2; exit 1; }
done
# jna must stay an AAR-typed dependency so consumers get its native libraries.
grep -A4 -F '<artifactId>jna</artifactId>' "$pom" | grep -F '<type>aar</type>'
grep -F '<artifactId>kotlinx-coroutines-android</artifactId>' "$pom"
grep -F '<artifactId>browser</artifactId>' "$pom"

if [[ -n "${EXPECTED_SHA256:-}" ]]; then
    actual=$(sha256sum "$base.aar" | cut -d' ' -f1)
    if [[ "$actual" != "$EXPECTED_SHA256" ]]; then
        echo "The published AAR has SHA-256 $actual, expected $EXPECTED_SHA256" >&2
        exit 1
    fi
    # The module metadata lists each file with its hashes; consumers that
    # resolve through it check the AAR against this entry.
    python3 - "$base.module" "native-oidc-android-$version.aar" "$EXPECTED_SHA256" <<'EOF'
import json
import sys

module_path, aar_name, expected = sys.argv[1:]
with open(module_path, encoding="utf-8") as module:
    metadata = json.load(module)
entries = [
    entry
    for variant in metadata.get("variants", [])
    for entry in variant.get("files", [])
    if entry.get("name") == aar_name
]
if not entries:
    sys.exit(f"{module_path} lists no {aar_name}")
for entry in entries:
    if entry.get("sha256") != expected:
        sys.exit(f"{module_path} lists {aar_name} with SHA-256 {entry.get('sha256')}, expected {expected}")
EOF
    echo "The published AAR and its module metadata match SHA-256 $EXPECTED_SHA256"
fi

if [[ -n "${ORG_GRADLE_PROJECT_signingInMemoryKey:-}" ]]; then
    for file in "${files[@]}"; do
        test -s "$file.asc" || { echo "missing signature $file.asc" >&2; exit 1; }
    done
fi

if [[ -n "${BUNDLE_OUT:-}" ]]; then
    for file in "${files[@]}"; do
        for checksum in md5 sha1; do
            test -s "$file.$checksum" || { echo "missing checksum $file.$checksum" >&2; exit 1; }
        done
    done
    bundle_out=$(cd -- "$(dirname -- "$BUNDLE_OUT")" && pwd)/$(basename -- "$BUNDLE_OUT")
    rm -f "$bundle_out"
    (cd "$local_repo" && find . -type f ! -name 'maven-metadata*' | sed 's|^\./||' | LC_ALL=C sort |
        zip -X -q "$bundle_out" -@)
    echo "Central Portal bundle written to $bundle_out:"
    unzip -Z1 "$bundle_out"
fi

echo "Maven publication for $version is complete"

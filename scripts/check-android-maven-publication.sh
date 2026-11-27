#!/usr/bin/env bash
# Publishes the Android library to a throwaway local Maven repository and
# checks the files Maven Central requires: the AAR with both Rust ABIs, a POM
# with complete metadata and the transitive dependencies, and the sources and
# javadoc jars. When signing is configured, every signature must exist, carry
# no armor "Version:" header and no unreplaced "@" placeholder (the bcpg jar
# Gradle ships writes the header with an unreplaced Bouncy Castle
# release-name token, Issue #169), and verify with the signing key; with
# BUNDLE_OUT, each signature's .md5/.sha1 must match its content.
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
        # The bcpg jar Gradle's signing plugin uses writes the armor "Version:"
        # header from Bouncy Castle's default version string, which is still an
        # unreplaced release-name placeholder in the published artifact (Issue
        # 169). The build drops the header after signing, so it must not be
        # here, and an unreplaced "@...@" token cannot survive anywhere else:
        # neither the base64 alphabet nor the armor delimiters use "@".
        if grep -q -- '^Version:' "$file.asc"; then
            echo "signature $file.asc carries a Version: armor header; the sign task must drop it (Issue #169)" >&2
            exit 1
        fi
        if LC_ALL=C grep -q '@' "$file.asc"; then
            echo "signature $file.asc contains an unreplaced '@' placeholder (Issue #169)" >&2
            exit 1
        fi
    done
    # The signatures must verify with the key that wrote them. The key material
    # is turned back into an armored block (check-gpg-keyserver.sh accepts the
    # same two forms: an armored block, or a single raw base64 key), imported
    # into a throwaway keyring, and every file is verified against it.
    command -v gpg >/dev/null 2>&1 || { echo "gpg is required to verify the signatures (Issue #169)" >&2; exit 1; }
    keyring=$(mktemp -d)
    trap 'rm -rf "$local_repo" "$keyring"' EXIT
    key_file="$keyring/signing-key.pgp"
    if grep -q -- '^-----BEGIN PGP' <<<"${ORG_GRADLE_PROJECT_signingInMemoryKey}"; then
        printf '%s\n' "${ORG_GRADLE_PROJECT_signingInMemoryKey}" > "$key_file"
    else
        {
            printf -- '-----BEGIN PGP PRIVATE KEY BLOCK-----\n'
            printf '%s\n' "${ORG_GRADLE_PROJECT_signingInMemoryKey}"
            printf -- '-----END PGP PRIVATE KEY BLOCK-----\n'
        } > "$key_file"
    fi
    GNUPGHOME="$keyring" gpg --batch --import < "$key_file" >/dev/null
    for file in "${files[@]}"; do
        if ! GNUPGHOME="$keyring" gpg --batch --no-verbose --verify "$file.asc" "$file" >/dev/null; then
            echo "signature $file.asc does not verify for $file" >&2
            exit 1
        fi
    done
fi

if [[ -n "${BUNDLE_OUT:-}" ]]; then
    for file in "${files[@]}"; do
        for checksum in md5 sha1; do
            test -s "$file.$checksum" || { echo "missing checksum $file.$checksum" >&2; exit 1; }
        done
    done
    if [[ -n "${ORG_GRADLE_PROJECT_signingInMemoryKey:-}" ]]; then
        for file in "${files[@]}"; do
            for checksum in md5 sha1; do
                test -s "$file.asc.$checksum" || { echo "missing checksum $file.asc.$checksum" >&2; exit 1; }
            done
        done
        # The .asc checksums must describe the signature files as the publish
        # task saw them (header already dropped, Issue 169); a mismatch means
        # the armor changed after the checksums were written.
        python3 - "$base" <<'EOF'
import hashlib
import os
import sys

base = sys.argv[1]
suffixes = (".aar", ".pom", ".module", "-sources.jar", "-javadoc.jar")

def digest(path, algorithm):
    handle = hashlib.new(algorithm)
    with open(path, "rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            handle.update(chunk)
    return handle.hexdigest()

mismatch = False
for suffix in suffixes:
    asc = base + suffix + ".asc"
    for algorithm, extension in (("md5", ".md5"), ("sha1", ".sha1")):
        with open(asc + extension, encoding="utf-8") as file:
            expected = file.read().split()[0]
        actual = digest(asc, algorithm)
        if actual != expected:
            print(f"{os.path.basename(asc)}{extension} has {algorithm} {actual}, expected {expected}", file=sys.stderr)
            mismatch = True
sys.exit(1 if mismatch else 0)
EOF
    fi
    bundle_out=$(cd -- "$(dirname -- "$BUNDLE_OUT")" && pwd)/$(basename -- "$BUNDLE_OUT")
    rm -f "$bundle_out"
    (cd "$local_repo" && find . -type f ! -name 'maven-metadata*' | sed 's|^\./||' | LC_ALL=C sort |
        zip -X -q "$bundle_out" -@)
    echo "Central Portal bundle written to $bundle_out:"
    unzip -Z1 "$bundle_out"
fi

echo "Maven publication for $version is complete"

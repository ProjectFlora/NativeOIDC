#!/usr/bin/env bash
# Downloads the Android AAR and SHA256SUMS of the GitHub Release v<version>
# into <directory> and checks the AAR against its SHA256SUMS line. The Maven
# Central publication adopts this file, so Central and the GitHub Release
# ship the same bytes (Issue #180). Needs GH_TOKEN; in GitHub Actions the
# repository comes from GITHUB_REPOSITORY.
#
# Writes path=<the AAR> and sha256=<its SHA-256> to GITHUB_OUTPUT when set.
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "Usage: $0 <version> <directory>" >&2
    exit 2
fi

version=$1
directory=$2
aar="native-oidc-android-$version.aar"
repo_args=()
[[ -n "${GITHUB_REPOSITORY:-}" ]] && repo_args=(--repo "$GITHUB_REPOSITORY")

mkdir -p "$directory"
cd "$directory"
rm -f "$aar" SHA256SUMS
gh release download "v$version" "${repo_args[@]}" --pattern "$aar" --pattern SHA256SUMS

lines=$(awk -v file="$aar" '$2 == file' SHA256SUMS)
if [[ $(printf '%s' "$lines" | grep -c .) -ne 1 ]]; then
    echo "SHA256SUMS of v$version must list $aar exactly once" >&2
    exit 1
fi
printf '%s\n' "$lines" | sha256sum --check --strict
sha256=${lines%% *}

echo "The GitHub Release AAR $aar has SHA-256 $sha256"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    {
        echo "path=$PWD/$aar"
        echo "sha256=$sha256"
    } >> "$GITHUB_OUTPUT"
fi

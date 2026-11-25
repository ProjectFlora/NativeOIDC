#!/usr/bin/env python3
"""Validate a release documentation set built by build-release-docs.py.

Usage:
  check-release-docs.py <archive.zip | extracted directory>
      --version <version> --commit-sha <sha> [--repo-root DIR]

A directory is either the archive's top directory
(NativeOIDC-Docs-<version>/) or the directory the archive was unzipped
into. The checks:

- every entry is under NativeOIDC-Docs-<version>/;
- the required files are present (REQUIRED);
- every relative link in every Markdown file resolves to a file in the
  archive, or to a directory with at least one file beneath it;
- the entry page records the given version and full commit SHA;
- the entry page's file list equals the archive's files;
- the entry page's guide presence table matches the archive, for each
  optional language guide.

With --repo-root (a checkout of the commit the set was built from), every
file under repo/ must be byte-identical to that checkout, and each
optional guide must be in the archive exactly when the checkout has it.

Exits 1 and prints one line per failure when any check fails.
"""
import argparse
import importlib.util
import os
import re
import sys
import zipfile

_SCRIPTS = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("build_release_docs", os.path.join(_SCRIPTS, "build-release-docs.py"))
build_release_docs = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(build_release_docs)

ENTRY = build_release_docs.ENTRY
REPO_DIR = build_release_docs.REPO_DIR
RUST_API_DIR = build_release_docs.RUST_API_DIR

REQUIRED = (
    ENTRY,
    f"{RUST_API_DIR}/{build_release_docs.RUST_CRATE_INDEX}",
) + tuple(f"{REPO_DIR}/{path}" for path in (
    "README.md",
    "docs/README.md",
    "docs/user-guide.md",
    "docs/api-coverage.md",
    "docs/swift-xcframework.md",
    "docs/kotlin-guide.md",
    "docs/windows-cpp-guide.md",
    "docs/windows-api.md",
    "native/windows/README.md",
    "native/windows/include/native_oidc_windows.h",
    "native/windows/dotnet/NativeOidcWindows.cs",
    "native/windows/dotnet/NativeOidcWindows.csproj",
    "examples/manual_authorization.rs",
))

_INVENTORY_ITEM = re.compile(r"^- `([^`]+)`$")
_GUIDE_ROW = re.compile(r"^\| `([^`]+)` \| (.+) \|$")


def load_archive(path, version):
    """Return ({archive path: bytes}, errors) for a zip file or directory."""
    top = f"NativeOIDC-Docs-{version}"
    files = {}
    errors = []
    if os.path.isdir(path):
        root = path
        if os.path.isdir(os.path.join(path, top)):
            root = os.path.join(path, top)
            others = sorted(set(os.listdir(path)) - {top})
            if others:
                errors.append(f"entries outside {top}/: {', '.join(others)}")
        elif os.path.basename(os.path.normpath(path)) != top:
            errors.append(f"{path} is neither {top}/ nor a directory that contains it")
            return files, errors
        for directory, subdirectories, names in os.walk(root):
            subdirectories.sort()
            for name in names:
                full = os.path.join(directory, name)
                with open(full, "rb") as handle:
                    files[os.path.relpath(full, root).replace(os.sep, "/")] = handle.read()
        return files, errors
    try:
        archive = zipfile.ZipFile(path)
    except (OSError, zipfile.BadZipFile) as error:
        return files, [f"cannot open {path}: {error}"]
    with archive:
        for info in archive.infolist():
            name = info.filename
            if not name.startswith(f"{top}/") or "/../" in f"/{name}" or "\\" in name:
                errors.append(f"entry `{name}` is outside {top}/")
                continue
            if info.is_dir():
                continue
            relative = name[len(top) + 1:]
            if relative in files:
                errors.append(f"entry `{name}` appears twice")
            files[relative] = archive.read(info)
    return files, errors


def check_links(files):
    errors = []
    directories = set()
    for path in files:
        parts = path.split("/")[:-1]
        for depth in range(1, len(parts) + 1):
            directories.add("/".join(parts[:depth]))
    for path in sorted(files):
        if not path.endswith(".md"):
            continue
        try:
            text = files[path].decode("utf-8")
        except UnicodeDecodeError:
            errors.append(f"{path}: not UTF-8")
            continue
        for line, target in build_release_docs.markdown_links(text):
            try:
                resolved = build_release_docs.resolve_link(path, target)
            except build_release_docs.BuildError as error:
                errors.append(f"{str(error).replace('the repository', 'the archive')} (line {line})")
                continue
            if resolved is None or resolved in files or resolved in directories or resolved == "":
                continue
            errors.append(f"{path}:{line}: link `{target}` names {resolved}, which is not in the archive")
    return errors


def check_entry(files, version, commit_sha):
    errors = []
    if ENTRY not in files:
        return errors  # reported as a missing required file
    lines = files[ENTRY].decode("utf-8", errors="replace").splitlines()
    if f"- Release version: `{version}`" not in lines:
        errors.append(f"{ENTRY}: release version line does not say `{version}`")
    if f"- Commit: `{commit_sha}`" not in lines:
        errors.append(f"{ENTRY}: commit line does not say `{commit_sha}`")
    inventory = set()
    in_files = False
    for line in lines:
        if line == "## Files":
            in_files = True
        elif line.startswith("## "):
            in_files = False
        elif in_files:
            match = _INVENTORY_ITEM.match(line)
            if match:
                inventory.add(match.group(1))
    if not inventory:
        errors.append(f"{ENTRY}: no file list")
    else:
        for path in sorted(set(files) - inventory):
            errors.append(f"{ENTRY}: file list omits {path}")
        for path in sorted(inventory - set(files)):
            errors.append(f"{ENTRY}: file list names {path}, which is not in the archive")
    rows = {}
    for line in lines:
        match = _GUIDE_ROW.match(line)
        if match:
            rows[match.group(1)] = match.group(2)
    for path, title, issue, _ in build_release_docs.guide_presence([]):
        expected = "yes" if f"{REPO_DIR}/{path}" in files else f"no (#{issue})"
        if rows.get(path) != expected:
            errors.append(f"{ENTRY}: guide presence for {path} is `{rows.get(path)}`, expected `{expected}`")
    return errors


def check_repo(files, repo_root):
    errors = []
    prefix = f"{REPO_DIR}/"
    for path in sorted(files):
        if not path.startswith(prefix):
            continue
        source = os.path.join(repo_root, path[len(prefix):])
        if not os.path.isfile(source):
            errors.append(f"{path}: not in the repository checkout")
            continue
        with open(source, "rb") as handle:
            if handle.read() != files[path]:
                errors.append(f"{path}: differs from the repository checkout")
    for path, title, issue, _ in build_release_docs.guide_presence([]):
        in_repo = os.path.isfile(os.path.join(repo_root, path))
        in_archive = f"{prefix}{path}" in files
        if in_repo != in_archive:
            where = "the checkout has it but the archive does not" if in_repo else \
                "the archive has it but the checkout does not"
            errors.append(f"{title} ({path}): {where}")
    return errors


def check(archive, version, commit_sha, repo_root=None):
    """Return the list of failures (empty when the set is valid)."""
    files, errors = load_archive(archive, version)
    if not files:
        return errors + ["the archive contains no files"]
    for path in REQUIRED:
        if path not in files:
            errors.append(f"required file {path} is missing")
    errors += check_links(files)
    errors += check_entry(files, version, commit_sha)
    if repo_root is not None:
        errors += check_repo(files, repo_root)
    return errors


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("archive")
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit-sha", required=True)
    parser.add_argument("--repo-root", default=None)
    args = parser.parse_args(argv)
    errors = check(args.archive, args.version, args.commit_sha, args.repo_root)
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    if errors:
        print(f"{len(errors)} check(s) failed for {args.archive}", file=sys.stderr)
        return 1
    print(f"{args.archive}: release documentation set for {args.version} at {args.commit_sha} is valid")
    return 0


if __name__ == "__main__":
    sys.exit(main())

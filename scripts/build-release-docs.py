#!/usr/bin/env python3
"""Build the release documentation set, NativeOIDC-Docs-<version>.zip.

Usage:
  build-release-docs.py <version> <commit-sha> --rustc-version "<rustc -V>"
      [--repo-root DIR] [--rust-doc DIR] [--output-dir DIR]

The set is the transitive closure of relative Markdown links from
docs/README.md (the documentation index), plus companion files that a
linked file needs next to it, the `cargo doc` output under rust-api/, and
a generated entry page, README.md. Repository files are stored under repo/
at their repository-relative paths with their exact bytes, so every
relative link that works in the repository also works in the unzipped
archive, with no checkout needed.

A link whose target does not exist, a link that leaves the repository, a
root-relative link, and a directory link with no included file beneath it
all fail the build: the set never ships a link that cannot be followed.

The zip is deterministic: entries are sorted, timestamps are pinned to
2026-01-01 00:00:00 (the Apple packaging pin), and permissions are fixed.
The archive is assembled twice from the same inputs and the two results
must hash identically (in-run self-test). The script prints a summary and
appends it to GITHUB_STEP_SUMMARY when that is set.

Standard library only, so it runs on every CI host and locally.
"""
import argparse
import hashlib
import io
import os
import re
import sys
import urllib.parse
import zipfile

INDEX = "docs/README.md"
# Archive layout: the generated entry page, the rustdoc tree, and the
# repository files under their own directory, so the project README.md and
# the entry README.md do not collide, and rustdoc's own src/ stays apart
# from the repository's src/.
ENTRY = "README.md"
RUST_API_DIR = "rust-api"
REPO_DIR = "repo"

# Files that must ship next to a linked file, although no link names them.
COMPANIONS = {
    "native/windows/dotnet/NativeOidcWindows.cs": ("native/windows/dotnet/NativeOidcWindows.csproj",),
}

# Language guides that are still being written on their own issues. Each is
# included when the release commit has it; the entry records which are
# present, and names the tracking issue for one that is absent.
OPTIONAL_GUIDES = (
    ("docs/swift-guide.md", "Swift user guide (iOS, iPadOS, macOS)", 85),
    ("docs/windows-dotnet-guide.md", "Windows .NET user guide", 88),
)

# The entry's platform guide list, in order. An OPTIONAL_GUIDES path takes
# its title from there (label None).
PLATFORM_GUIDES = (
    ("docs/swift-xcframework.md", "Swift: XCFramework and Swift Package Manager installation"),
    ("docs/swift-guide.md", None),
    ("docs/kotlin-guide.md", "Kotlin/Android user guide"),
    ("docs/windows-cpp-guide.md", "Windows C++ user guide"),
    ("docs/windows-dotnet-guide.md", None),
    ("native/windows/README.md", "Windows package usage (C++ and .NET)"),
)

# Example code, listed on the entry page when the guides link it.
EXAMPLE_PREFIXES = ("examples/", "native/consumer-smoke/")

# Rust API reference entry inside the rustdoc tree.
RUST_CRATE_INDEX = "native_oidc/index.html"
# cargo's build lock inside target/doc; not documentation.
RUST_DOC_EXCLUDE = (".lock",)

ZIP_DATE_TIME = (2026, 1, 1, 0, 0, 0)
FILE_MODE = 0o644
DIR_MODE = 0o755

_FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
_CODE_SPAN = re.compile(r"(`+)(?:(?!\1).)+?\1", re.S)
_SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
# A reference-style link definition: `[label]: destination "title"`.
# The destination may also start on the next line (GFM allows one line
# break after the colon), so it is optional here.
_DEFINITION = re.compile(r"^ {0,3}\[(?:[^\]\\]|\\.)+\]:[ \t]*(<[^>]*>|\S+)?")
_DESTINATION = re.compile(r"^[ \t]*(<[^>]*>|\S+)")


class BuildError(Exception):
    """A problem that makes the documentation set unshippable."""


def markdown_links(text):
    """Yield (line_number, target) for each link or image in `text`.

    Inline links and images, and reference-style link definitions (the
    `[label]: target` lines that `[text][label]` uses), are reported.

    Fenced code blocks and inline code spans are skipped, so code samples
    that look like links are not followed. Inline links are found per
    paragraph (the lines between blank lines, fences and definitions), so
    a link whose text or destination continues on the next line is found
    too; the reported line is the one where its destination starts.
    """
    paragraph = []
    fence = None
    # Set after a definition whose destination is on the next line.
    split_definition = False
    for number, line in enumerate(text.splitlines(), start=1):
        if not line.strip() or _FENCE.match(line) or _DEFINITION.match(line):
            yield from _inline_links(paragraph)
            paragraph.clear()
        if split_definition:
            split_definition = False
            # A code fence ends the incomplete definition instead of being
            # read as its destination.
            destination = None if _FENCE.match(line) else _DESTINATION.match(line)
            if destination:
                target = _definition_target(destination.group(1))
                if target:
                    yield number, target
                continue
        match = _FENCE.match(line)
        if fence is not None:
            if match and match.group(1)[0] == fence[0] and len(match.group(1)) >= len(fence) \
                    and not line[match.end():].strip():
                fence = None
            continue
        if match:
            fence = match.group(1)
            continue
        definition = _DEFINITION.match(line)
        if definition:
            if definition.group(1) is None:
                split_definition = True
                continue
            target = _definition_target(definition.group(1))
            if target:
                yield number, target
            continue
        if line.strip():
            paragraph.append((number, line))
    yield from _inline_links(paragraph)


def _inline_links(paragraph):
    """Yield (line_number, target) for the inline links in `paragraph`."""
    if not paragraph:
        return
    first = paragraph[0][0]
    text = "\n".join(line for _, line in paragraph)
    # Blank out code spans (which may span lines), keeping line breaks so
    # offsets still map to line numbers.
    text = _CODE_SPAN.sub(lambda span: re.sub(r"[^\n]", " ", span.group(0)), text)
    position = 0
    while True:
        start = text.find("](", position)
        if start < 0:
            break
        target, end, target_start = _link_target(text, start + 2)
        position = end
        if target is not None:
            yield first + text.count("\n", 0, target_start), target


def _definition_target(destination):
    """Return a definition's destination without its angle brackets."""
    if destination.startswith("<") and destination.endswith(">"):
        return destination[1:-1]
    return destination


def _link_target(line, index):
    """Parse a link destination starting at `index`.

    Return (target, end, target_start). Whitespace, including a line
    break, may come before the destination.
    """
    while index < len(line) and line[index] in " \t\n":
        index += 1
    if index < len(line) and line[index] == "<":
        close = line.find(">", index)
        if close < 0 or "\n" in line[index:close]:
            return None, index, index
        return line[index + 1:close], close + 1, index
    depth = 0
    start = index
    while index < len(line):
        char = line[index]
        if char == "\\" and index + 1 < len(line):
            index += 2
            continue
        if char == "(":
            depth += 1
        elif char == ")":
            if depth == 0:
                break
            depth -= 1
        elif char in " \t\n":
            break
        index += 1
    target = line[start:index]
    return (target or None), index, start


def resolve_link(source, target):
    """Return the repository path `target` names from `source`, or None.

    None means the link is not a file link (external URL, mail address, or
    an anchor in the same file). Raises BuildError for links that cannot be
    followed inside the archive.
    """
    if _SCHEME.match(target) or target.startswith("//"):
        return None
    path = target.split("#", 1)[0].split("?", 1)[0]
    if not path:
        return None
    path = urllib.parse.unquote(path)
    if path.startswith("/"):
        raise BuildError(f"{source}: root-relative link `{target}` is not supported")
    joined = os.path.normpath(os.path.join(os.path.dirname(source), path)).replace(os.sep, "/")
    if joined == ".." or joined.startswith("../"):
        raise BuildError(f"{source}: link `{target}` leaves the repository")
    return "" if joined == "." else joined


def link_closure(repo_root, index=INDEX):
    """Return the sorted repository paths reachable from `index`.

    Markdown files are followed transitively; other files are included but
    not parsed. Raises BuildError listing every link that cannot be followed.
    """
    errors = []
    included = set()
    directory_links = []
    pending = [index]
    if not os.path.isfile(os.path.join(repo_root, index)):
        raise BuildError(f"documentation index {index} does not exist")
    while pending:
        path = pending.pop()
        if path in included:
            continue
        included.add(path)
        for companion in COMPANIONS.get(path, ()):
            if not os.path.isfile(os.path.join(repo_root, companion)):
                errors.append(f"{path}: companion file {companion} does not exist")
            else:
                pending.append(companion)
        if not path.endswith(".md"):
            continue
        with open(os.path.join(repo_root, path), encoding="utf-8") as handle:
            text = handle.read()
        for line, target in markdown_links(text):
            try:
                resolved = resolve_link(path, target)
            except BuildError as error:
                errors.append(f"{error} (line {line})")
                continue
            if resolved is None:
                continue
            full = os.path.join(repo_root, resolved)
            if os.path.isdir(full):
                directory_links.append((path, line, target, resolved))
            elif os.path.isfile(full):
                pending.append(resolved)
            else:
                errors.append(f"{path}:{line}: link `{target}` names {resolved}, which does not exist")
    for path, line, target, directory in directory_links:
        prefix = directory + "/" if directory else ""
        if not any(item.startswith(prefix) for item in included):
            errors.append(f"{path}:{line}: directory link `{target}` has no included file beneath it")
    if errors:
        raise BuildError("\n".join(sorted(set(errors))))
    return sorted(included)


def rust_doc_files(rust_doc):
    """Return the sorted relative paths of the rustdoc tree."""
    if not os.path.isfile(os.path.join(rust_doc, RUST_CRATE_INDEX)):
        raise BuildError(f"{rust_doc}/{RUST_CRATE_INDEX} does not exist; run `cargo doc --locked --no-deps` first")
    files = []
    for directory, subdirectories, names in os.walk(rust_doc):
        subdirectories.sort()
        for name in names:
            relative = os.path.relpath(os.path.join(directory, name), rust_doc).replace(os.sep, "/")
            if relative not in RUST_DOC_EXCLUDE:
                files.append(relative)
    return sorted(files)


def guide_presence(repo_files):
    """Return [(path, title, issue, present)] for OPTIONAL_GUIDES."""
    present = set(repo_files)
    return [(path, title, issue, path in present) for path, title, issue in OPTIONAL_GUIDES]


def archive_inventory(repo_files, rust_files):
    """Return the sorted archive paths (relative to the top directory)."""
    return sorted(
        [ENTRY]
        + [f"{REPO_DIR}/{path}" for path in repo_files]
        + [f"{RUST_API_DIR}/{path}" for path in rust_files]
    )


def entry_markdown(version, commit_sha, rustc_version, repo_files, rust_files):
    """Return the generated entry page."""
    guides = guide_presence(repo_files)
    present = set(repo_files)

    def link(path, label):
        return f"[{label}]({REPO_DIR}/{path})" if path in present else None

    lines = [
        f"# NativeOIDC {version} documentation",
        "",
        f"- Release version: `{version}`",
        f"- Commit: `{commit_sha}`",
        f"- Rust API reference built with: `{rustc_version}`",
        "",
        "Every file in this set is taken from the commit above, so it matches the",
        "packages of the same release. Relative links work inside the unzipped",
        "directory; no repository checkout is needed.",
        "",
        "## Start here",
        "",
        f"- [Documentation index]({REPO_DIR}/docs/README.md)",
        f"- [Shared user guide]({REPO_DIR}/docs/user-guide.md)",
        f"- [Public API coverage matrix]({REPO_DIR}/docs/api-coverage.md)",
        f"- [Project README]({REPO_DIR}/README.md)",
        "",
        "## Platform guides",
        "",
    ]
    optional = {path: (title, issue, is_present) for path, title, issue, is_present in guides}
    for path, label in PLATFORM_GUIDES:
        if path in optional:
            title, issue, is_present = optional[path]
            if is_present:
                lines.append(f"- [{title}]({REPO_DIR}/{path})")
            else:
                lines.append(
                    f"- {title}: not in this release "
                    f"(tracked in https://github.com/ProjectFlora/NativeOIDC/issues/{issue})"
                )
        else:
            item = link(path, label)
            if item:
                lines.append(f"- {item}")
    lines += [
        "",
        "## API references",
        "",
        f"- [Rust API reference]({RUST_API_DIR}/{RUST_CRATE_INDEX}) (rustdoc, built on Linux; "
        "its `ffi` module documents the UniFFI surface used by Swift and Kotlin)",
    ]
    for path, label in (
        ("docs/windows-api.md", "Windows ABI and .NET reference"),
        ("native/windows/include/native_oidc_windows.h", "C header"),
        ("native/windows/dotnet/NativeOidcWindows.cs", "C# wrapper"),
    ):
        item = link(path, label)
        if item:
            lines.append(f"- {item}")
    examples = [path for path in repo_files if path.startswith(EXAMPLE_PREFIXES)]
    if examples:
        lines += ["", "## Examples", "", "Example sources the guides refer to.", ""]
        lines += [f"- [{path}]({REPO_DIR}/{path})" for path in examples]
    lines += [
        "",
        "## Guide presence",
        "",
        "| Guide | Present |",
        "| --- | --- |",
    ]
    for path, title, issue, is_present in guides:
        status = "yes" if is_present else f"no (#{issue})"
        lines.append(f"| `{path}` | {status} |")
    inventory = archive_inventory(repo_files, rust_files)
    lines += [
        "",
        "## Files",
        "",
        f"<details><summary>{len(inventory)} files</summary>",
        "",
    ]
    lines += [f"- `{path}`" for path in inventory]
    lines += ["", "</details>", ""]
    return "\n".join(lines)


def archive_bytes(top, entries):
    """Return a deterministic zip of `entries` ({path: bytes}) under `top`/."""
    buffer = io.BytesIO()
    directories = set()
    for path in entries:
        parts = path.split("/")[:-1]
        for depth in range(1, len(parts) + 1):
            directories.add("/".join(parts[:depth]))
    names = [(f"{top}/", None)]
    names += [(f"{top}/{directory}/", None) for directory in directories]
    names += [(f"{top}/{path}", data) for path, data in entries.items()]
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, data in sorted(names):
            info = zipfile.ZipInfo(name, date_time=ZIP_DATE_TIME)
            info.create_system = 3
            if data is None:
                info.external_attr = (0o040000 | DIR_MODE) << 16 | 0x10
                archive.writestr(info, b"")
            else:
                info.external_attr = (0o100000 | FILE_MODE) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, data, compresslevel=9)
    return buffer.getvalue()


def collect(version, commit_sha, rustc_version, repo_root, rust_doc):
    """Return ({archive path: bytes}, repo_files, rust_files)."""
    repo_files = link_closure(repo_root)
    rust_files = rust_doc_files(rust_doc)
    entries = {}
    for path in repo_files:
        with open(os.path.join(repo_root, path), "rb") as handle:
            entries[f"{REPO_DIR}/{path}"] = handle.read()
    for path in rust_files:
        with open(os.path.join(rust_doc, path), "rb") as handle:
            entries[f"{RUST_API_DIR}/{path}"] = handle.read()
    entry = entry_markdown(version, commit_sha, rustc_version, repo_files, rust_files)
    entries[ENTRY] = entry.encode("utf-8")
    return entries, repo_files, rust_files


def build(version, commit_sha, rustc_version, repo_root, rust_doc, output_dir):
    """Write the archive and return a summary dict."""
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?", version):
        raise BuildError(f"version `{version}` is not MAJOR.MINOR.PATCH[-suffix]")
    if not re.fullmatch(r"[0-9a-f]{40}", commit_sha):
        raise BuildError(f"commit SHA `{commit_sha}` is not a full 40-character SHA-1")
    if not rustc_version.strip():
        raise BuildError("rustc version is empty")
    top = f"NativeOIDC-Docs-{version}"
    first = archive_bytes(top, collect(version, commit_sha, rustc_version, repo_root, rust_doc)[0])
    entries, repo_files, rust_files = collect(version, commit_sha, rustc_version, repo_root, rust_doc)
    second = archive_bytes(top, entries)
    first_hash = hashlib.sha256(first).hexdigest()
    second_hash = hashlib.sha256(second).hexdigest()
    if first_hash != second_hash:
        raise BuildError(f"self-test failed: two assemblies differ ({first_hash} != {second_hash})")
    os.makedirs(output_dir, exist_ok=True)
    output = os.path.join(output_dir, f"{top}.zip")
    with open(output, "wb") as handle:
        handle.write(second)
    return {
        "archive": output,
        "sha256": second_hash,
        "size": len(second),
        "files": len(entries),
        "repo_files": len(repo_files),
        "rust_files": len(rust_files),
        "guides": guide_presence(repo_files),
    }


def summary_markdown(summary):
    lines = [
        "#### Release documentation set",
        "",
        f"- Archive: `{os.path.basename(summary['archive'])}` ({summary['size']} bytes)",
        f"- SHA-256 (self-test, two identical assemblies): `{summary['sha256']}`",
        f"- Files: {summary['files']} ({summary['repo_files']} from the repository, "
        f"{summary['rust_files']} rustdoc, 1 entry)",
    ]
    for path, title, issue, present in summary["guides"]:
        lines.append(f"- {title} (`{path}`): {'present' if present else f'absent, tracked in #{issue}'}")
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("version")
    parser.add_argument("commit_sha")
    parser.add_argument("--rustc-version", required=True)
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--rust-doc", default=None, help="rustdoc output (default: <repo-root>/target/doc)")
    parser.add_argument("--output-dir", default="dist")
    args = parser.parse_args(argv)
    rust_doc = args.rust_doc or os.path.join(args.repo_root, "target", "doc")
    try:
        summary = build(args.version, args.commit_sha, args.rustc_version, args.repo_root, rust_doc, args.output_dir)
    except BuildError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    report = summary_markdown(summary)
    print(report)
    step_summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if step_summary:
        with open(step_summary, "a", encoding="utf-8") as handle:
            handle.write(report + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())

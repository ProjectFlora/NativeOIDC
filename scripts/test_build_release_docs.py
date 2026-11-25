"""Tests for scripts/build-release-docs.py.

Each test builds a small repository tree and rustdoc tree in a temporary
directory. The CI Tests job also builds the real set from the checkout
and validates it with scripts/check-release-docs.py.

Run with: python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import hashlib
import importlib.util
import io
import os
import shutil
import tempfile
import unittest
import unittest.mock
import zipfile

SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("build_release_docs", os.path.join(SCRIPTS_DIR, "build-release-docs.py"))
build_release_docs = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(build_release_docs)

SHA = "0123456789abcdef0123456789abcdef01234567"
RUSTC = "rustc 1.100.0 (fixture)"

INDEX = """# Documentation

[Guide](guide.md), [anchored](guide.md#section), [same file](#top),
[external](https://example.com/x.md), [mail](mailto:a@example.com),
[spaced](with%20space.md), ![image](images/logo.png "Logo"),
[angle](<angle target.md>), [Windows](../native/windows/),
[Windows header](../native/windows/include/native_oidc_windows.h),
[C# wrapper](../native/windows/dotnet/NativeOidcWindows.cs).

Inline code is not a link: `[x](not-a-file.md)`.

```markdown
[fenced](fenced-not-a-file.md)
```

~~~~
[tilde fenced](also-not-a-file.md)
```
[still fenced](still-not-a-file.md)
~~~~
"""

GUIDE = """# Guide

See [the example](../examples/demo.rs) and [the source](../src/lib.rs).
"""


def write(root, path, data):
    full = os.path.join(root, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    mode = "wb" if isinstance(data, bytes) else "w"
    with open(full, mode, **({} if isinstance(data, bytes) else {"encoding": "utf-8", "newline": ""})) as handle:
        handle.write(data)


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.temp)
        self.repo = os.path.join(self.temp, "repo")
        self.rust_doc = os.path.join(self.temp, "doc")
        self.output = os.path.join(self.temp, "dist")
        files = {
            "docs/README.md": INDEX,
            "docs/guide.md": GUIDE,
            "docs/with space.md": "# Spaced\n",
            "docs/angle target.md": "# Angle\n",
            "docs/images/logo.png": b"\x89PNG fixture",
            "docs/unlinked.md": "# Not linked from the index\n",
            "examples/demo.rs": "fn main() {}\n",
            "src/lib.rs": "// [not followed](../nowhere.md)\n",
            "native/windows/include/native_oidc_windows.h": "/* header */\n",
            "native/windows/dotnet/NativeOidcWindows.cs": "// wrapper\n",
            "native/windows/dotnet/NativeOidcWindows.csproj": "<Project />\n",
            "native/windows/unlinked.txt": "not linked\n",
        }
        for path, data in files.items():
            write(self.repo, path, data)
        write(self.rust_doc, "native_oidc/index.html", "<html>crate</html>\n")
        write(self.rust_doc, "static.files/style.css", "body {}\n")
        write(self.rust_doc, ".lock", "")

    def build(self, version="1.2.3"):
        return build_release_docs.build(version, SHA, RUSTC, self.repo, self.rust_doc, self.output)

    def archive_names(self, path):
        with zipfile.ZipFile(path) as archive:
            return archive.namelist()


class LinkClosureTests(Fixture):
    def test_closure_follows_markdown_links_and_skips_code(self):
        self.assertEqual(
            build_release_docs.link_closure(self.repo),
            [
                "docs/README.md",
                "docs/angle target.md",
                "docs/guide.md",
                "docs/images/logo.png",
                "docs/with space.md",
                "examples/demo.rs",
                "native/windows/dotnet/NativeOidcWindows.cs",
                "native/windows/dotnet/NativeOidcWindows.csproj",
                "native/windows/include/native_oidc_windows.h",
                "src/lib.rs",
            ],
        )

    def assert_build_error(self, text, expected):
        write(self.repo, "docs/guide.md", GUIDE + text)
        with self.assertRaises(build_release_docs.BuildError) as raised:
            build_release_docs.link_closure(self.repo)
        self.assertIn(expected, str(raised.exception))

    def test_reference_only_link_is_included(self):
        write(self.repo, "docs/guide.md", "# Guide\n\n[Sample][sample]\n\n[sample]: ../examples/demo.rs\n")
        self.assertIn("examples/demo.rs", build_release_docs.link_closure(self.repo))

    def test_reference_only_link_split_after_colon_is_included(self):
        write(self.repo, "docs/guide.md", "# Guide\n\n[Sample][sample]\n\n[sample]:\n  ../examples/demo.rs\n")
        self.assertIn("examples/demo.rs", build_release_docs.link_closure(self.repo))

    def test_missing_reference_target_fails(self):
        self.assert_build_error("\n[gone]: gone.md\n", "link `gone.md` names docs/gone.md, which does not exist")

    def test_missing_target_fails(self):
        self.assert_build_error("[gone](gone.md)\n", "docs/guide.md:4: link `gone.md` names docs/gone.md, which does not exist")

    def test_link_leaving_the_repository_fails(self):
        self.assert_build_error("[out](../../outside.md)\n", "leaves the repository")

    def test_root_relative_link_fails(self):
        self.assert_build_error("[root](/docs/guide.md)\n", "root-relative link")

    def test_directory_link_without_included_file_fails(self):
        write(self.repo, "lonely/file.txt", "x\n")
        self.assert_build_error("[dir](../lonely/)\n", "directory link `../lonely/` has no included file beneath it")

    def test_missing_index_fails(self):
        os.remove(os.path.join(self.repo, "docs/README.md"))
        with self.assertRaises(build_release_docs.BuildError):
            build_release_docs.link_closure(self.repo)

    def test_missing_companion_fails(self):
        os.remove(os.path.join(self.repo, "native/windows/dotnet/NativeOidcWindows.csproj"))
        with self.assertRaises(build_release_docs.BuildError) as raised:
            build_release_docs.link_closure(self.repo)
        self.assertIn("companion file native/windows/dotnet/NativeOidcWindows.csproj does not exist", str(raised.exception))

    def test_every_error_is_reported(self):
        write(self.repo, "docs/guide.md", GUIDE + "[a](a.md) [b](b.md)\n")
        with self.assertRaises(build_release_docs.BuildError) as raised:
            build_release_docs.link_closure(self.repo)
        self.assertIn("docs/a.md", str(raised.exception))
        self.assertIn("docs/b.md", str(raised.exception))


class MarkdownLinkTests(unittest.TestCase):
    def test_parenthesised_target_and_title(self):
        links = list(build_release_docs.markdown_links('[a](x(1).md "title") [b]( y.md )\n'))
        self.assertEqual(links, [(1, "x(1).md"), (1, "y.md")])

    def test_reference_style_definitions(self):
        text = '[Sample][sample] and [b][]\n\n[sample]: ../examples/demo.rs\n  [b]: <b c.md> "Title"\n'
        self.assertEqual(list(build_release_docs.markdown_links(text)), [(3, "../examples/demo.rs"), (4, "b c.md")])

    def test_reference_destination_on_the_next_line(self):
        text = '[Sample][sample]\n\n[sample]:\n    ../examples/demo.rs "Title"\n[b]:\n  <b c.md>\n'
        self.assertEqual(list(build_release_docs.markdown_links(text)), [(4, "../examples/demo.rs"), (6, "b c.md")])

    def test_reference_definition_in_fence_is_skipped(self):
        self.assertEqual(list(build_release_docs.markdown_links("```\n[a]: a.md\n```\n")), [])

    def test_unclosed_fence_hides_the_rest(self):
        self.assertEqual(list(build_release_docs.markdown_links("```\n[a](a.md)\n")), [])

    def test_resolve_link(self):
        resolve = build_release_docs.resolve_link
        self.assertIsNone(resolve("docs/a.md", "https://example.com"))
        self.assertIsNone(resolve("docs/a.md", "#anchor"))
        self.assertEqual(resolve("docs/a.md", "../README.md#x"), "README.md")
        self.assertEqual(resolve("docs/a.md", "b%20c.md"), "docs/b c.md")


class BuildTests(Fixture):
    def test_archive_layout_and_bytes(self):
        summary = self.build()
        self.assertEqual(os.path.basename(summary["archive"]), "NativeOIDC-Docs-1.2.3.zip")
        names = self.archive_names(summary["archive"])
        top = "NativeOIDC-Docs-1.2.3/"
        self.assertTrue(all(name.startswith(top) for name in names))
        self.assertIn(top + "README.md", names)
        self.assertIn(top + "repo/docs/README.md", names)
        self.assertIn(top + "repo/native/windows/dotnet/NativeOidcWindows.csproj", names)
        self.assertIn(top + "rust-api/native_oidc/index.html", names)
        self.assertNotIn(top + "rust-api/.lock", names)
        self.assertNotIn(top + "repo/docs/unlinked.md", names)
        self.assertNotIn(top + "repo/native/windows/unlinked.txt", names)
        with zipfile.ZipFile(summary["archive"]) as archive:
            self.assertEqual(archive.read(top + "repo/docs/images/logo.png"), b"\x89PNG fixture")
            for info in archive.infolist():
                self.assertEqual(info.date_time, (2026, 1, 1, 0, 0, 0))
        self.assertEqual(summary["files"], 13)
        self.assertEqual(summary["repo_files"], 10)
        self.assertEqual(summary["rust_files"], 2)

    def test_builds_are_byte_identical(self):
        first = self.build()
        with open(first["archive"], "rb") as handle:
            first_bytes = handle.read()
        second = self.build()
        with open(second["archive"], "rb") as handle:
            second_bytes = handle.read()
        self.assertEqual(first_bytes, second_bytes)
        self.assertEqual(first["sha256"], hashlib.sha256(second_bytes).hexdigest())

    def test_entry_records_version_commit_and_inventory(self):
        summary = self.build()
        with zipfile.ZipFile(summary["archive"]) as archive:
            entry = archive.read("NativeOIDC-Docs-1.2.3/README.md").decode("utf-8")
            files = sorted(name[len("NativeOIDC-Docs-1.2.3/"):] for name in archive.namelist() if not name.endswith("/"))
        self.assertIn("- Release version: `1.2.3`", entry)
        self.assertIn(f"- Commit: `{SHA}`", entry)
        self.assertIn(f"- Rust API reference built with: `{RUSTC}`", entry)
        self.assertIn("[Rust API reference](rust-api/native_oidc/index.html)", entry)
        self.assertIn("[C# wrapper](repo/native/windows/dotnet/NativeOidcWindows.cs)", entry)
        self.assertIn("- [examples/demo.rs](repo/examples/demo.rs)", entry)
        for path in files:
            self.assertIn(f"- `{path}`\n", entry)
        self.assertIn(f"{len(files)} files", entry)

    def test_entry_records_optional_guide_presence(self):
        write(self.repo, "docs/guide.md", GUIDE + "[Swift](swift-guide.md)\n")
        write(self.repo, "docs/swift-guide.md", "# Swift\n")
        summary = self.build()
        with zipfile.ZipFile(summary["archive"]) as archive:
            entry = archive.read("NativeOIDC-Docs-1.2.3/README.md").decode("utf-8")
        self.assertIn("[Swift user guide (iOS, iPadOS, macOS)](repo/docs/swift-guide.md)", entry)
        self.assertIn("| `docs/swift-guide.md` | yes |", entry)
        self.assertIn("Windows .NET user guide: not in this release", entry)
        self.assertIn("| `docs/windows-dotnet-guide.md` | no (#88) |", entry)

    def test_missing_rustdoc_fails(self):
        os.remove(os.path.join(self.rust_doc, "native_oidc/index.html"))
        with self.assertRaises(build_release_docs.BuildError) as raised:
            self.build()
        self.assertIn("cargo doc", str(raised.exception))

    def test_rejects_bad_version_and_sha(self):
        with self.assertRaises(build_release_docs.BuildError):
            self.build(version="1.2")
        with self.assertRaises(build_release_docs.BuildError):
            build_release_docs.build("1.2.3", "abc123", RUSTC, self.repo, self.rust_doc, self.output)
        with self.assertRaises(build_release_docs.BuildError):
            build_release_docs.build("1.2.3", SHA, " ", self.repo, self.rust_doc, self.output)

    def test_main_writes_step_summary(self):
        summary_path = os.path.join(self.temp, "summary.md")
        previous = os.environ.get("GITHUB_STEP_SUMMARY")
        os.environ["GITHUB_STEP_SUMMARY"] = summary_path
        try:
            with unittest.mock.patch("sys.stdout", new=io.StringIO()):
                code = build_release_docs.main([
                    "1.2.3", SHA, "--rustc-version", RUSTC, "--repo-root", self.repo,
                    "--rust-doc", self.rust_doc, "--output-dir", self.output,
                ])
        finally:
            if previous is None:
                del os.environ["GITHUB_STEP_SUMMARY"]
            else:
                os.environ["GITHUB_STEP_SUMMARY"] = previous
        self.assertEqual(code, 0)
        with open(summary_path, encoding="utf-8") as handle:
            report = handle.read()
        self.assertIn("SHA-256 (self-test, two identical assemblies)", report)
        self.assertIn("Windows .NET user guide (`docs/windows-dotnet-guide.md`): absent, tracked in #88", report)


if __name__ == "__main__":
    unittest.main()

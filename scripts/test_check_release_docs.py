"""Tests for scripts/check-release-docs.py.

Each test builds a healthy documentation set with build-release-docs.py
from a small fixture tree, breaks one thing, and checks the failure.

Run with: python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import importlib.util
import io
import os
import shutil
import tempfile
import unittest
import unittest.mock
import zipfile

SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, os.path.join(SCRIPTS_DIR, filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


build_release_docs = _load("build_release_docs", "build-release-docs.py")
check_release_docs = _load("check_release_docs", "check-release-docs.py")

VERSION = "1.2.3"
SHA = "0123456789abcdef0123456789abcdef01234567"
TOP = f"NativeOIDC-Docs-{VERSION}"

REPO_FILES = {
    "README.md": "# Project\n\n[Docs](docs/README.md)\n",
    "docs/README.md": (
        "# Docs\n\n[Project](../README.md#top) [Guide](user-guide.md) [Coverage](api-coverage.md)\n"
        "[Swift install](swift-xcframework.md) [Kotlin](kotlin-guide.md) [C++](windows-cpp-guide.md)\n"
        "[ABI](windows-api.md) [Windows](../native/windows/README.md) [Example](../examples/manual_authorization.rs)\n"
    ),
    "docs/user-guide.md": "# Guide\n",
    "docs/api-coverage.md": "# Coverage\n",
    "docs/swift-xcframework.md": "# Swift install\n",
    "docs/kotlin-guide.md": "# Kotlin\n",
    "docs/windows-cpp-guide.md": "# C++\n",
    "docs/windows-api.md": "# ABI\n\n[Header](../native/windows/include/native_oidc_windows.h)\n",
    "native/windows/README.md": "# Windows\n\n[Wrapper](dotnet/NativeOidcWindows.cs)\n",
    "native/windows/include/native_oidc_windows.h": "/* header */\n",
    "native/windows/dotnet/NativeOidcWindows.cs": "// wrapper\n",
    "native/windows/dotnet/NativeOidcWindows.csproj": "<Project />\n",
    "examples/manual_authorization.rs": "fn main() {}\n",
}


def write(root, path, data):
    full = os.path.join(root, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "wb") as handle:
        handle.write(data if isinstance(data, bytes) else data.encode("utf-8"))


class CheckTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.temp)
        self.repo = os.path.join(self.temp, "repo")
        self.rust_doc = os.path.join(self.temp, "doc")
        for path, data in REPO_FILES.items():
            write(self.repo, path, data)
        write(self.rust_doc, "native_oidc/index.html", "<html></html>\n")

    def build(self):
        summary = build_release_docs.build(
            VERSION, SHA, "rustc 1.100.0", self.repo, self.rust_doc, os.path.join(self.temp, "dist"))
        return summary["archive"]

    def extract(self, archive):
        target = os.path.join(self.temp, "extracted")
        with zipfile.ZipFile(archive) as handle:
            handle.extractall(target)
        return target

    def rewrite(self, archive, change):
        """Rewrite `archive` with change({relative path: bytes}) applied."""
        with zipfile.ZipFile(archive) as handle:
            files = {
                info.filename[len(TOP) + 1:]: handle.read(info)
                for info in handle.infolist() if not info.is_dir()
            }
        change(files)
        with open(archive, "wb") as handle:
            handle.write(build_release_docs.archive_bytes(TOP, files))

    def edit_entry(self, archive, old, new):
        def change(files):
            text = files["README.md"].decode("utf-8")
            self.assertIn(old, text)
            files["README.md"] = text.replace(old, new).encode("utf-8")
        self.rewrite(archive, change)

    def errors(self, archive, **kwargs):
        return check_release_docs.check(archive, kwargs.pop("version", VERSION), kwargs.pop("sha", SHA), **kwargs)

    def assert_error(self, errors, expected):
        self.assertTrue(any(expected in error for error in errors), f"{expected!r} not in {errors}")

    def test_healthy_archive_passes(self):
        archive = self.build()
        self.assertEqual(self.errors(archive, repo_root=self.repo), [])
        extracted = self.extract(archive)
        self.assertEqual(self.errors(extracted), [])
        self.assertEqual(self.errors(os.path.join(extracted, TOP), repo_root=self.repo), [])

    def test_missing_required_file(self):
        archive = self.build()
        self.rewrite(archive, lambda files: files.pop("repo/native/windows/dotnet/NativeOidcWindows.csproj"))
        self.assert_error(self.errors(archive), "required file repo/native/windows/dotnet/NativeOidcWindows.csproj is missing")

    def test_broken_link(self):
        archive = self.build()
        self.rewrite(archive, lambda files: files.pop("repo/docs/user-guide.md"))
        self.assert_error(
            self.errors(archive),
            "repo/docs/README.md:3: link `user-guide.md` names repo/docs/user-guide.md, which is not in the archive",
        )

    def test_link_leaving_the_archive(self):
        archive = self.build()

        def change(files):
            files["repo/docs/kotlin-guide.md"] = b"[out](../../../outside.md)\n"
        self.rewrite(archive, change)
        self.assert_error(self.errors(archive), "leaves the archive")

    def test_wrong_version(self):
        archive = self.build()
        self.edit_entry(archive, "- Release version: `1.2.3`", "- Release version: `1.2.4`")
        self.assert_error(self.errors(archive), "release version line does not say `1.2.3`")

    def test_wrong_top_directory_for_version(self):
        archive = self.build()
        self.assert_error(self.errors(archive, version="1.2.4"), "is outside NativeOIDC-Docs-1.2.4/")

    def test_wrong_sha(self):
        archive = self.build()
        other = "f" * 40
        self.assert_error(self.errors(archive, sha=other), f"commit line does not say `{other}`")

    def test_inventory_mismatch(self):
        archive = self.build()

        def change(files):
            files["repo/extra.txt"] = b"extra\n"
            files.pop("rust-api/native_oidc/index.html")
        self.rewrite(archive, change)
        errors = self.errors(archive)
        self.assert_error(errors, "file list omits repo/extra.txt")
        self.assert_error(errors, "file list names rust-api/native_oidc/index.html, which is not in the archive")

    def test_optional_guide_inconsistent_with_entry(self):
        archive = self.build()

        def change(files):
            files["repo/docs/swift-guide.md"] = b"# Swift\n"
            text = files["README.md"].decode("utf-8")
            files["README.md"] = text.replace("- `repo/docs/user-guide.md`", "- `repo/docs/swift-guide.md`\n- `repo/docs/user-guide.md`").encode("utf-8")
        self.rewrite(archive, change)
        self.assert_error(self.errors(archive), "guide presence for docs/swift-guide.md is `no (#85)`, expected `yes`")

    def test_optional_guide_missing_from_archive(self):
        archive = self.build()
        write(self.repo, "docs/swift-guide.md", "# Swift\n")
        self.assert_error(
            self.errors(archive, repo_root=self.repo),
            "(docs/swift-guide.md): the checkout has it but the archive does not",
        )

    def test_repository_bytes_differ(self):
        archive = self.build()
        write(self.repo, "docs/user-guide.md", "# Guide\r\n")
        self.assert_error(self.errors(archive, repo_root=self.repo), "repo/docs/user-guide.md: differs from the repository checkout")

    def test_unreadable_archive(self):
        path = os.path.join(self.temp, "broken.zip")
        write(self.temp, "broken.zip", b"not a zip")
        self.assert_error(self.errors(path), "cannot open")

    def test_main_exit_codes(self):
        archive = self.build()
        with unittest.mock.patch("sys.stdout", new=io.StringIO()):
            self.assertEqual(check_release_docs.main([archive, "--version", VERSION, "--commit-sha", SHA]), 0)
        with unittest.mock.patch("sys.stderr", new=io.StringIO()) as stderr:
            self.assertEqual(check_release_docs.main([archive, "--version", VERSION, "--commit-sha", "f" * 40]), 1)
        self.assertIn("check(s) failed", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()

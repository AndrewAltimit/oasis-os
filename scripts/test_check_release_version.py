"""Unit tests for scripts/check-release-version.py (release tag gate).

Run: python3 -m unittest discover -s scripts -p "test_*.py"
"""

import importlib.util
import os
import shutil
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "check_release_version", os.path.join(HERE, "check-release-version.py")
)
crv = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(crv)


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


ROOT_TOML = """\
[workspace]
members = ["crates/a"]

[workspace.package]
version = "{v}"   # inherited by every member
edition = "2024"

[workspace.dependencies]
foo = {{ version = "9.9.9" }}
"""

PSP_TOML = """\
[workspace]

[package]
name = "oasis-backend-psp"
version = "{v}"

[dependencies]
bar = {{ version = "0.1.0" }}
"""


class TagParseTest(unittest.TestCase):
    def test_accepts_release_and_prerelease_tags(self):
        self.assertEqual(crv.tag_version("v1.5.0"), "1.5.0")
        self.assertEqual(crv.tag_version("refs/tags/v1.5.0"), "1.5.0")
        self.assertEqual(crv.tag_version("v2.0.0-rc.1"), "2.0.0-rc.1")

    def test_rejects_non_release_refs(self):
        for bad in ("1.5.0", "v1.5", "refs/heads/main", "release-1.5.0", ""):
            self.assertIsNone(crv.tag_version(bad), bad)


class SectionVersionTest(unittest.TestCase):
    def test_reads_only_the_named_section(self):
        text = ROOT_TOML.format(v="1.4.0")
        self.assertEqual(crv.section_version(text, "workspace.package"), "1.4.0")
        self.assertIsNone(crv.section_version(text, "package"))

    def test_inherited_version_is_not_a_literal(self):
        text = '[package]\nname = "x"\nversion.workspace = true\n'
        self.assertIsNone(crv.section_version(text, "package"))


class CheckTest(unittest.TestCase):
    def _tree(self, workspace, psp):
        root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, root, ignore_errors=True)
        _write(os.path.join(root, "Cargo.toml"), ROOT_TOML.format(v=workspace))
        _write(
            os.path.join(root, "crates/oasis-backend-psp/Cargo.toml"),
            PSP_TOML.format(v=psp),
        )
        return root

    def test_matching_versions_pass(self):
        self.assertEqual(crv.check("v1.5.0", self._tree("1.5.0", "1.5.0")), [])

    def test_stale_workspace_version_fails(self):
        # The v1.5.0-tagged-on-1.4.0 release this gate exists for.
        errors = crv.check("refs/tags/v1.5.0", self._tree("1.4.0", "1.5.0"))
        self.assertEqual(len(errors), 1)
        self.assertIn("1.4.0 does not match tag v1.5.0", errors[0])

    def test_drifted_standalone_crate_fails(self):
        errors = crv.check("v1.5.0", self._tree("1.5.0", "1.4.0"))
        self.assertEqual(len(errors), 1)
        self.assertIn("oasis-backend-psp", errors[0])

    def test_bad_tag_is_an_error(self):
        self.assertTrue(crv.check("main", self._tree("1.5.0", "1.5.0")))

    def test_repository_manifests_are_readable(self):
        # Every manifest the gate reads exists and has a literal version,
        # so a real tag only fails on a genuine mismatch.
        for where, version in crv.collect(crv.ROOT):
            self.assertIsNotNone(version, where)
        self.assertEqual(len(crv.collect(crv.ROOT)), 1 + len(crv.SHIPPED_STANDALONE))


if __name__ == "__main__":
    unittest.main()

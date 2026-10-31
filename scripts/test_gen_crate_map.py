"""Unit tests for scripts/gen-crate-map.py (the site's crate-map data).

Run: python3 -m unittest discover -s scripts -p "test_*.py"
"""

import importlib.util
import os
import shutil
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "gen_crate_map", os.path.join(HERE, "gen-crate-map.py")
)
gcm = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gcm)


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


class TomlFallbackTest(unittest.TestCase):
    """The fallback reader (Python < 3.11) must agree with tomllib."""

    SAMPLE = '''
# comment
[package]
name = "demo"            # trailing comment
version.workspace = true
description = "A \\"quoted\\" thing \\u00e9"

[features]
default = ["a"]
a = [
    "dep:oasis-x",   # why
    "oasis-y/feat",
]

[dependencies]
oasis-x = { workspace = true, optional = true }
oasis-y = { path = "../oasis-y", default-features = false, features = ["f"] }

[target.'cfg(target_arch = "wasm32")'.dependencies]
oasis-z = { version = '1' }

[[bin]]
name = "one"

[[bin]]
name = "two"
'''

    def test_sample_parses(self):
        doc = gcm.parse_toml_fallback(self.SAMPLE)
        self.assertEqual(doc["package"]["name"], "demo")
        self.assertEqual(doc["package"]["version"], {"workspace": True})
        self.assertEqual(doc["package"]["description"], 'A "quoted" thing é')
        self.assertEqual(doc["features"]["a"], ["dep:oasis-x", "oasis-y/feat"])
        self.assertTrue(doc["dependencies"]["oasis-x"]["optional"])
        self.assertIn("oasis-z", doc["target"]['cfg(target_arch = "wasm32")']["dependencies"])
        self.assertEqual([b["name"] for b in doc["bin"]], ["one", "two"])

    @unittest.skipIf(gcm._tomllib is None, "tomllib needs Python 3.11+")
    def test_matches_tomllib_on_every_manifest(self):
        paths = [os.path.join(gcm.ROOT, "Cargo.toml")]
        crates = os.path.join(gcm.ROOT, "crates")
        paths += [os.path.join(crates, d, "Cargo.toml") for d in sorted(os.listdir(crates))]
        for p in paths:
            if not os.path.isfile(p):
                continue
            with open(p, encoding="utf-8") as f:
                text = f.read()
            with self.subTest(manifest=os.path.relpath(p, gcm.ROOT)):
                self.assertEqual(gcm.parse_toml_fallback(text), gcm._tomllib.loads(text))


class ScanSyntheticTest(unittest.TestCase):
    """Dependency classification on a tiny fake workspace."""

    def setUp(self):
        self.root = tempfile.mkdtemp()
        _write(os.path.join(self.root, "Cargo.toml"), '''
[workspace]
members = ["crates/oasis-a", "crates/oasis-b"]
exclude = ["crates/oasis-c"]
[workspace.package]
version = "9.9.9"
''')
        _write(os.path.join(self.root, "crates/oasis-a/Cargo.toml"), '''
[package]
name = "oasis-a"
description = "The base."
''')
        _write(os.path.join(self.root, "crates/oasis-a/src/lib.rs"), "pub fn a() {}\n\n")
        _write(os.path.join(self.root, "crates/oasis-b/Cargo.toml"), '''
[package]
name = "oasis-b"
[features]
extra = ["dep:oasis-c"]
[dependencies]
oasis-a.workspace = true
oasis-c = { path = "../oasis-c", optional = true }
serde = "1"
[dev-dependencies]
oasis-a = { workspace = true, features = ["testing"] }
oasis-d = { path = "../oasis-d" }
''')
        _write(os.path.join(self.root, "crates/oasis-b/src/lib.rs"), "//! Bee crate.\n")
        _write(os.path.join(self.root, "crates/oasis-c/Cargo.toml"), '''
[workspace]
[package]
name = "oasis-c"
''')
        _write(os.path.join(self.root, "crates/oasis-c/src/main.rs"), "//! Sea.\nfn main() {}")
        _write(os.path.join(self.root, "crates/oasis-d/Cargo.toml"), '''
[package]
name = "oasis-d"
[target.'cfg(unix)'.dependencies]
oasis-a = { path = "../oasis-a" }
''')
        # A nested package is its own crate: its lines are not counted.
        _write(os.path.join(self.root, "crates/oasis-a/fuzz/Cargo.toml"), "[package]\nname='f'\n")
        _write(os.path.join(self.root, "crates/oasis-a/fuzz/x.rs"), "1\n2\n3\n")
        self.crates = {c["name"]: c for c in gcm.scan(self.root)}

    def tearDown(self):
        shutil.rmtree(self.root)

    def test_membership_and_target(self):
        self.assertEqual(self.crates["oasis-a"]["membership"], "member")
        self.assertEqual(self.crates["oasis-c"]["membership"], "excluded")
        self.assertEqual(self.crates["oasis-d"]["membership"], "standalone")
        self.assertEqual(self.crates["oasis-a"]["target"], "workspace")
        self.assertEqual(self.crates["oasis-c"]["target"], "psp")

    def test_dependency_kinds(self):
        deps = {d["name"]: d for d in self.crates["oasis-b"]["deps"]}
        self.assertEqual(set(deps), {"oasis-a", "oasis-c", "oasis-d"})  # not serde
        self.assertEqual(deps["oasis-a"]["kind"], "normal")  # normal wins over dev
        self.assertEqual(deps["oasis-c"]["kind"], "optional")
        self.assertEqual(deps["oasis-c"]["features"], ["extra"])
        self.assertEqual(deps["oasis-d"]["kind"], "dev")
        d_deps = self.crates["oasis-d"]["deps"]
        self.assertEqual(d_deps, [{"name": "oasis-a", "kind": "normal", "cfg": "cfg(unix)"}])

    def test_descriptions_fall_back_to_crate_docs(self):
        self.assertEqual(self.crates["oasis-a"]["description"], "The base.")
        self.assertEqual(self.crates["oasis-b"]["description"], "Bee crate.")
        self.assertEqual(self.crates["oasis-c"]["description"], "Sea.")

    def test_line_counts(self):
        self.assertEqual(self.crates["oasis-a"]["loc"], 2)
        self.assertEqual(self.crates["oasis-a"]["files"], 1)
        self.assertEqual(self.crates["oasis-c"]["loc"], 2)  # no trailing newline
        self.assertEqual(self.crates["oasis-c"]["bins"], ["oasis-c"])

    def test_build_rejects_a_crate_without_a_layer(self):
        with self.assertRaises(SystemExit) as cm:
            gcm.build(self.root)
        self.assertIn("has no layer", str(cm.exception))


class RealWorkspaceTest(unittest.TestCase):
    """The real workspace, and the committed site/data/crates.json."""

    @classmethod
    def setUpClass(cls):
        cls.doc = gcm.build()
        cls.crates = {c["name"]: c for c in cls.doc["crates"]}

    def test_every_crate_directory_is_mapped(self):
        dirs = [
            d
            for d in os.listdir(os.path.join(gcm.ROOT, "crates"))
            if os.path.isfile(os.path.join(gcm.ROOT, "crates", d, "Cargo.toml"))
        ]
        self.assertEqual(len(self.crates), len(dirs))
        for c in self.crates.values():
            self.assertTrue(c["layer"], c["name"])
            self.assertTrue(c["description"], c["name"])

    def test_known_shape(self):
        self.assertEqual(self.crates["oasis-types"]["deps"], [])
        core = {d["name"]: d["kind"] for d in self.crates["oasis-core"]["deps"]}
        self.assertEqual(core.get("oasis-types"), "normal")
        self.assertNotIn("oasis-app-tv-guide", core)  # core ships no app crate
        self.assertEqual(self.crates["oasis-backend-psp"]["membership"], "excluded")
        self.assertEqual(self.crates["oasis-plugin-psp"]["membership"], "standalone")
        self.assertNotIn(
            "oasis-core", [d["name"] for d in self.crates["oasis-plugin-psp"]["deps"]]
        )

    def test_walk_steps_name_real_crates_and_code(self):
        for walk in self.doc["walks"]:
            self.assertGreaterEqual(len(walk["steps"]), 3, walk["id"])
            for step in walk["steps"]:
                self.assertTrue(step["crates"])
                for ref in step["code"]:
                    path, _, items = ref.partition(": ")
                    full = os.path.join(gcm.ROOT, path)
                    with self.subTest(walk=walk["id"], ref=ref):
                        self.assertTrue(os.path.exists(full), path)
                        if not items or not os.path.isfile(full):
                            continue
                        # "Type::method, other" -> each last segment must
                        # appear in the file the step points at.
                        with open(full, encoding="utf-8") as f:
                            text = f.read()
                        for item in items.split(","):
                            name = item.strip().split("::")[-1].split(".")[-1]
                            self.assertIn(name, text, "%s not in %s" % (name, path))

    def test_committed_json_is_current(self):
        diffs = gcm.check()
        self.assertEqual(
            diffs, [], "site/data/crates.json is stale; run python3 scripts/gen-crate-map.py"
        )


if __name__ == "__main__":
    unittest.main()

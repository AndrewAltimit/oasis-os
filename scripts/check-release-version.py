#!/usr/bin/env python3
"""Fail a release whose tag does not match the Cargo version it ships.

v1.5.0 was once tagged while `[workspace.package] version` still said
1.4.0, so every binary in that release reported the old version. The
`create-release` job in .github/workflows/main-ci.yml runs this before it
publishes anything.

It checks the pushed tag (`vX.Y.Z[-pre]`) against

- `version` in `[workspace.package]` of the root Cargo.toml (every
  workspace member inherits it), and
- `version` in `[package]` of the standalone PSP crates whose builds are
  release artifacts (they are outside the workspace, so they carry their
  own copy and drift silently).

Usage:
    python3 scripts/check-release-version.py v1.5.0
    python3 scripts/check-release-version.py refs/tags/v1.5.0
    python3 scripts/check-release-version.py            # tag from $GITHUB_REF

Exits 0 when everything matches, 1 on a mismatch, 2 on bad input. To fix a
mismatch, bump the versions in a commit, then move or re-create the tag on
that commit. Stdlib only (the runner's python3 may predate tomllib).
"""

from __future__ import annotations

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Standalone (non-workspace) crates whose outputs ship in the release.
SHIPPED_STANDALONE = (
    "crates/oasis-backend-psp/Cargo.toml",  # EBOOT.PBP
    "crates/oasis-plugin-psp/Cargo.toml",  # oasis-overlay.prx
    "crates/oasis-recovery-psp/Cargo.toml",  # recovery EBOOT.PBP
)

TAG_RE = re.compile(r"^(?:refs/tags/)?v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$")


def tag_version(tag: str) -> str | None:
    """`v1.2.3` / `refs/tags/v1.2.3-rc.1` -> `1.2.3` / `1.2.3-rc.1`."""
    m = TAG_RE.match(tag.strip())
    return m.group(1) if m else None


def section_version(text: str, section: str) -> str | None:
    """The `version = "..."` string directly inside `[section]`, if any."""
    current = None
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if line.startswith("["):
            current = line.strip("[]").strip()
            continue
        if current != section:
            continue
        m = re.match(r'^version\s*=\s*"([^"]+)"$', line)
        if m:
            return m.group(1)
    return None


def collect(root: str) -> list[tuple[str, str | None]]:
    """(manifest, version) for the workspace and each shipped standalone crate."""
    found = []
    with open(os.path.join(root, "Cargo.toml"), encoding="utf-8") as f:
        workspace = section_version(f.read(), "workspace.package")
    found.append(("Cargo.toml [workspace.package]", workspace))
    for rel in SHIPPED_STANDALONE:
        path = os.path.join(root, rel)
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as f:
            found.append((f"{rel} [package]", section_version(f.read(), "package")))
    return found


def check(tag: str, root: str = ROOT) -> list[str]:
    """Error messages (empty when the tag and every version agree)."""
    want = tag_version(tag)
    if want is None:
        return [f"{tag!r} is not a release tag (expected vMAJOR.MINOR.PATCH[-pre])"]
    errors = []
    for where, have in collect(root):
        if have is None:
            errors.append(f"{where}: no literal `version = \"...\"` found")
        elif have != want:
            errors.append(f"{where}: version {have} does not match tag v{want}")
    return errors


def main(argv: list[str]) -> int:
    tag = argv[1] if len(argv) > 1 else os.environ.get("GITHUB_REF", "")
    if not tag:
        print("usage: check-release-version.py <tag>  (or set GITHUB_REF)", file=sys.stderr)
        return 2
    if tag_version(tag) is None:
        print(f"error: {tag!r} is not a release tag (vMAJOR.MINOR.PATCH)", file=sys.stderr)
        return 2
    errors = check(tag)
    for e in errors:
        print(f"::error::{e}")
    if errors:
        print(
            "Bump the versions above in a commit, then re-tag that commit.",
            file=sys.stderr,
        )
        return 1
    print(f"Release tag {tag} matches the Cargo versions.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

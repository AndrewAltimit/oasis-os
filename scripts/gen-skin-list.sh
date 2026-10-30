#!/usr/bin/env bash
# Regenerate the site's skin lists from site/data/skins.json.
#
# site/data/skins.json is the one hand-edited list of skins on the site
# (display name, gallery category, blurb); resolutions come from
# skins/<id>/skin.toml. The pages that list skins carry generated copies
# between `<!-- BEGIN skins:<region> -->` / `<!-- END skins:<region> -->`
# markers, so they stay plain static HTML:
#
#   site/index.html        gallery filter chips + gallery list
#   site/demo/index.html   demo skin picker
#   www/index.html         local demo skin picker
#
# The generator is the `site_skins` integration test of oasis-backend-wasm
# (so CI's `cargo test --workspace` fails when a page is stale); this script
# runs it in update mode.
#
# Usage:
#   ./scripts/gen-skin-list.sh          # rewrite the generated regions
#   ./scripts/gen-skin-list.sh --check  # only check (exit 1 if stale)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ "${1:-}" = "--check" ]; then
  exec cargo test -p oasis-backend-wasm --test site_skins
fi
OASIS_UPDATE_SITE_SKINS=1 cargo test -p oasis-backend-wasm --test site_skins
echo "==> Skin lists regenerated from site/data/skins.json"

#!/usr/bin/env python3
"""Generate site/data/crates.json, the data behind the site's crate map.

The map (site/crates.html) is drawn from this file, and this file is
drawn from the manifests: every `crates/*/Cargo.toml` is parsed directly
(not through `cargo metadata`, which cannot see the excluded
`oasis-backend-psp` or the nine standalone PSP crates). For each crate it
records the description, its layer, where it builds (workspace member,
excluded, standalone PSP), the line count of its Rust sources, and its
internal dependencies split into normal / optional (feature-gated) /
dev / build edges.

Only two things here are written by hand: LAYERS (which band of the map
a crate sits in, following the dependency graph in CLAUDE.md) and WALKS
(the guided "follow the data" tours, each step checked against the code
it names). The generator refuses to run when a crate has no layer, a
layer names a crate that does not exist, or a walk names an unknown
crate, so the map cannot silently drift from the workspace.

Usage:
    python3 scripts/gen-crate-map.py           # rewrite site/data/crates.json
    python3 scripts/gen-crate-map.py --check   # exit 1 if it is stale

`--check` ignores the line counts (they move with every commit); it fails
when a crate, description, layer, dependency edge or walk changed without
the JSON being regenerated. scripts/test_gen_crate_map.py runs the same
check, so CI's python unit-test step enforces it.

Stdlib only. Uses tomllib on Python 3.11+, and a small built-in TOML
reader (enough for Cargo manifests) on older interpreters.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys

try:  # Python 3.11+
    import tomllib as _tomllib
except ImportError:  # pragma: no cover - exercised on old runners only
    _tomllib = None

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = "site/data/crates.json"  # relative to the repo root, "/" on every OS
REPO = "https://github.com/AndrewAltimit/oasis-os"

# --------------------------------------------------------------------------
# Layers, top of the map to the bottom (the homepage architecture diagram's
# order): hosts link everything; foundation links nothing internal; the
# backends sit below the trait boundary; the standalone PSP crates last.
# --------------------------------------------------------------------------
LAYERS = [
    {
        "id": "hosts",
        "title": "Hosts",
        "blurb": "Binaries and libraries a platform links: the desktop shell, the C ABI "
        "for UE5, the PSP USB thin-client host.",
        "crates": ["oasis-app", "oasis-ffi", "oasis-usb-host"],
    },
    {
        "id": "registry",
        "title": "Registry",
        "blurb": "The built-in app bundle hosts depend on: one AppRegistry of every app crate.",
        "crates": ["oasis-apps"],
    },
    {
        "id": "coordination",
        "title": "Coordination",
        "blurb": "Dashboard, shell chrome, AppRunner, plugins, scripting, agent. Depends on "
        "the app framework but on no app crate.",
        "crates": ["oasis-core"],
    },
    {
        "id": "apps",
        "title": "Apps",
        "blurb": "The App trait and the ten app crates; each draws itself through "
        "App::draw_windowed.",
        "crates": [
            "oasis-app-core",
            "oasis-app-tv-guide",
            "oasis-app-radio",
            "oasis-app-media",
            "oasis-app-file-manager",
            "oasis-app-text-editor",
            "oasis-app-paint",
            "oasis-app-games",
            "oasis-app-calculator",
            "oasis-app-settings",
            "oasis-app-system-monitor",
        ],
    },
    {
        "id": "engines",
        "title": "Engines",
        "blurb": "The browser engine and its parser / loader crates, JavaScript, the "
        "terminal, video and audio.",
        "crates": [
            "oasis-browser",
            "oasis-html",
            "oasis-css",
            "oasis-http",
            "oasis-js",
            "oasis-terminal",
            "oasis-video",
            "oasis-audio",
        ],
    },
    {
        "id": "presentation",
        "title": "Presentation",
        "blurb": "Scene graph, widgets, window manager, skins, vector art, shader "
        "wallpapers and the software rasterizer.",
        "crates": [
            "oasis-sdi",
            "oasis-ui",
            "oasis-wm",
            "oasis-skin",
            "oasis-vector",
            "oasis-shader",
            "oasis-rasterize",
        ],
    },
    {
        "id": "services",
        "title": "Services",
        "blurb": "Virtual file system, platform services, networking, the MCP server, "
        "translations.",
        "crates": ["oasis-vfs", "oasis-platform", "oasis-net", "oasis-mcp", "oasis-i18n"],
    },
    {
        "id": "foundation",
        "title": "Foundation",
        "blurb": "Colors, input events, geometry, errors and the backend traits every "
        "platform implements; plus the test doubles.",
        "crates": ["oasis-types", "oasis-test-backend"],
    },
    {
        "id": "backends",
        "title": "Backends",
        "blurb": "Below the trait boundary: each implements the oasis-types traits for "
        "one platform.",
        "crates": [
            "oasis-backend-sdl",
            "oasis-backend-wasm",
            "oasis-backend-ue5",
            "oasis-backend-psp",
        ],
    },
    {
        "id": "psp",
        "title": "PSP tools",
        "blurb": "Standalone mipsel-sony-psp crates built one by one with cargo psp: the "
        "overlay PRX, recovery, dev loop, Media Engine and USB research.",
        "crates": [
            "oasis-plugin-psp",
            "oasis-recovery-psp",
            "oasis-devloop-psp",
            "oasis-me-boot",
            "oasis-prx-decrypt-psp",
            "oasis-usb-client-psp",
            "oasis-usb-debug-psp",
            "oasis-usb-trace-psp",
            "oasis-usb-vbus-psp",
        ],
    },
]

# --------------------------------------------------------------------------
# Walks: the path data takes through the crates. Every step names the
# crates it lights up and the code it describes (path + item), so a
# reader can check it; keep them true when the code moves.
# --------------------------------------------------------------------------
WALKS = [
    {
        "id": "frame",
        "title": "A frame",
        "blurb": "From the shell's main loop to pixels on a desktop window, a canvas, a "
        "UE5 texture or a PSP screen.",
        "steps": [
            {
                "title": "The shell starts a frame",
                "crates": ["oasis-app"],
                "text": "Shell::render calls draw_frame, which clears the back buffer to the "
                "skin's background color and, when the skin has a shader wallpaper, "
                "shades it and blits it first.",
                "code": [
                    "crates/oasis-app/src/shell.rs: Shell::render",
                    "crates/oasis-app/src/shell.rs: draw_frame",
                ],
            },
            {
                "title": "Shader wallpaper",
                "crates": ["oasis-shader", "oasis-backend-sdl"],
                "text": "The SDL shader bridge renders the skin's Shadertoy-style wallpaper "
                "(GPU when it can, the CPU implementation otherwise) and blits it under "
                "everything else.",
                "code": [
                    "crates/oasis-backend-sdl/src/shader_bridge.rs: render_and_blit",
                ],
            },
            {
                "title": "Scene objects",
                "crates": ["oasis-sdi", "oasis-core"],
                "text": "oasis-core keeps the dashboard, status bar and taskbar as named "
                "objects in the SDI registry; SdiRegistry::draw paints them in z-order "
                "(or as a base and overlay layer around vector icons).",
                "code": [
                    "crates/oasis-app/src/render.rs: update_sdi",
                    "crates/oasis-sdi/src/registry/draw.rs: SdiRegistry::draw",
                ],
            },
            {
                "title": "Window chrome",
                "crates": ["oasis-wm"],
                "text": "In desktop mode WindowManager::draw_with_clips_overlay draws each "
                "window's decorations, then clips to its content rect and hands the "
                "content area to a callback.",
                "code": ["crates/oasis-wm/src/render.rs: draw_with_clips_overlay"],
            },
            {
                "title": "The app draws itself",
                "crates": ["oasis-core", "oasis-app-core", "oasis-app-calculator"],
                "text": "The callback finds the window's AppRunner, which delegates to the "
                "app's App::draw_windowed (here, the Calculator) with the content rect "
                "and the active theme.",
                "code": [
                    "crates/oasis-core/src/apps/runner.rs: AppRunner::draw_windowed",
                    "crates/oasis-app-core/src/app_trait.rs: App::draw_windowed",
                ],
            },
            {
                "title": "Through the trait boundary",
                "crates": ["oasis-types", "oasis-ui"],
                "text": "Apps and widgets only see &mut dyn SdiBackend: fill_rect, "
                "draw_text, blit and the extension-trait defaults. Nothing above this "
                "line calls a platform API.",
                "code": [
                    "crates/oasis-types/src/backend/sdi_core.rs: SdiCore",
                    "crates/oasis-types/src/backend/sdi_backend.rs: SdiBackend",
                ],
            },
            {
                "title": "Pixels on a platform",
                "crates": [
                    "oasis-backend-sdl",
                    "oasis-backend-wasm",
                    "oasis-backend-ue5",
                    "oasis-backend-psp",
                ],
                "text": "swap_buffers presents: SDL3 on desktop and Pi, Canvas 2D in the "
                "browser (oasis-backend-wasm runs the same window loop), an RGBA "
                "framebuffer UE5 reads through oasis-ffi's oasis_get_buffer, and sceGu "
                "on the PSP (its own loop draws the window manager with per-window "
                "draw functions).",
                "code": [
                    "crates/oasis-backend-sdl/src/core_impl.rs: swap_buffers",
                    "crates/oasis-backend-wasm/src/tick.rs: draw_with_clips_overlay",
                    "crates/oasis-ffi/src/render.rs: oasis_tick, oasis_get_buffer",
                    "crates/oasis-backend-psp/src/render_desktop.rs: draw_with_clips_noalloc",
                ],
            },
        ],
    },
    {
        "id": "page",
        "title": "A web page",
        "blurb": "From a URL to a laid-out, painted page inside a window.",
        "steps": [
            {
                "title": "Navigate",
                "crates": ["oasis-browser"],
                "text": "Every host opens its own BrowserWidget for the Browser window. "
                "navigate_vfs answers from the page cache or the VFS, or queues a page "
                "load on the loader's IO thread.",
                "code": [
                    "crates/oasis-browser/src/widget/pipeline/navigation.rs: navigate_vfs",
                ],
            },
            {
                "title": "Fetch",
                "crates": ["oasis-http", "oasis-net"],
                "text": "oasis-http parses the Url and fetches on its IO thread: HTTP/1.1 or "
                "HTTP/2 over TLS (Gemini framing for gemini://), redirects, cookies and "
                "CSP. poll_io_thread drains the results back on the UI thread.",
                "code": [
                    "crates/oasis-http/src/io_thread.rs: IoThread",
                    "crates/oasis-browser/src/widget/pipeline/navigation.rs: poll_io_thread",
                ],
            },
            {
                "title": "Parse HTML",
                "crates": ["oasis-html"],
                "text": "load_html runs the tokenizer and the tree builder into an arena DOM "
                "(reusing the previous document's allocation when it can).",
                "code": [
                    "crates/oasis-browser/src/widget/pipeline/document.rs: load_html",
                    "crates/oasis-html/src/tree_builder/mod.rs: TreeBuilder::build",
                ],
            },
            {
                "title": "Cascade CSS",
                "crates": ["oasis-css"],
                "text": "Stylesheets from <style>, <link> and the UA sheet go through "
                "cascade::style_tree: selectors, @media / @container / @layer, var() "
                "and computed values for every node.",
                "code": [
                    "crates/oasis-browser/src/widget/pipeline/stylesheets.rs",
                    "crates/oasis-css/src/cascade/mod.rs: style_tree",
                ],
            },
            {
                "title": "Run scripts",
                "crates": ["oasis-js"],
                "text": "With the javascript feature (desktop and PSP), <script> tags run in "
                "QuickJS-NG under a watchdog, against DOM bindings that can mutate the "
                "document and trigger a re-cascade.",
                "code": ["crates/oasis-browser/src/widget/pipeline/scripts.rs"],
            },
            {
                "title": "Layout",
                "crates": ["oasis-browser"],
                "text": "layout::block::build_layout_tree turns styled nodes into boxes: "
                "block, inline, flex, grid, tables, floats, positioning (run twice when "
                "container queries need sizes).",
                "code": [
                    "crates/oasis-browser/src/widget/pipeline/document.rs: build_layout_tree",
                ],
            },
            {
                "title": "Paint",
                "crates": ["oasis-browser", "oasis-types"],
                "text": "BrowserWidget::paint records a display list once and replays it "
                "(or just its dirty parts) into the window's &mut dyn SdiBackend, inside "
                "the clip the window manager set.",
                "code": ["crates/oasis-browser/src/widget/paint.rs: BrowserWidget::paint"],
            },
            {
                "title": "On screen",
                "crates": ["oasis-app", "oasis-wm", "oasis-backend-sdl"],
                "text": "The shell's window callback calls bw.paint(be) for the Browser "
                "window, and the frame presents like any other.",
                "code": ["crates/oasis-app/src/shell.rs: draw_frame"],
            },
        ],
    },
    {
        "id": "video",
        "title": "A video stream",
        "blurb": "From a TV Guide channel to decoded frames and audio on the desktop.",
        "steps": [
            {
                "title": "Pick a channel",
                "crates": ["oasis-app-tv-guide"],
                "text": "The TV Guide app shows an EPG grid over an Internet Archive catalog. "
                "Tuning a channel emits a tune request the shell drains each frame.",
                "code": [
                    "crates/oasis-app-tv-guide/src/grid_state.rs: tune",
                    "crates/oasis-app-tv-guide/src/app.rs: make_tune_ipc",
                ],
            },
            {
                "title": "Tune",
                "crates": ["oasis-app"],
                "text": "tv_controller::tune::handle_tune_requests cancels any old session "
                "and starts a download thread for the URL at the seek offset.",
                "code": ["crates/oasis-app/src/tv_controller/tune.rs: handle_tune_requests"],
            },
            {
                "title": "Download",
                "crates": ["oasis-app", "oasis-net", "oasis-core"],
                "text": "stream_download_range speaks HTTP/1.1 over TCP, with rustls for "
                "https (oasis-net's provider, through oasis-core), reconnecting on stalls "
                "and failing over to a fresh CDN redirect.",
                "code": [
                    "crates/oasis-app/src/tv_controller/download.rs: stream_download_range",
                    "crates/oasis-app/src/tv_controller/cdn_failover.rs",
                ],
            },
            {
                "title": "StreamingBuffer",
                "crates": ["oasis-app"],
                "text": "A sliding window (8 MB behind the read cursor, 16 MB lookahead, 2 MB "
                "prebuffer) the decoder reads through Read + Seek while the download "
                "thread fills it.",
                "code": ["crates/oasis-app/src/tv_controller/streaming_buffer.rs"],
            },
            {
                "title": "Decode",
                "crates": ["oasis-video"],
                "text": "SoftwareVideoDecoder::open_stream demuxes the MP4 (symphonia, with "
                "the dependency-free demux-lite for seek lookup) and decodes H.264 "
                "(ffmpeg or openh264) and AAC on a decoder thread.",
                "code": [
                    "crates/oasis-app/src/video_player.rs: start_software_source",
                    "crates/oasis-video/src/decoder.rs: SoftwareVideoDecoder::open_stream",
                ],
            },
            {
                "title": "Frames and samples",
                "crates": ["oasis-app", "oasis-types"],
                "text": "VideoPlayer::tick paces one frame per display interval into a "
                "texture with load_texture, and feeds PCM to the AudioBackend's "
                "streaming track.",
                "code": [
                    "crates/oasis-app/src/video_player.rs: VideoPlayer::tick",
                    "crates/oasis-app/src/tv_controller/player.rs: tick_video_player",
                ],
            },
            {
                "title": "Back in the guide",
                "crates": ["oasis-app-tv-guide", "oasis-backend-sdl"],
                "text": "The texture becomes the guide's preview_texture; its draw_windowed "
                "blits it into the window, and SDL presents it. (The PSP plays the same "
                "catalog through its own Media Engine pipeline in oasis-backend-psp.)",
                "code": ["crates/oasis-app-tv-guide/src/grid_render.rs: backend.blit"],
            },
        ],
    },
]

DEP_TABLES = (("dependencies", "normal"), ("dev-dependencies", "dev"),
              ("build-dependencies", "build"))
KIND_RANK = {"normal": 0, "optional": 1, "build": 2, "dev": 3}


# --------------------------------------------------------------------------
# Minimal TOML reader (fallback for Python < 3.11). Covers what Cargo
# manifests use: tables, arrays of tables, quoted / dotted keys, basic and
# literal strings, integers, booleans, arrays and inline tables.
# --------------------------------------------------------------------------
class _TomlError(ValueError):
    pass


class _Reader:
    def __init__(self, text):
        self.s = text
        self.i = 0

    def ws(self, newlines=False):
        while self.i < len(self.s):
            c = self.s[self.i]
            if c in " \t\r" or (newlines and c == "\n"):
                self.i += 1
            elif c == "#":
                while self.i < len(self.s) and self.s[self.i] != "\n":
                    self.i += 1
            else:
                break

    def peek(self):
        return self.s[self.i] if self.i < len(self.s) else ""

    def expect(self, c):
        if self.peek() != c:
            raise _TomlError("expected %r at offset %d" % (c, self.i))
        self.i += 1

    def string(self):
        q = self.peek()
        if self.s.startswith(q * 3, self.i):
            end = self.s.index(q * 3, self.i + 3)
            raw = self.s[self.i + 3:end]
            self.i = end + 3
            raw = raw[1:] if raw.startswith("\n") else raw
            return raw if q == "'" else _unescape(raw)
        self.i += 1
        start = self.i
        if q == "'":
            end = self.s.index("'", start)
            self.i = end + 1
            return self.s[start:end]
        out = []
        while True:
            c = self.s[self.i]
            if c == "\\":
                out.append(self.s[self.i:self.i + 2])
                self.i += 2
            elif c == '"':
                self.i += 1
                return _unescape("".join(out))
            else:
                out.append(c)
                self.i += 1

    def key_part(self):
        if self.peek() in "\"'":
            return self.string()
        m = re.compile(r"[A-Za-z0-9_-]+").match(self.s, self.i)
        if not m:
            raise _TomlError("bad key at offset %d" % self.i)
        self.i = m.end()
        return m.group(0)

    def key(self):
        parts = [self.key_part()]
        self.ws()
        while self.peek() == ".":
            self.i += 1
            self.ws()
            parts.append(self.key_part())
            self.ws()
        return parts

    def value(self):
        c = self.peek()
        if c in "\"'":
            return self.string()
        if c == "[":
            self.i += 1
            arr = []
            while True:
                self.ws(True)
                if self.peek() == "]":
                    self.i += 1
                    return arr
                arr.append(self.value())
                self.ws(True)
                if self.peek() == ",":
                    self.i += 1
        if c == "{":
            self.i += 1
            tbl = {}
            self.ws()
            if self.peek() == "}":
                self.i += 1
                return tbl
            while True:
                self.ws()
                k = self.key()
                self.expect("=")
                self.ws()
                _assign(tbl, k, self.value())
                self.ws()
                if self.peek() == ",":
                    self.i += 1
                    continue
                self.expect("}")
                return tbl
        m = re.compile(r"[A-Za-z0-9_+.:-]+").match(self.s, self.i)
        if not m:
            raise _TomlError("bad value at offset %d" % self.i)
        self.i = m.end()
        tok = m.group(0)
        if tok == "true":
            return True
        if tok == "false":
            return False
        try:
            return int(tok.replace("_", ""), 0)
        except ValueError:
            try:
                return float(tok)
            except ValueError:
                return tok


def _unescape(s):
    return re.sub(
        r"\\(u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8}|.)",
        lambda m: {"n": "\n", "t": "\t", "r": "\r", '"': '"', "\\": "\\"}.get(
            m.group(1), chr(int(m.group(1)[1:], 16)) if len(m.group(1)) > 1 else m.group(1)
        ),
        s,
    )


def _assign(tbl, parts, val):
    for p in parts[:-1]:
        tbl = tbl.setdefault(p, {})
    tbl[parts[-1]] = val


def parse_toml(text):
    """Parse Cargo-manifest TOML: tomllib when available, else the fallback."""
    if _tomllib is not None:
        return _tomllib.loads(text)
    return parse_toml_fallback(text)


def parse_toml_fallback(text):
    r = _Reader(text)
    doc = {}
    cur = doc
    while True:
        r.ws(True)
        if r.i >= len(r.s):
            return doc
        if r.peek() == "[":
            r.i += 1
            is_array = r.peek() == "["
            if is_array:
                r.i += 1
            r.ws()
            parts = r.key()
            r.expect("]")
            if is_array:
                r.expect("]")
            parent = doc
            for p in parts[:-1]:
                parent = parent.setdefault(p, {})
                if isinstance(parent, list):
                    parent = parent[-1]
            if is_array:
                cur = {}
                parent.setdefault(parts[-1], []).append(cur)
            else:
                cur = parent.setdefault(parts[-1], {})
        else:
            k = r.key()
            r.expect("=")
            r.ws()
            _assign(cur, k, r.value())


# --------------------------------------------------------------------------
# Manifest scan
# --------------------------------------------------------------------------
def _read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def _doc_line(crate_dir):
    """First `//!` line of the crate root: the description fallback."""
    for name in ("lib.rs", "main.rs"):
        p = os.path.join(crate_dir, "src", name)
        if os.path.exists(p):
            for line in _read(p).splitlines():
                s = line.strip()
                if s.startswith("//!"):
                    s = s[3:].strip()
                    if s:
                        return s
                elif s and not s.startswith("#"):
                    break
    return ""


def _count_rs(crate_dir):
    """Lines and files of Rust source, skipping build output and nested
    packages (e.g. oasis-core/fuzz, which is its own crate)."""
    lines = files = 0
    for dirpath, dirnames, filenames in os.walk(crate_dir):
        dirnames[:] = sorted(
            d
            for d in dirnames
            if d not in ("target", ".git")
            and not os.path.exists(os.path.join(dirpath, d, "Cargo.toml"))
        )
        for fn in filenames:
            if fn.endswith(".rs"):
                with open(os.path.join(dirpath, fn), "rb") as f:
                    data = f.read()
                files += 1
                lines += data.count(b"\n") + (0 if not data or data.endswith(b"\n") else 1)
    return lines, files


def _workspace(root):
    doc = parse_toml(_read(os.path.join(root, "Cargo.toml")))
    ws = doc.get("workspace", {})
    members = {os.path.basename(m.rstrip("/")) for m in ws.get("members", [])}
    excluded = {os.path.basename(m.rstrip("/")) for m in ws.get("exclude", [])}
    version = ws.get("package", {}).get("version", "")
    return members, excluded, version


def _dep_tables(manifest):
    """Yield (table, kind, cfg) for every dependency table, target ones too."""
    for table, kind in DEP_TABLES:
        if table in manifest:
            yield manifest[table], kind, None
    for cfg, body in sorted(manifest.get("target", {}).items()):
        for table, kind in DEP_TABLES:
            if table in body:
                yield body[table], kind, cfg


def _features_enabling(features, dep):
    out = []
    for feat, items in features.items():
        for it in items:
            if it == "dep:" + dep or it == dep or it.split("/", 1)[0].rstrip("?") == dep:
                out.append(feat)
                break
    return sorted(out)


def scan(root=ROOT):
    """Read every crate manifest under root/crates; return the crate list."""
    members, excluded, _ = _workspace(root)
    crates_dir = os.path.join(root, "crates")
    raw = {}
    for d in sorted(os.listdir(crates_dir)):
        mpath = os.path.join(crates_dir, d, "Cargo.toml")
        if os.path.isfile(mpath):
            raw[d] = parse_toml(_read(mpath))
    names = {m["package"]["name"]: d for d, m in raw.items()}

    crates = []
    for d, m in raw.items():
        pkg = m["package"]
        name = pkg["name"]
        features = m.get("features", {})
        deps = {}
        for table, kind, cfg in _dep_tables(m):
            for key, spec in table.items():
                spec = spec if isinstance(spec, dict) else {"version": spec}
                target = spec.get("package", key)
                if target not in names or target == name:
                    continue
                k = kind
                if kind == "normal" and spec.get("optional"):
                    k = "optional"
                entry = {"name": target, "kind": k}
                if k == "optional":
                    entry["features"] = _features_enabling(features, key)
                if cfg:
                    entry["cfg"] = cfg
                prev = deps.get(target)
                if prev is None or KIND_RANK[k] < KIND_RANK[prev["kind"]]:
                    deps[target] = entry
        if d in members:
            membership = "member"
        elif d in excluded:
            membership = "excluded"
        else:
            membership = "standalone"
        crate_dir = os.path.join(crates_dir, d)
        lib = m.get("lib", {})
        bins = [b["name"] for b in m.get("bin", []) if "name" in b]
        if not bins and os.path.exists(os.path.join(crate_dir, "src", "main.rs")):
            bins = [name]
        has_lib = bool(lib) or os.path.exists(os.path.join(crate_dir, "src", "lib.rs"))
        version = pkg.get("version")
        loc, files = _count_rs(crate_dir)
        crates.append(
            {
                "name": name,
                "dir": "crates/" + d,
                "layer": None,
                "description": pkg.get("description") or _doc_line(crate_dir),
                "target": "workspace" if membership == "member" else "psp",
                "membership": membership,
                "version": version if isinstance(version, str) else None,
                "lib": has_lib,
                "crate_type": lib.get("crate-type", []),
                "bins": bins,
                "features": sorted(f for f in features if f != "default"),
                "loc": loc,
                "files": files,
                "deps": sorted(
                    deps.values(), key=lambda e: (KIND_RANK[e["kind"]], e["name"])
                ),
            }
        )
    return crates


def build(root=ROOT):
    """The whole crates.json document, validated against LAYERS and WALKS."""
    crates = scan(root)
    by_name = {c["name"]: c for c in crates}
    errors = []
    for layer in LAYERS:
        for n in layer["crates"]:
            if n not in by_name:
                errors.append("layer %s names unknown crate %s" % (layer["id"], n))
            elif by_name[n]["layer"]:
                errors.append("crate %s is in two layers" % n)
            else:
                by_name[n]["layer"] = layer["id"]
    for c in crates:
        if not c["layer"]:
            errors.append("crate %s has no layer: add it to LAYERS" % c["name"])
    for walk in WALKS:
        for i, step in enumerate(walk["steps"]):
            for n in step["crates"]:
                if n not in by_name:
                    errors.append("walk %s step %d names unknown crate %s" % (walk["id"], i, n))
    if errors:
        raise SystemExit("gen-crate-map: " + "\n  ".join([""] + errors))
    _, _, version = _workspace(root)
    order = {n: i for layer in LAYERS for i, n in enumerate(layer["crates"])}
    crates.sort(key=lambda c: ([lay["id"] for lay in LAYERS].index(c["layer"]), order[c["name"]]))
    return {
        "generated_by": "scripts/gen-crate-map.py",
        "repo": REPO,
        "version": version,
        "layers": [
            {"id": lay["id"], "title": lay["title"], "blurb": lay["blurb"]} for lay in LAYERS
        ],
        "crates": crates,
        "walks": WALKS,
    }


def dumps(doc):
    return json.dumps(doc, indent=1, ensure_ascii=False) + "\n"


def structure(doc):
    """The document minus the line counts, for the staleness check."""
    d = json.loads(json.dumps(doc))
    for c in d.get("crates", []):
        c.pop("loc", None)
        c.pop("files", None)
    return d


def check(root=ROOT):
    """Return a list of differences between the committed JSON and a fresh build."""
    path = os.path.join(root, *OUT.split("/"))
    if not os.path.exists(path):
        return ["%s does not exist" % OUT]
    with open(path, encoding="utf-8") as f:
        old = structure(json.load(f))
    new = structure(build(root))
    diffs = []
    old_c = {c["name"]: c for c in old.get("crates", [])}
    new_c = {c["name"]: c for c in new["crates"]}
    for n in sorted(set(old_c) | set(new_c)):
        if n not in old_c:
            diffs.append("new crate %s" % n)
        elif n not in new_c:
            diffs.append("removed crate %s" % n)
        elif old_c[n] != new_c[n]:
            keys = sorted(k for k in new_c[n] if old_c[n].get(k) != new_c[n].get(k))
            diffs.append("%s: %s changed" % (n, ", ".join(keys)))
    for k in ("layers", "walks", "version", "repo"):
        if old.get(k) != new.get(k):
            diffs.append("%s changed" % k)
    if not diffs and old != new:
        diffs.append("crate order changed")
    return diffs


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true", help="fail if %s is stale" % OUT)
    args = ap.parse_args(argv)
    if args.check:
        diffs = check()
        if diffs:
            print("%s is stale:" % OUT)
            for d in diffs:
                print("  " + d)
            print("Regenerate it: python3 scripts/gen-crate-map.py")
            return 1
        print("%s is up to date" % OUT)
        return 0
    doc = build()
    path = os.path.join(ROOT, *OUT.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(dumps(doc))
    edges = sum(len(c["deps"]) for c in doc["crates"])
    print("wrote %s: %d crates, %d edges" % (OUT, len(doc["crates"]), edges))
    return 0


if __name__ == "__main__":
    sys.exit(main())

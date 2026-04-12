# Loading & navigation

This doc covers everything between "user clicked a link" and "the new
document is in memory": URL resolution, the resource cache, the IO
thread, cookies, CSP, and forms.

## Files

```text
src/
├── nav.rs                  navigation history (back / forward stacks)
├── loader/
│   ├── mod.rs              orchestrator: cache lookup, dispatch, content type detection
│   ├── http.rs             HTTP/HTTPS fetch (TLS via oasis-types::TlsProvider)
│   ├── gemini_fetch.rs     Gemini protocol fetch
│   ├── io_thread.rs        background IO thread + IoWork / IoResult queue
│   ├── cache.rs            byte-bounded LRU response cache
│   ├── cookies.rs          cookie jar (per-host, persistent across navigations)
│   ├── csp.rs              Content-Security-Policy enforcement
│   └── vfs.rs              virtual filesystem source
├── forms/                  form state, validation, submission
└── widget_pipeline.rs      hooks IoResult → parse → cascade → layout
```

## Resource sources

`ResourceRequest` carries a URL plus a `ResourceSource`:

- `Network` — fetch over HTTP, HTTPS, or Gemini.
- `Vfs` — read from the embedded virtual filesystem (sandbox builds).
- `VfsThenNetwork` — try VFS first, fall back to network on miss. This
  is the default for desktop builds with bundled assets.

The loader picks a fetcher based on URL scheme (`http`, `https`,
`gemini`, `file`, `about`) and the requested source.

## Cache

`ResourceCache` (`loader/cache.rs`) is an LRU keyed by URL, bounded by
total bytes (default 8 MiB, configurable via `BrowserConfig`). Entries
store the response body, content type, ETag, `Last-Modified`, and the
parsed forms (e.g. an HTML body keeps a precomputed `Document` so
back / forward navigation skips re-parsing).

`navigate_cached_or_fetch()` is the canonical entry point — it tries
the cache first, then falls back to a fresh fetch. Cache validators
(ETag, If-Modified-Since) are honored on revalidation.

## IO thread

The IO thread (`loader/io_thread.rs`) is the only background thread in
the crate. It is spawned lazily on the first network request and lives
for the lifetime of the `BrowserWidget`.

```text
main thread                  IO thread
-----------                  ---------
push IoWork  ─────────────▶  recv IoWork
                              fetch (HTTP / TLS / Gemini)
                              decode (gzip)
                              update cookie jar
recv IoResult ◀────────────  send IoResult
parse + cascade + layout
```

`IoWork` and `IoResult` cross thread boundaries via `mpsc::Sender` /
`Receiver`. The IO thread owns its own `CookieJar` clone; cookie
updates are merged back into the main-thread jar when the result is
processed. The TLS provider is borrowed via a raw pointer that the
crate guarantees outlives the IO thread (the `BrowserWidget` joins the
thread on `Drop`).

The IO thread processes work **sequentially** — there is no parallel
fetch. This is fine for an embedded engine driving one tab.

## Cookies

`CookieJar` (`loader/cookies.rs`) is a per-host cookie store with
`Set-Cookie` parsing, `Path`, `Domain`, `Expires`, `Max-Age`,
`Secure`, `HttpOnly`, and `SameSite` honored. JavaScript reads /
writes via `document.cookie`, which delegates to the same jar.

## CSP

`loader/csp.rs` parses `Content-Security-Policy` response headers and
enforces:

- `script-src` — blocks inline / external `<script>` execution.
- `style-src` — blocks inline / external `<style>` and `style="…"`.
- `connect-src` — blocks `fetch()` / XHR / WebSocket connect attempts.
- `img-src` — relaxed (warn-only) so existing pages render.

CSP enforcement happens at the loader (for resource requests) and at
the JS engine (for inline script execution). Violations are logged via
the `log` crate.

## Navigation history

`NavigationController` (`nav.rs`) maintains:

```rust
pub struct NavigationController {
    back_stack: Vec<BrowserHistoryEntry>,
    forward_stack: Vec<BrowserHistoryEntry>,
    current: BrowserHistoryEntry,
}

pub struct BrowserHistoryEntry {
    pub url: Url,
    pub title: String,
    pub scroll_y: f32,
    pub form_state: HashMap<NodeId, String>,
}
```

- `navigate_url(url)` pushes the current entry onto `back_stack`,
  resets `forward_stack`, and starts a fetch.
- `go_back()` and `go_forward()` swap entries between the stacks and
  ask the loader for a cached response (no re-fetch by default).
- The painter restores `scroll_y` after layout completes, and the form
  manager restores text input values from `form_state`. This makes
  back / forward feel instant on cached pages.

## Forms

`forms/` implements form state and submission:

- **`FormManager`** owns per-form state, focus, tab order.
- **Input types** supported: `text`, `password`, `checkbox`, `radio`,
  `hidden`, `submit`, `button`, `reset`, `file`, `search`, `email`,
  `number`, `range`, `date`, `color`, `url`, `tel`, plus `<textarea>`
  and `<select>` (with overlay dropdown).
- **`<label for="…">`** click forwards focus to the labelled control.
- **Tab key** cycles through focusable elements in document order;
  `tabindex` is honored.
- **Submission** — `<form>` collects field values, builds a
  `application/x-www-form-urlencoded` (default) or
  `multipart/form-data` (for file uploads) body, and calls
  `BrowserWidget::navigate_post(url, body)`.

Validation lives in `forms/validation.rs` (HTML5 `required`, `pattern`,
`min`, `max`, `step`, type-specific format checks). It runs on submit
and via the JS-exposed `checkValidity()` API.

## Image loading

`widget_images.rs` plus `image.rs` and `image_atlas.rs` handle image
decode:

1. `<img src="…">` triggers a loader fetch.
2. The bytes are dispatched to a worker thread that runs the format
   detector (`image::detect_format`) and the appropriate decoder
   (PNG / JPEG / GIF / BMP).
3. The decoded RGBA buffer is uploaded as a backend texture.
4. Many small images (favicons, icons, list bullets) are packed into a
   shared `ImageAtlas` so they can be drawn with a single `blit_sub`.

The decoder pool is small (typically 2 workers) and uses the same
channel pattern as the IO thread.

## Plugins

`plugin.rs` exposes a `UrlSchemeHandler` trait. Embedders can register
custom URL schemes (e.g. `oasis://`) to intercept loads and serve
content from a Rust function. The PSP build uses this to expose
in-binary documentation pages without going through the IO thread.

## Tests

- `loader::cache::tests` — LRU eviction, byte budget enforcement.
- `loader::cookies::tests` — `Set-Cookie` parsing, expiry, host scoping.
- `nav::tests` — back / forward stack invariants, plus a `proptest`
  block (`prop`) that verifies path-independence properties.
- `tests/browser_integration.rs` — full pipeline navigation and link
  click smoke tests.

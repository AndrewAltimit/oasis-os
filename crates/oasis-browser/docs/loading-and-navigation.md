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
│   ├── http1.rs            HTTP/1.1 framing + redirect-hop header policy (shared with http_psp.rs)
│   ├── http2.rs, hpack.rs  HTTP/2 (ALPN h2) + HPACK
│   ├── gemini_fetch.rs     Gemini protocol fetch
│   ├── io_thread.rs        background IO thread + IoWork / IoResult queue
│   ├── cache.rs            byte-bounded LRU response cache
│   ├── cookies.rs          RFC 6265 session cookie jar
│   ├── csp.rs              Content-Security-Policy parsing + checks
│   └── vfs.rs              virtual filesystem source
├── forms/                  form state, validation, submission
└── widget/pipeline.rs      hooks IoResult → parse → cascade → layout
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

`ResourceCache` (`oasis-http/src/cache.rs`) is an LRU keyed by URL, bounded by
total bytes (default 8 MiB, configurable via `BrowserConfig`). Entries
store the response body, content type, ETag, `Last-Modified`, and the
parsed forms (e.g. an HTML body keeps a precomputed `Document` so
back / forward navigation skips re-parsing).

`navigate_cached_or_fetch()` is the canonical entry point — it tries
the cache first, then falls back to a fresh fetch. Cache validators
(ETag, If-Modified-Since) are honored on revalidation.

## IO thread

The IO thread (`oasis-http/src/io_thread.rs`) is the only background thread in
the crate. It is spawned lazily on the first network request and lives
for the lifetime of the `BrowserWidget`.

```text
main thread                  IO thread
-----------                  ---------
push IoWork  ─────────────▶  recv IoWork
                              fetch (HTTP / TLS / Gemini)
                              decode (gzip)
                              update cookie jar (per redirect hop)
recv IoResult ◀────────────  send IoResult
parse + cascade + layout
```

`IoWork` and `IoResult` cross thread boundaries via `mpsc::Sender` /
`Receiver`. The IO thread owns its own `CookieJar` clone; cookie
updates (one `(hop URL, Set-Cookie headers)` entry per redirect hop)
are merged back into the main-thread jar when the result is processed.
The IO thread holds the TLS provider by `Arc`. Dropping the widget
detaches a busy worker, which finishes its in-flight request and exits.

The IO thread processes work **sequentially** — there is no parallel
fetch. This is fine for an embedded engine driving one tab.

## Cookies

`CookieJar` (`oasis-http/src/cookies.rs`) is a session-only RFC 6265 store
(desktop; PSP has none yet):

- host-only cookies unless `Domain=` is given; `Domain=` must
  domain-match the setting host, is refused for IP hosts, and is refused
  for public suffixes (single-label domains plus the embedded
  `MULTI_LABEL_SUFFIXES` list — not a full PSL);
- `Max-Age` (preferred) / `Expires` (RFC 6265 §5.1.1 date parser) set an
  absolute expiry capped at 400 days; `Max-Age<=0` or a past date
  deletes; expired cookies are never sent;
- `Secure` cookies go over `https:` only, are refused from `http:`
  responses, and cannot be overwritten or deleted from `http:`;
  `__Secure-` / `__Host-` prefixes are enforced;
- control characters and `name=value` over 4 KiB are refused; at most 50
  cookies per domain and 3000 total (oldest evicted);
- `SameSite` is enforced when sending (`SameSiteContext`): a request is
  cross-site when its registrable domain (PSL-lite) differs from that of
  `ResourceRequest::initiator`. Cross-site subresources (images,
  stylesheets) get no `Strict` / `Lax` cookies; cross-site top-level
  navigations get `Lax` ones only with GET / HEAD, so a cross-site form
  POST carries neither. Missing `SameSite` behaves as `None`. GET
  navigations have no initiator (typed URLs and link clicks share
  `navigate_vfs_inner`), so they count as same-site; `SameSite` is not
  applied when storing.

The HTTP client asks a `CookieHook` for each redirect hop's own
`Cookie:` header and stores every hop's `Set-Cookie` under that hop's
URL (`http::http_request_with_cookies`; the I/O thread's `RecordingJar`
and the synchronous path's `cookies::ScopedJar` add the `SameSite`
context), so cookies never follow a cross-site redirect and a login →
302 → dashboard flow keeps its session. The only cookie-carrying
synchronous loads are page loads when the I/O thread can't be used, and
the I/O thread re-clones the main jar whenever it is respawned, so the
two jars don't diverge in practice. `document.cookie` is a separate
per-page map (`js_dom/storage.rs`; 4 KiB per pair, 50 pairs) that is
never sent on requests.

## CSP

`oasis-http/src/csp.rs` parses every `Content-Security-Policy` response header
(each comma-separated policy must allow a load; report-only and
`<meta http-equiv>` policies are ignored). `widget/pipeline.rs` and
`js_dom/fetch.rs` enforce:

- `script-src-elem` → `script-src` → `default-src` — inline `<script>`
  runs only with a matching `nonce`, a matching `'sha256-…'` hash, or
  `'unsafe-inline'` (ignored when the list has a nonce, hash or
  `'strict-dynamic'`). External `<script src>` is never loaded.
- `script-src-attr` → `script-src` → `default-src` — each inline event
  handler (`onclick=…`) is checked with its own text: registered under
  `'unsafe-inline'`, or `'unsafe-hashes'` plus a matching hash.
  Scripts and handlers inside `<template>` never run.
- `style-src-elem` → `style-src` → `default-src` — inline `<style>`
  (nonce / hash / `'unsafe-inline'`) and `<link rel=stylesheet>` /
  `@import` URLs.
- `style-src-attr` → `style-src` → `default-src` — parser-inserted
  `style="…"` attributes. Attributes written later by scripts
  (`element.style`) are not filtered.
- `connect-src` → `default-src` — script `fetch()`, including every
  redirect hop and `data:` URLs.
- `img-src` — parsed, not enforced (see `widget/images.rs`).

The policy is stored with the page's cache entry, so a back / forward
re-render from cache, or a `304 Not Modified` without a CSP header, runs
under the policy the page was served with. Violations are logged with
`log::warn!`; nothing is reported.

`vfs://` subresources (images, stylesheets, `@import`, fonts) are
refused for pages with a network origin
(`loader::subresource_allowed`), and the HTTP clients refuse redirects
from the network to any non-http(s) scheme (Gemini: non-`gemini:`), so a
remote page can never end up labelled with a local URL.

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

`widget/images.rs` plus `image.rs` and `image_atlas.rs` handle image
decode:

1. `<img src="…">` triggers a loader fetch.
2. The bytes are dispatched to a decode thread that runs the format
   detector (`image::detect_format`) and the appropriate decoder
   (PNG / JPEG / GIF / BMP, WebP with the `webp` feature) through
   `image::decode_image_with_limits(data, config.image_limits())`.
3. Images larger than `BrowserConfig::max_image_dimension` (per side)
   or `max_image_pixels` are downscaled: JPEGs by DCT scaling during
   decode (1/2, 1/4, 1/8), other formats bilinearly after decode.
   Defaults are 4096 px / 2 Mpx on desktop and 1024 px / 1 Mpx on PSP.
   PNG / GIF / WebP / BMP sources wider or taller than 4096 px (1024 on
   PSP) are rejected, since they decode at full size first. The decoded
   image keeps its natural size, which layout uses for `<img>` boxes and
   background tiles.
4. The decoded RGBA buffer is uploaded as a backend texture.
5. Many small images (favicons, icons, list bullets) are packed into a
   shared `ImageAtlas` so they can be drawn with a single `blit_sub`.

There is one decode thread on native targets; WASM and PSP decode
synchronously.

## Tests

- `loader::cache::tests` — LRU eviction, byte budget enforcement.
- `loader::cookies::tests` — `Set-Cookie` parsing, expiry / deletion,
  host-only and public-suffix scoping, caps.
- `loader::csp::tests`, `loader::http1::tests`, `loader::http2::tests`,
  `loader::hpack::tests` — CSP matching, framing, redirect-hop headers,
  HTTP/2 + HPACK caps.
- `e2e_tests::http` — cross-site redirect cookie isolation, redirect-hop
  `Set-Cookie`, `Max-Age=0` logout, Referer reduction, `vfs://` gating,
  CSP `style-src`; `e2e_tests::js` — CSP `script-src` nonces.
- `nav::tests` — back / forward stack invariants, plus a `proptest`
  block (`prop`) that verifies path-independence properties.
- `tests/browser_integration.rs` — full pipeline navigation and link
  click smoke tests.

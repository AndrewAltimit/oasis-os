# Gemini protocol

[Gemini](https://geminiprotocol.net/) is a small text protocol that
sits between Gopher and the web. `oasis-browser` ships first-class
Gemini support: any URL with the `gemini://` scheme is fetched, parsed,
converted to HTML, and rendered through the same layout / paint
pipeline as a regular page.

## Files

```text
src/
├── gemini/
│   ├── mod.rs        GeminiStatus, response types
│   ├── parser.rs     .gmi text-format parser (line-oriented)
│   └── renderer.rs   .gmi → HTML translation
└── loader/
    └── gemini_fetch.rs  TLS-only fetcher (no plain TCP fallback)
```

## Protocol notes

Gemini is intentionally minimal:

- Single-line request: `gemini://host/path\r\n`.
- Mandatory TLS, no version negotiation, no SNI requirement.
- Single-line response header: `<status code> <meta>\r\n`.
- Body follows for `2x` status codes only.

`GeminiStatus` covers all six status classes:

| Code | Class | Meaning |
| --- | --- | --- |
| 1x | Input | Server requesting input via `meta` prompt. |
| 2x | Success | `meta` is the MIME type of the body. |
| 3x | Redirect | `meta` is the redirect URL. |
| 4x | TemporaryFailure | Retryable error. |
| 5x | PermanentFailure | Non-retryable error. |
| 6x | ClientCertRequired | Body protected by client cert (not supported). |

We follow redirects automatically up to 5 hops, surface input requests
as a small inline form, and render failure responses as plain error
pages.

## `.gmi` format

Gemini's text format is line-oriented. Every line falls into one of a
small number of categories:

| Prefix | Meaning |
| --- | --- |
| `# ` | Heading 1 |
| `## ` | Heading 2 |
| `### ` | Heading 3 |
| `=> url [label]` | Link line |
| `> ` | Blockquote |
| `* ` | List item |
| ```` ``` ```` | Toggle preformatted block |
| (anything else) | Paragraph text |

`parser.rs` produces an iterator of `GeminiLine` enum values; nothing
is stateful except the preformatted toggle.

## Translation to HTML

`renderer.rs` walks the parsed lines and emits a synthetic HTML
document:

```text
# heading           → <h1>heading</h1>
=> /faq Frequently  → <p><a href="/faq">Frequently</a></p>
> quote             → <blockquote>quote</blockquote>
* item              → <ul><li>item</li></ul>   (consecutive items grouped)
```` text ````        → <pre>text</pre>           (consecutive pre lines grouped)
plain text          → <p>plain text</p>
```

The synthetic HTML is fed to the same `TreeBuilder` and cascade as a
regular page, so the only Gemini-specific code in the rest of the
crate is the URL scheme dispatch in the loader.

## Styling

Gemini pages reuse the default UA stylesheet plus a small `gemini.css`
that lives in `src/css/default.rs`. The defaults give links the usual
underlined-blue look, blockquotes a left border, and `<pre>` blocks a
monospace font and subtle background.

Author CSS is **not** loaded for Gemini pages — there is no
`<link rel="stylesheet">` in `.gmi` and the protocol does not have an
equivalent. If you want a custom theme, edit `gemini.css`.

## Loader integration

`loader/gemini_fetch.rs` mirrors `loader/http.rs` but enforces
`gemini://` scheme and TLS. Fetches go through the same `IoThread` as
HTTP requests, so caching, navigation history, and back / forward all
just work — there is no Gemini-specific cache or history path.

## Tests

There are no dedicated Gemini integration tests in this crate yet. The
parser is exercised by `parser::tests` and the renderer is covered
indirectly by manual smoke pages in `tests/browser_integration.rs`.

## Limitations

- Client certificates are not supported (status `6x`).
- The Gemini-specific input form (status `1x`) is functional but
  visually plain.
- We do not implement Titan (the upload extension) or any other
  Gemini-adjacent protocols.

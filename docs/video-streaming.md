# Video Streaming

Desktop and PSP implementations of MP4/H.264+AAC streaming for the TV
Guide app. The PSP path is summarised in
[`psp-architecture.md`](psp-architecture.md); this document focuses on
the desktop pipeline.

## Desktop: `StreamingBuffer` progressive playback

TV Guide on desktop uses in-process progressive streaming via
`StreamingBuffer` (in `tv_controller.rs`). A background download
thread feeds an `Arc<StreamingInner>` sliding-window buffer while
symphonia decodes from the same buffer via `Read + Seek`.

### Key mechanisms

- **`probe_mode`** — during symphonia's probe phase, reads return
  zeros so the `mdat` body is skipped instantly. `decoder_pos` is NOT
  updated during probe to prevent a throttle deadlock.
- **Deferred tail probe** — a separate thread fetches the last 8 MB
  for moov-at-end files, but only launches after >8 MB body data has
  been received without finding `moov`. Prevents CDN connection
  throttling.
- **`should_throttle()`** — backpressure:
  `decoder_pos > 0 ? received > decoder_pos + 16 MB : has_moov &&
  buf_size > 16 MB`.
- **CDN failover** — range requests route through the original
  archive.org URL (not the cached CDN) to get a fresh 302 redirect,
  avoiding 401 errors from stale CDN nodes. `open_range_connection()`
  follows redirect chains.
- **Prebuffer gate** — decoder waits for `MIN_PREBUFFER` (2 MB) of
  body data before seeking, preventing reads into empty buffer
  regions.
- **Seek restart** — after probe discovers `moov`, the download
  restarts from the estimated byte offset via a Range request.
  Linear interpolation: `(seek_secs / duration) * file_size`.
- **HTTPS ALPN pinning** — every blocking TLS client in the streaming
  path (`fetch_range_inner`, `open_range_connection_inner`,
  `stream_download_inner`, plus the catalog `https_get_body`) is an
  HTTP/1.1 parser. The shared rustls config also advertises `h2` so
  the in-app browser can negotiate HTTP/2, so these clients call
  `connect_tls_with_alpn(.., &[b"http/1.1"])` to pin the server to
  HTTP/1.1. Without this, archive.org CDN endpoints select `h2` and
  the `\r\n\r\n` header parser trips on HTTP/2 frames — surfacing as
  `"connection closed before headers complete"`.

## Related docs

- [`psp-architecture.md`](psp-architecture.md) §Video Streaming — the
  PSP in-memory path (demux_lite, sceMpeg, sceAudiocodec).
- `crates/oasis-video/` — shared demux / decode crate.

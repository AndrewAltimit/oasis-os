# Video Streaming

Desktop and PSP implementations of MP4/H.264+AAC streaming for the TV
Guide app. The PSP path is summarised in
[`psp-architecture.md`](psp-architecture.md); this document focuses on
the desktop pipeline.

## Desktop: `StreamingBuffer` progressive playback

TV Guide on desktop uses in-process progressive streaming via
`StreamingBuffer` (in oasis-app,
`crates/oasis-app/src/tv_controller/streaming_buffer.rs`; it implements
`oasis_video::VideoSource`). A background download thread feeds an
`Arc<StreamingInner>` sliding-window buffer while
`oasis_video::SoftwareVideoDecoder` reads the same buffer via
`Read + Seek`.

### Decoder backends (oasis-video features)

oasis-video's features are additive and none is on by default:

| Feature | What it adds |
|---|---|
| `demux-lite` | `demux_lite` (dependency-free MP4 parser: PSP, `seek_point_from_moov`, fuzzing) |
| `symphonia` | symphonia MP4 demux + AAC-LC; an audio-only `SoftwareVideoDecoder`. Implies `demux-lite` |
| `h264` | openh264 video decode for the symphonia backend (Baseline profile). Implies `symphonia` |
| `ffmpeg` | statically linked ffmpeg demux + H.264 + AAC (Main/High profile, SIMD) |

oasis-app's `video-decode` / `video-decode-ffmpeg` (and the same pair in
oasis-ffi) select `h264` / `ffmpeg`; oasis-app also enables `demux-lite`.
When both `h264` and `ffmpeg` end up enabled (`--all-features`, or two
crates in one build picking different backends), ffmpeg is the
`SoftwareVideoDecoder` backend and the symphonia modules stay available
as libraries. `SoftwareVideoDecoder::open_stream_with_moov` and
`keyframe_seek_target` exist on both backends (ffmpeg reads the
container itself, so it ignores the moov and seeks to keyframes on its
own). The PSP build enables only `demux-lite` and so links neither
symphonia nor openh264.

The probe-mode and keyframe details below describe the symphonia
backend, which is what `video-decode` builds and the e2e tests in
`crates/oasis-video/tests/decode_e2e.rs` exercise (they are compiled
only when openh264 is the selected backend, i.e. `h264` without
`ffmpeg`).

### Opening a stream (symphonia backend)

symphonia does not extract H.264 parameters, so the decoder needs the
`avcC` box (SPS/PPS + NAL length size) and the `stss` keyframe index
itself:

- `open_stream_with_moov(source, moov)` — the caller already has the
  `moov` atom (the streaming download retains it); nothing extra is read.
- `open_stream(source)` — walks the **top-level box headers** with
  seeks, skipping `mdat`, reads only `moov` (capped at 64 MiB), then
  rewinds and hands the source to symphonia. (It used to `read_to_end`
  the whole source; `open(Vec<u8>)` copied the file a second time.)
- `avcC` is looked up inside `moov` only, so bytes in `mdat` that happen
  to spell `avcC` (a moov-at-end file) cannot be taken for it.

While one stream is read, the other stream's packets are buffered, at
most 2048 per stream: past that the oldest is dropped (a dropped video
packet restarts decode at the next IDR), so audio-only consumption of a
file with video does not buffer the whole video track.

### `demux_lite` (PSP, seek points)

`demux_lite::Mp4Lite` / `parse_moov_tracks` parse untrusted network
bytes on every platform, so they are hardened against hostile files:

- every box is bounds-checked against its parent and its end offset is
  overflow-checked; container nesting is limited to 16 levels;
- a sample table must hold the entries it declares (allocation is bounded
  by the bytes actually in the file); a constant-size `stsz` is stored as
  `(size, count)`; `avcC` / `esds` are capped at 64 KiB and single samples
  at 32 MiB;
- `SampleTable::finalize` builds a per-run index, so offset / PTS / DTS /
  seek lookups are binary searches rather than walks over the tables
  (sequential playback was quadratic in track length, and a hostile
  `stsc` / `stts` could spin a single lookup for billions of steps);
- `esds` is parsed by walking its descriptors, `mp4a` reads the real
  `channelcount` (and skips QuickTime v1/v2 sound-entry extensions), and
  `avcC` keeps the first SPS and PPS.

Unit tests sweep truncations and bit flips of every fixture in
`tests/fixtures/` through all public entry points, and a proptest checks
the indexed lookups against the original linear walks.

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
  restarts via a Range request just before (256 KB) the exact byte the
  demuxer will read first: `demux_lite::seek_point_from_moov` finds the
  video keyframe at or before the target in the sample tables and takes
  the lower of its offset and the audio sample's at that time. Linear
  interpolation (`(seek_secs / duration) * mdat_size`, 2 MB margin) is
  only a fallback — on VBR video it can miss by many MB, and a restart
  past the real sample makes every read land in the evicted region.
- **Keyframe seek** — symphonia's isomp4 demuxer ignores `stss`, so a
  raw seek lands mid-GOP. `SoftwareVideoDecoder` indexes keyframes from
  the moov and snaps `seek()` back to the keyframe (the same one the
  seek restart targets), so video starts immediately and audio starts
  with it. Non-IDR packets before the first IDR are discarded, and every
  IDR carries SPS/PPS so the decoder can resync after an error.
- **openh264 flushing** — the decoder is built with `Flush::NoFlush`.
  The crate default flushes the reorder buffer after every packet, which
  on B-frame (Main/High profile) streams fails with `dsOutOfMemory` after
  a few frames and then loses the parameter sets (video froze, playback
  fell back to audio-only). Buffered pictures are drained at EOF.
- **Stall recovery** — a demux read error ends the session (auto-advance
  retunes) instead of spinning in audio-only mode. When the decoder runs
  dry and a frame then arrives >300 ms late, the player resumes its clock
  from that frame rather than skipping video ahead of the (also stalled)
  audio.
- **Resume** — if the linear download's connection closes, errors or
  stalls (`STALL_TIMEOUT`) before `Content-Length`, it resumes from the
  buffer frontier via `stream_download_range`, which also resumes a
  Range body that closes early. Throttle pauses are never treated as
  stalls (16 MB of lookahead is minutes of video), and the reconnect
  budget resets once a connection delivers 1 MB. A server that answers
  Range with `200` is handled by skipping the already-received prefix.
- **Probe edge cases** — probe-mode reads within 64 KB past the download
  frontier wait (≤10 s) for the real bytes once moov is retained, so the
  atom headers right after moov are never read as zeros; reads inside a
  top-level atom that claims to extend past a truncated file's end
  return zeros during the probe so truncated files still open.
- **End-to-end tests** — `tv_controller/e2e_tests.rs` serves fixtures from
  an in-process HTTP server with scriptable misbehaviour (throttle, drop,
  stall, ignored Range, redirect, 404, truncation) and drives the real
  download, `StreamingBuffer` and `VideoPlayer` (virtual clock via
  `VideoPlayer::tick_at`). Decoder-level e2e tests live in
  `crates/oasis-video/tests/decode_e2e.rs`; fixtures in
  `tests/fixtures/streaming/`.
- **Dev builds** — the workspace optimizes the codec crates (openh264,
  symphonia) even in the `dev` profile; unoptimized, 720p decode ran at
  ~9 fps under `cargo run`.
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

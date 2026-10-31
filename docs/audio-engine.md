# Audio Engine

`oasis-audio` is the playlist + radio + format-glue layer that sits between
apps (Music Player, Internet Radio, video apps) and the per-target
`AudioBackend` implementations. Decoding lives almost entirely on the backend
side; this crate's job is sequencing, metadata, and streaming back-pressure.

The `AudioBackend` trait is defined in
`oasis-types/src/backend/audio.rs`. Backends (SDL3 on desktop/Pi, WASM via
WebAudio, UE5 via the engine mixer, PSP via `sceAudio` + `sceAudiocodec`)
implement it.

## Public types

All in `crates/oasis-audio/src/`.

- `AudioManager` (`manager.rs`) — the music playlist + playback-state
  machine behind the `/var/audio/*` VFS IPC. It never touches an
  `AudioBackend`: `apply(&MediaRequest)` updates the playlist/state and
  returns a `MediaAction` (`PlayPath`, `Cue`, `Pause`, `Resume`, `Stop`,
  `SetVolume`, `None`) for the host to carry out. `track_finished()` is the
  end-of-track transition (advance like `next`).
- `Playlist` (`playlist.rs`) — ordered tracks plus shuffle/repeat state.
- `TrackInfo` (`types.rs`) — title, artist, duration, VFS path, builder
  setters.
- `PlaybackState` (`types.rs`) — `Stopped` (default), `Playing`, `Paused`.
- `RepeatMode` — `Off`, `All`, `One`; case-insensitive parse. Defined in
  `oasis-types/src/ipc.rs` next to `MediaRequest` and re-exported here.
- `RadioManager` (`radio/mod.rs`) plus `RadioSource` trait and three sources:
  `IcecastSource`, `ArchiveSource`, `VfsSource`.
- `mixer` — the two PCM helpers the SFX engine uses (`mono_to_stereo_i16`,
  `resample_nearest_i16`).

## Music IPC (`/var/audio/*`)

The paths and the request grammar live in `oasis_types::ipc` so producers
and the consumer cannot drift apart:

| Path | Direction | Contents |
| --- | --- | --- |
| `AUDIO_REQUEST_PATH` = `/var/audio/request` | in | One `MediaRequest` per line: `play_file <path>`, `play`, `pause`, `resume`, `stop`, `next`, `prev`, `vol <0-100>`, `repeat <off\|all\|one>`, `shuffle`. |
| `AUDIO_STATUS_PATH` = `/var/audio/status` | out | `State`, `Volume`, `Track`, `Path`, `Position` (while not stopped), `Repeat`, `Shuffle`, `Playlist: N tracks` lines. |
| `AUDIO_POSITION_PATH` = `/var/audio/position` | out | `<position_ms> <duration_ms> <path>` for the Music Player's progress bar. |

Producers: the `music` terminal command (`oasis-terminal/src/audio_commands.rs`,
which appends so several requests in one frame all arrive), the Music
Player app (`play_file` / `stop`), and the MCP `play_media` tool.

The consumer is `oasis_audio::MusicController` (`controller.rs`), driven by
each host that runs the Music Player: the **desktop shell**
(`oasis-app/src/media_controller.rs`) and the **WASM backend**
(`oasis-backend-wasm/src/media.rs`). The host lends it its `AudioManager`,
the music track id and its `AudioBackend`; each tick the controller drains
every recognised line, feeds it to the manager, performs the returned
`MediaAction` on the backend and republishes position and status. Two
host-specific rules come in as arguments: `stream_shared` (desktop: TV Guide
audio streams through the same process-global SDL stream, so `stop` /
`pause` / `resume` are skipped while it is live) and a `before_play` hook
(both hosts stop internet radio before a track starts). The playlist is
every file played through `play_file`, in first-play order; `next` / `prev`
honour repeat and shuffle; `next` / `prev` while stopped only move the
selection.

`next` / `prev` while **paused** keep playback paused, as in common media
players: the old track is released and the new entry is *cued*
(`MediaAction::Cue`: shown in the status and at `0 0 <path>` in the
position file, nothing loaded); `resume` or `play` then starts it. While
playing they start the new entry.

A track that finishes on its own **auto-advances**: each tick the
controller checks for a loaded music track that the model says is playing
while `AudioBackend::is_playing()` is false, and applies
`AudioManager::track_finished()` (the `next` rules: repeat one replays,
repeat all wraps, shuffle follows the shuffle order, and the end of a
non-repeating playlist stops and releases the track). The check is skipped
while the stream is shared (`is_playing` then reports the other consumer).
Backends report the end through `is_playing()`: WASM once the `<audio>`
element has `ended`, SDL once a static track's queued PCM has drained.
Tuning internet radio releases the music track on both hosts first, so the
radio's takeover is never mistaken for the end of the track.

The UE5 FFI host does not service this IPC (it has no media controller),
so `music status` there prints "(audio subsystem not initialized)".

## Playlist semantics

`Playlist` keeps a `Vec<TrackInfo>` in insertion order.

Shuffle keeps a permutation of the track indices plus a cursor into it:

- `set_shuffle(true)` draws a fresh Fisher-Yates order (xorshift64, seeded
  from the clock; `Playlist::with_seed` makes it reproducible for tests)
  with the **current track first**, so one round plays every other track
  exactly once. Starting from a not-yet-started playlist plays the order's
  first track.
- Picking a track (`set_current`, e.g. `music play <file>`) that has not
  played yet this round moves it up to play now; the rest of the round is
  kept. Picking an already-played one continues from its slot.
- `add()` inserts the new track at a random point in the rest of the
  round; `remove()` edits the order in place (removing the current track
  moves to its shuffle successor). Neither reshuffles what already played.
- With repeat `All` the round wraps to the start of the same order, so
  `prev` undoes `next` across the wrap.
- The flag is private (`shuffle()` / `set_shuffle()`), so the order can
  never be out of step with it.

Repeat modes:

- `Off` — `advance()` returns false at the end, so the manager transitions to
  `Stopped`.
- `All` — `advance()` wraps from the last index (or the last shuffle slot)
  back to the first.
- `One` — both `advance()` and `go_back()` return the same index, so the
  same track is replayed on `next` / `prev`.

## Decoding pipeline

There is **no MP3 decoder in this crate.** `AudioBackend::load_track(data)`
takes raw bytes and the backend decides what to do with them — the SDL3
backend on desktop, `sceAudiocodec` on PSP, WebAudio on WASM, the engine mixer on UE5.

The crate does ship two format helpers used by tests and a few backends:

- `wav.rs` — PCM WAV reader. Integer PCM only (format tag 1, or
  `WAVE_FORMAT_EXTENSIBLE` with the PCM SubFormat), 8-bit unsigned or
  16-bit little-endian signed, mono or stereo, a sample rate within
  `wav::SAMPLE_RATES` (1 kHz - 768 kHz: a 1 Hz header would make the SFX
  resampler ask for 44100 output samples per input sample). The
  reader returns `Option<WavData>`; any unsupported format, bit depth, or
  channel count (and any malformed RIFF / `fmt ` / `data` chunk, including
  a `fmt ` chunk shorter than its 16 fixed bytes) yields `None` rather than
  partial or corrupted PCM. Callers must treat `None` as "fall through to
  the next decoder".
- `ogg.rs` — Ogg/Vorbis decoding via `symphonia` (`ogg.rs`). Returns
  interleaved `i16` PCM labelled with the decoded stream's sample rate and
  channel count (the header's, or 44.1 kHz / stereo, only if nothing
  decodes). Decoding stops at `MAX_DECODED_SAMPLES` (2^27 samples, ~25 min
  of 44.1 kHz stereo): a tiny Vorbis packet can expand to thousands of
  frames, so a small hostile file could otherwise decode to gigabytes.

`Cargo.toml` enables `symphonia/ogg` and `symphonia/vorbis` only — `mp3` is
deliberately not enabled because backends carry their own MP3 path.

ID3 tag parsing is **not implemented**. `TrackInfo::from_path` falls back to
the filename when no metadata is supplied (`types.rs`); populating tags is
the caller's responsibility (often handled at the radio source layer for ICY
metadata, or left blank for local files).

## Streaming and back-pressure

For continuous streams (radio, video audio track) the backend exposes:

| Trait method | Source | Purpose |
| --- | --- | --- |
| `load_streaming()` | backend/audio.rs | Allocate a streaming track id. |
| `feed_data(track, bytes)` | backend/audio.rs | Push a chunk of compressed bytes. |
| `streaming_can_accept(track)` | backend/audio.rs | "Is there room in the queue?" — back-pressure signal. |
| `finalize_streaming(track)` | backend/audio.rs | Signal EOF; flushes the lookahead buffer. |
| `feed_pcm_f32(samples, ch, sr)` | backend/audio.rs | Direct PCM path, used by the software video decoder. |

`RadioManager::tick` (`radio/mod.rs`) pumps a stream cooperatively: each
host frame it asks `streaming_can_accept`, and only if the answer is yes does
it pull the next chunk from the source via `RadioSource::poll`. The default
chunk size is 4 KiB, which at 60 fps gives ~240 KiB/s headroom — enough for a
~128 kbps MP3 stream with margin. The single-poll-per-frame discipline avoids
the sawtooth buffer pattern that emerges when SDL3 / PulseAudio drains in
bursts.

The stub backend's `streaming_can_accept` threshold is 16 KiB. Real
backends pick a value sized to their mixer buffer.

## Radio sources

`RadioSource` (`radio/source.rs`) is the source-side interface:

```rust
pub trait RadioSource {
    fn poll(&mut self) -> Result<Option<AudioChunk>>;
    fn disconnect(&mut self);
    fn state(&self) -> SourceState;
    fn source_type(&self) -> &str;
    fn streaming_url(&self) -> Option<&str> { None }
}
```

`poll` returns `Ok(None)` when no data is ready yet. The network sources
read from a non-blocking `NetworkStream`, so an empty socket is the normal
case, not an error: they test `OasisError::is_would_block()` (an
`io::ErrorKind::WouldBlock`), never the error's display text,
which on a plain TCP socket is the OS message ("Resource temporarily
unavailable") and does not say "would block". The loopback tests in
`source.rs` (`icecast_source_survives_os_would_block_on_plain_tcp`) drive a
real `StdNetworkBackend` socket to keep it that way. `TimedOut` is a dead
connection, not "no data yet", and fails the source. Response headers are
capped at `MAX_HEADER_BYTES` (16 KiB): a server that never sends the blank
line ending them fails the source instead of growing the header buffer.

A peer that disappears without closing the connection only ever yields
`WouldBlock`, so `ThreadedSource`'s pump drops a source that produces nothing
for `threaded::STALL_TIMEOUT` (20 s) while its readahead queue has room
(time paused on a full queue does not count) and surfaces
`stream stalled: ...` as the error. Request writes give up after
10 s of `WouldBlock`.

`streaming_url` is the WASM / UE5 escape hatch — when set, the backend may
hand the URL to a native streaming player (HTML5 `<audio>` for WASM) instead
of pumping bytes through `feed_data`.

Both network sources parse the response head with the shared
`parse_http_head` (`radio/source/http.rs`): it waits for `\r\n\r\n` in the raw bytes
(so partial reads and non-UTF-8 bodies are fine) and returns the status code
(from `HTTP/1.x NNN` or Shoutcast's `ICY NNN`), case-insensitive header
lookup, and the body offset.

### IcecastSource

- HTTP/1.0 GET with `Icy-MetaData: 1` so the server interleaves stream titles.
- Both sources build their request with one `get_request` helper that refuses
  a host or path containing whitespace or control bytes, and
  `StreamUrl::parse` / `resolve_redirect` refuse such URLs and `Location`s
  up front: a CR / LF in a station URL or a redirect would otherwise end the
  request line and inject headers.
- Only a 2xx status starts playback. A 3xx with `Location` fails with
  `redirect:<url>` (same convention as `ArchiveSource`; the location is also
  available as `IcecastSource::redirect_location()`), any other status
  with `HTTP <code>`, and a garbled status line with an error, so an error
  page's body is never fed to the decoder as audio.
- `radio::connect_icecast(url, connect)` (`connect.rs`) is the desktop entry
  point: it opens each hop through the caller's `connect` closure (TCP, plus
  TLS for `https`), waits up to 15 s for the response head and follows up to
  `MAX_REDIRECTS` (5) redirects. Absolute and relative `Location`s are
  accepted, same-scheme or `http` -> `https` only; an `https` -> `http`
  downgrade is refused before anything connects to it. Audio bytes that
  arrived with the head are kept for the first `poll()`.
- Parses `icy-metaint` from response headers (`icy::parse_icy_metaint`).
- `IcyDemuxer` (`icy.rs`) splits audio from metadata blocks and surfaces
  `StreamTitle='Artist - Song'` updates. Control characters are stripped
  from titles (here and again in `RadioManager::feed_audio`, which also
  covers archive.org titles), so a remote title cannot forge lines of the
  line-based status file.

### ArchiveSource

- HTTP/1.1 GET against archive.org.
- Handles 301 / 302 / 307 by encoding the new URL into the error string as
  `redirect:<url>`. `ArchiveSource` itself does not follow redirects; the
  desktop shell's `net_fetch::connect_archive_source` does, at most 3 hops
  per attempt, only to `archive.org` hosts, always over TLS (an `http://` CDN
  `Location` is upgraded) and resolving relative `Location`s against the
  current host. `RadioManager::resume_from_redirect` just clears the error;
  anything else that drives it must cap the hop count itself.
- 4xx and 5xx responses kill the source, and so does a 3xx without a
  `Location` (its body is an HTML stub, not audio).
- `ArchiveCatalog::parse_search_response` drops item identifiers outside
  `[A-Za-z0-9._-]` (they go into request paths unencoded), and
  `percent_encode` encodes control bytes in file names.
- Tracks `Content-Length` and ends the source once that many body bytes
  arrived.
- Emits metadata once on the first chunk, populated from a creator/title
  pre-fetch.

### VfsSource

In-memory byte buffer, configurable chunk size (default 4 KiB). Used by tests
and by UE5 asset streaming.

### Buffering and track end

`RadioManager` buffers `BUFFER_THRESHOLD` (32 KiB) before it starts the
streaming track. A source that ends before that much arrived (a very short
archive clip) is started with what it delivered and then finalized, rather
than discarded; either way the manager moves to `TrackEnded` for the caller
to auto-advance.

### Reconnection

There is **no automatic reconnect.** When `RadioSource::poll` returns an error
the `RadioManager` drops the source and transitions to `Error`
(`radio/mod.rs`). The caller (UI or terminal command) creates a new
source to retry.

## Sample rate and channel assumptions

- WAV: whatever the file says, within `wav::SAMPLE_RATES` (1-768 kHz).
  `mixer::resample_nearest_i16` (the SFX resampler) also caps its output at
  `MAX_RESAMPLED_SAMPLES` (2^27) whatever the input rate.
- OGG: whatever symphonia reports; defaults to 44.1 kHz / stereo if missing.
- Radio: feed raw bytes to the backend; the backend's decoder picks rate and
  channels.
- Software video decoder: pushes PCM `f32` via `feed_pcm_f32` with explicit
  rate and channel count.

The backend is responsible for resampling and channel mixing if it needs to
match the output device.

## Threading

`oasis-audio` itself spawns no threads. All decoding happens synchronously on
the caller's thread inside `backend.load_track()` and friends. The backends
that need a worker (PSP feeds `sceAudio` from a dedicated thread; SDL3
drains its audio stream on SDL's own audio thread) handle that internally and expose
`streaming_can_accept` so the host knows when to stop pushing.

## Cargo features

`oasis-audio` has no codec-selection features — the crate-level dependencies
(symphonia ogg + vorbis) are always enabled. MP3 codec selection happens at
the backend level, not here.

## Common failure modes

| Symptom | Root cause | What to check |
| --- | --- | --- |
| Stream cuts out periodically | `streaming_can_accept` returns false too aggressively. | Backend's mixer buffer threshold; `BUFFER_THRESHOLD` in `radio/mod.rs`. |
| Track plays as silence | Backend rejected the byte slice (unsupported format). | Backend's `load_track` log; codec features. |
| ICY title never updates | `icy-metaint` header missing on the server. | Confirm server actually sends it via `curl -I -H "Icy-MetaData: 1"`. |
| Archive stream stops on every track | Source disconnects on EOF; no auto-reconnect. | Caller must build a new `ArchiveSource`. |
| UE5 stream playback wrong rate | UE5 backend uses `streaming_url` path; rate negotiation is the engine's. | Check the URL plumbing, not `feed_data`. |

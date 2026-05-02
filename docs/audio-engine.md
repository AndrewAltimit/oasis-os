# Audio Engine

`oasis-audio` is the playlist + radio + format-glue layer that sits between
apps (Music Player, Internet Radio, video apps) and the per-target
`AudioBackend` implementations. Decoding lives almost entirely on the backend
side; this crate's job is sequencing, metadata, and streaming back-pressure.

The `AudioBackend` trait is defined in
`oasis-types/src/backend/audio.rs`. Backends (SDL3 via `rodio`, WASM via
WebAudio, UE5 via the engine mixer, PSP via `sceAudio` + `sceAudiocodec`)
implement it.

## Public types

All in `crates/oasis-audio/src/`.

- `AudioManager` (`manager.rs:19`) — top-level facade. Holds a `Playlist`,
  current volume, and `PlaybackState`, and delegates actual playback to an
  `AudioBackend`.
- `Playlist` (`playlist.rs`) — ordered tracks plus shuffle/repeat state.
- `TrackInfo` (`types.rs:8`) — title, artist, duration, VFS path, builder
  setters.
- `PlaybackState` (`types.rs:54`) — `Stopped`, `Playing`, `Paused`.
- `RepeatMode` (`types.rs:75`) — `Off`, `All`, `One`. Case-insensitive parse.
- `RadioManager` (`radio/mod.rs`) plus `RadioSource` trait and three sources:
  `IcecastSource`, `ArchiveSource`, `VfsSource`.

## AudioManager API

| Method | Source | Notes |
| --- | --- | --- |
| `play()` / `pause()` / `resume()` / `stop()` | manager.rs:73–114 | Standard transport. |
| `next()` / `prev()` | manager.rs:117–144 | Honour shuffle and repeat. |
| `set_repeat(mode)` | manager.rs:147 | Off / All / One. |
| `toggle_shuffle()` | manager.rs:154 | Reshuffles the playlist deterministically. |
| `set_volume(0..=100)` | manager.rs:49 | Clamped at 100. |
| `add_track_from_vfs(path)` | manager.rs:55 | Reads bytes via VFS, hands to `backend.load_track`. |
| `process_request(cmd)` | manager.rs:191 | Handles VFS IPC requests under `/var/audio/`. |
| `publish_status()` | manager.rs:183 | Writes `/var/audio/status` for headless drivers. |

The VFS IPC pattern (`/var/audio/request` in, `/var/audio/status` out) is what
the `music` terminal command uses, and what the WASM backend listens on.

## Playlist semantics

`Playlist` keeps a `Vec<TrackInfo>` in insertion order (`playlist.rs:13`).

Shuffle (`playlist.rs:186`) is a deterministic Fisher-Yates variant: split the
list in half, interleave, repeat. It is **not** cryptographically random — it
just needs to be reproducible for tests. The shuffle order is rebuilt whenever
you toggle shuffle on or add a new track.

Repeat modes:

- `Off` — `advance()` returns false at the end, so the manager transitions to
  `Stopped` (`playlist.rs:141`).
- `All` — `advance()` wraps from the last index back to 0
  (`playlist.rs:137`).
- `One` — both `advance()` and `go_back()` return the same index, so the
  current track loops on natural completion (`playlist.rs:119`, `154`).

## Decoding pipeline

There is **no MP3 decoder in this crate.** `AudioBackend::load_track(data)`
takes raw bytes and the backend decides what to do with them — `rodio` on
desktop, `sceAudiocodec` on PSP, WebAudio on WASM, the engine mixer on UE5.

The crate does ship two format helpers used by tests and a few backends:

- `wav.rs` — PCM WAV reader. Format tag 1 only, 8-bit unsigned or 16-bit
  little-endian signed, mono or stereo (`wav.rs:38`–73).
- `ogg.rs` — Ogg/Vorbis decoding via `symphonia` (`ogg.rs:65`). Returns
  interleaved `i16` PCM. Sample rate and channel layout come from the
  codec params; defaults to 44.1 kHz / stereo when missing.

`Cargo.toml` enables `symphonia/ogg` and `symphonia/vorbis` only — `mp3` is
deliberately not enabled because backends carry their own MP3 path.

ID3 tag parsing is **not implemented**. `TrackInfo::from_path` falls back to
the filename when no metadata is supplied (`types.rs:24`); populating tags is
the caller's responsibility (often handled at the radio source layer for ICY
metadata, or left blank for local files).

## Streaming and back-pressure

For continuous streams (radio, video audio track) the backend exposes:

| Trait method | Source | Purpose |
| --- | --- | --- |
| `load_streaming()` | backend/audio.rs:56 | Allocate a streaming track id. |
| `feed_data(track, bytes)` | backend/audio.rs:62 | Push a chunk of compressed bytes. |
| `streaming_can_accept(track)` | backend/audio.rs:75 | "Is there room in the queue?" — back-pressure signal. |
| `finalize_streaming(track)` | backend/audio.rs:84 | Signal EOF; flushes the lookahead buffer. |
| `feed_pcm_f32(samples, ch, sr)` | backend/audio.rs:92 | Direct PCM path, used by the software video decoder. |

`RadioManager::tick` (`radio/mod.rs:317`) pumps a stream cooperatively: each
host frame it asks `streaming_can_accept`, and only if the answer is yes does
it pull the next chunk from the source via `RadioSource::poll`. The default
chunk size is 4 KiB, which at 60 fps gives ~240 KiB/s headroom — enough for a
~128 kbps MP3 stream with margin. The single-poll-per-frame discipline avoids
the sawtooth buffer pattern that emerges when SDL3 / PulseAudio drains in
bursts.

The stub backend's `streaming_can_accept` threshold is 16 KiB
(`radio/source.rs:562`). Real backends pick a value sized to their mixer
buffer.

## Radio sources

`RadioSource` (`radio/source.rs:36`) is the source-side interface:

```rust
pub trait RadioSource {
    fn poll(&mut self) -> Option<AudioChunk>;
    fn disconnect(&mut self);
    fn state(&self) -> SourceState;
    fn source_type(&self) -> &'static str;
    fn streaming_url(&self) -> Option<&str> { None }
}
```

`streaming_url` is the WASM / UE5 escape hatch — when set, the backend may
hand the URL to a native streaming player (HTML5 `<audio>` for WASM) instead
of pumping bytes through `feed_data`.

### IcecastSource (`radio/source.rs:127`)

- HTTP/1.0 GET with `Icy-MetaData: 1` so the server interleaves stream titles.
- Parses `icy-metaint` from response headers (`icy.rs:17`).
- `IcyDemuxer` (`icy.rs:62`) splits audio from metadata blocks and surfaces
  `StreamTitle='Artist - Song'` updates.
- Stateful header parser accumulates bytes into `header_buf` until `\r\n\r\n`
  (`source.rs:196`), so partial reads are fine.

### ArchiveSource (`radio/source.rs:298`)

- HTTP/1.1 GET against archive.org.
- Handles 301 / 302 / 307 by encoding the new URL into the error string as
  `redirect:<url>` (`source.rs:456`); the manager re-issues against the new
  URL.
- 4xx and 5xx responses kill the source.
- Tracks `Content-Length` (`source.rs:495`).
- Emits metadata once on the first chunk, populated from a creator/title
  pre-fetch (`source.rs:472`).

### VfsSource (`radio/source.rs:62`)

In-memory byte buffer, configurable chunk size (default 4 KiB). Used by tests
and by UE5 asset streaming.

### Reconnection

There is **no automatic reconnect.** When `RadioSource::poll` returns an error
the `RadioManager` drops the source and transitions to `Error`
(`radio/mod.rs:313`). The caller (UI or terminal command) creates a new
source to retry.

## Sample rate and channel assumptions

- WAV: whatever the file says.
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
that need a worker (PSP feeds `sceAudio` from a dedicated thread; SDL3 lets
`rodio` own its mixer thread) handle that internally and expose
`streaming_can_accept` so the host knows when to stop pushing.

## Cargo features

`oasis-audio` has no codec-selection features — the crate-level dependencies
(symphonia ogg + vorbis) are always enabled. MP3 codec selection happens at
the backend level, not here.

## Common failure modes

| Symptom | Root cause | What to check |
| --- | --- | --- |
| Stream cuts out periodically | `streaming_can_accept` returns false too aggressively. | Backend's mixer buffer threshold; `BUFFER_THRESHOLD` in `source.rs`. |
| Track plays as silence | Backend rejected the byte slice (unsupported format). | Backend's `load_track` log; codec features. |
| ICY title never updates | `icy-metaint` header missing on the server. | Confirm server actually sends it via `curl -I -H "Icy-MetaData: 1"`. |
| Archive stream stops on every track | Source disconnects on EOF; no auto-reconnect. | Caller must build a new `ArchiveSource`. |
| UE5 stream playback wrong rate | UE5 backend uses `streaming_url` path; rate negotiation is the engine's. | Check the URL plumbing, not `feed_data`. |

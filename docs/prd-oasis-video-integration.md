# PRD: oasis-video Integration

> **Status**: Partially complete (as of 2026-03-14). Desktop streaming (Phases 1-2) and UE5 (Phase 4) are fully working. PSP video decode (Phase 3) has audio streaming working but H.264 ME decode and memory/FPS targets are pending. Some Phase 5 quality items remain. This document is retained for historical reference.

## Status: Streaming Cross-Platform Video Decode (Phase 1-2 Complete)

### Problem

`oasis-video` is a pure-Rust MP4/H.264+AAC software decoder included in the
workspace but imported by nothing. Current video playback varies by platform:

| Platform | Current Approach | Issues |
|----------|-----------------|--------|
| **SDL3 desktop** | In-process streaming decode via `StreamingBuffer` + oasis-video (ffmpeg fallback available) | No external dependencies required |
| **WASM** | Hidden `<video>` element + canvas `drawImage` capture | Works but archive.org CORS blocks fetch() |
| **PSP** | Stub only (thread skeleton, no decode) | No playback at all |
| **UE5** | Nothing | No video support |

The current `SoftwareVideoDecoder::open(mp4_data: Vec<u8>)` API requires
loading the entire MP4 into memory. A typical Internet Archive video
(480×272, 25 min, H.264 @ 1500kbps) is ~280 MB — far exceeding PSP's 24 MB
user heap and wasteful even on desktop. **Streaming must be a first-class
architectural concern**, not an afterthought.

---

## Current oasis-video API

```rust
pub struct SoftwareVideoDecoder {
    pub fn open(mp4_data: Vec<u8>) -> Result<Self, VideoError>
    pub fn next_video_frame(&mut self) -> Result<Option<VideoFrame>, VideoError>
    pub fn next_audio_samples(&mut self) -> Result<Option<AudioChunk>, VideoError>
    pub fn seek(&mut self, secs: f64) -> Result<(), VideoError>
    pub fn video_size(&self) -> (u32, u32)
    pub fn audio_format(&self) -> (u32, u16)
}

pub struct VideoFrame {
    pub rgba: Vec<u8>, pub width: u32, pub height: u32, pub timestamp_secs: f64
}
pub struct AudioChunk {
    pub pcm_f32: Vec<f32>, pub channels: u16, pub sample_rate: u32, pub timestamp_secs: f64
}
```

**Internal stack:**
- `demux.rs`: symphonia 0.5 `MediaSourceStream` wrapping `Cursor::new(data)` —
  accepts any `Box<dyn Read + Seek>`, not just `Vec<u8>`
- `h264.rs`: openh264 0.9 (optional `h264` feature, C library)
- `aac.rs`: symphonia's AAC codec → interleaved f32 PCM via `SampleBuffer`
- `yuv.rs`: pure-Rust BT.601 YUV420→RGBA fallback

**Key insight:** symphonia's `MediaSourceStream::new()` already accepts
`Box<dyn MediaSource>` (which is `Read + Seek + Send`). The `Vec<u8>`
requirement is only in the public API wrapper, not an internal limitation.

---

## Why It's Orphaned

1. Created for WASM fallback when browser H.264 codec unavailable, but
   archive.org CDN lacks CORS headers → `fetch()` blocked → fallback never fired
2. `h264` feature can't compile to wasm32 (openh264-sys has no prebuilt wasm)
3. ffmpeg subprocess (SDL) and `<video>` element (WASM) both work fine
4. PSP blocked on symphonia pulling `std::sync::Once` → PPSSPP HLE gap
5. API designed as `Vec<u8>` load-all — unsuitable for constrained platforms

---

## Design Goals

1. **Streaming-first:** Never require the full MP4 in memory. Demux from
   disk-backed or network-backed `Read + Seek` sources.
2. **Progressive playback:** Start playing before download completes (desktop).
   Download to disk first, then stream from disk (PSP).
3. **Platform-appropriate strategies:** Each platform uses the best available
   approach — software decode, hardware decode, or native element.
4. **Graceful fallback:** ffmpeg on desktop, `<video>` on WASM, stub on
   unsupported platforms.
5. **Bounded memory:** Fixed-size decode buffers. No unbounded allocations
   proportional to file size.

---

## Streaming Architecture

### Core Abstraction: `VideoSource` Trait

```rust
/// Media source that can be opened from various backends.
/// Wraps symphonia's MediaSource requirement (Read + Seek + Send).
pub trait VideoSource: std::io::Read + std::io::Seek + Send + 'static {}

impl<T: std::io::Read + std::io::Seek + Send + 'static> VideoSource for T {}
```

### New Public API

```rust
pub struct SoftwareVideoDecoder { /* ... */ }

impl SoftwareVideoDecoder {
    /// Open from any seekable source (file, memory cursor, etc.)
    pub fn open_stream(source: Box<dyn VideoSource>) -> Result<Self, VideoError>;

    /// Legacy: open from in-memory buffer (wraps open_stream with Cursor)
    pub fn open(mp4_data: Vec<u8>) -> Result<Self, VideoError>;

    /// Frame pull API (unchanged)
    pub fn next_video_frame(&mut self) -> Result<Option<VideoFrame>, VideoError>;
    pub fn next_audio_samples(&mut self) -> Result<Option<AudioChunk>, VideoError>;
    pub fn seek(&mut self, secs: f64) -> Result<(), VideoError>;
    pub fn video_size(&self) -> (u32, u32);
    pub fn audio_format(&self) -> (u32, u16);
}
```

### Platform Source Strategies

| Platform | VideoSource Implementation | Memory Ceiling |
|----------|---------------------------|----------------|
| SDL3 desktop | `StreamingBuffer` (in-memory sliding window fed by download thread) | ~16-32 MB sliding window |
| PSP | `File` from Memory Stick (`ms0:/PSP/GAME/OASISOS/tv_cache.mp4`) | ~4 MB decode buffers |
| UE5 | `File` or `Cursor<Vec<u8>>` (host provides data) | ~8 MB decode buffers |
| WASM | N/A (uses native `<video>` element) | — |

### Memory Budget Breakdown

**Desktop (SDL3):**
```
Decode buffers:
  - 2× RGBA frame buffer (480×272×4)  = ~1.0 MB
  - Audio ring buffer (1s @ 44.1kHz)  = ~0.35 MB
  - Demux packet buffer (64 packets)  = ~2.0 MB
  - symphonia internal buffers        = ~1.0 MB
  Total decode overhead               ≈ 4.5 MB (fixed)

Optional progressive HTTP cache:
  - LRU page cache (256× 64KB pages)  = ~16 MB
  Total with progressive              ≈ 20 MB (fixed)
```

**PSP:**
```
Decode buffers:
  - 1× RGBA frame buffer (480×272×4)  = ~0.5 MB
  - Audio ring buffer (32KB)           = 0.03 MB
  - Demux read buffer (64KB)           = 0.06 MB
  - NAL unit buffer (256KB)            = 0.25 MB
  Total decode overhead                ≈ 1.0 MB (fixed)

Remaining for application:
  - PSP user heap: 24 MB
  - Video overhead: ~1 MB
  - Available for shell + UI: ~23 MB
```

---

## Integration Plan

### Phase 1: Streaming API Refactor (P0, 2 tasks)

**Task 1a — Add `open_stream()` to `SoftwareVideoDecoder`:**
- Change `demux.rs` to accept `Box<dyn VideoSource>` instead of `Vec<u8>`
- symphonia's `MediaSourceStream` already accepts `Box<dyn Read + Seek>` —
  this is a thin wrapper change
- Keep `open(Vec<u8>)` as convenience wrapper: `open_stream(Box::new(Cursor::new(data)))`
- Add `VideoSource` trait (blanket impl over `Read + Seek + Send`)

**Task 1b — Feature flags in workspace:**
```toml
# oasis-app/Cargo.toml
[features]
default = ["video-decode"]
video-decode = ["oasis-video/h264"]

# Cargo.toml (workspace)
[workspace.dependencies]
oasis-video = { path = "crates/oasis-video" }
```
- Wrap all oasis-video call sites in `#[cfg(feature = "video-decode")]`

**Acceptance criteria:**
- `open_stream(Box::new(File::open("test.mp4")?))` works without loading file to memory
- `open(vec_data)` still works (backwards compatible)
- Feature flag compiles cleanly with and without `video-decode`

### Phase 2: SDL3 Desktop Integration (P1, 4 tasks)

**Task 2a — Refactor `video_player.rs` with backend enum:**
```rust
enum DecodeBackend {
    Ffmpeg { video: Child, audio: Child },
    Software(SoftwareVideoDecoder),
}
```
- When `video-decode` feature enabled: try oasis-video first
- Fallback to ffmpeg when feature disabled or oasis-video fails
- Keep identical public API: `start()`, `tick()`, `stop()`

**Task 2b — Download-to-disk pipeline:**
- Download MP4 to temp file (`/tmp/oasis_tv_XXXX.mp4`) using existing HTTP client
- Open `File` handle → pass to `SoftwareVideoDecoder::open_stream()`
- Progress indicator in TV Guide UI during download
- Cache path in `MemoryVfs` metadata for replay without re-download
- Clean up temp files on `stop()` or process exit

**Task 2c — Progressive HTTP playback (stretch goal):**
- `HttpRangeFile` struct implementing `Read + Seek`:
  - Sends HTTP Range requests on `read()` calls
  - LRU page cache (256× 64KB = 16 MB) for recently accessed regions
  - `seek()` updates internal offset, next `read()` fetches new range
- Requires server to support `Accept-Ranges: bytes` (archive.org does)
- Enables play-while-downloading: symphonia reads moov atom first, then
  streams mdat chunks on demand
- Fallback: if Range not supported, fall back to full download (Task 2b)

**Task 2d — Audio/video sync + seeking:**
- Use `VideoFrame.timestamp_secs` and `AudioChunk.timestamp_secs` for A/V sync
- Audio buffer queue: `mpsc::channel` with 8-frame capacity (same pattern as ffmpeg path)
- Presentation clock: track wall-clock time vs stream time, skip/duplicate frames to sync
- Seek: call `decoder.seek(secs)`, flush audio device + video texture, resume decode
- Target: A/V sync within ±50ms, seek latency <500ms

**Acceptance criteria:**
- TV Guide video plays with oasis-video, no ffmpeg binary needed
- Memory usage stays under 25 MB regardless of video file size
- Audio/video synchronized within ±50ms
- Seek works accurately within keyframe boundaries
- ffmpeg fallback still works when `video-decode` feature disabled

### Phase 3: PSP Backend (P2, 5 tasks)

**Context:** PSP has 24 MB user heap, existing 20 MB download size cap, and
proven streaming patterns (32KB audio buffer with 4KB refill chunks). symphonia
cannot be used on PSP due to `std::sync::Once` dependency that PPSSPP HLE
doesn't implement.

**Task 3a — Lightweight `no_std` MP4 demuxer (`demux_lite.rs`):**
- Manual MP4 box parser: ftyp, moov (mvhd, trak, stbl tables), mdat
- Reads from `Read + Seek` source (file on Memory Stick)
- Outputs: iterator of `NalUnit(&[u8])` and `AacFrame(&[u8])` with timestamps
- Parse stbl tables (stco/co64, stsz, stss, stsc, stts, ctts) for random access
- No heap allocation for box traversal — stack-based state machine
- Target: <5 KB code size

**Task 3b — PSP Media Engine H.264 decode (preferred path):**
- Use `sceVideocodecInit` / `sceVideocodecDecode` for hardware H.264
- Feed NAL units from `demux_lite` → ME → decoded YCbCr frames
- Convert ME output to PSP display format (ABGR 8888 or RGB 565)
- ME runs on dedicated coprocessor — frees main CPU for UI rendering
- Note: PPSSPP emulates `sceVideocodec` partially; test with real hardware too

**Task 3c — PSP `sceAudiocodec` AAC decode:**
- Use `sceAudiocodecInit` / `sceAudiocodecDecode` for hardware AAC
- Feed raw AAC frames from `demux_lite` → hardware decoder → PCM
- Output to existing PSP audio channel (32KB ring buffer, 4KB refill chunks)
- Follow proven audio streaming pattern from `audio.rs`

**Task 3d — Fallback: openh264 software decode on PSP:**
- Only if Media Engine unavailable or PPSSPP doesn't emulate `sceVideocodec`
- openh264 compiles for MIPS (needs cross-compilation verification)
- Much slower than ME — target 10fps at 320×240

**Task 3e — Wire into `video.rs` thread infrastructure:**
- Replace stub `VideoCmd` handler with real decode loop:
  ```rust
  // Existing infrastructure (already implemented):
  SpscQueue<VideoCmd, 4>  // Play, Stop, Shutdown commands
  SpscQueue<DecodedFrame, 2>  // Double-buffered frame output
  ```
- Download flow: HTTP GET → Memory Stick file → `demux_lite` → ME decode
- Download path: `ms0:/PSP/GAME/OASISOS/tv_cache.mp4`
- Thread priorities: video decode = 24, audio output = 16 (existing convention)
- Frame pacing: decode at video framerate, present via `sceDisplaySetFrameBuf`

**PSP video pipeline:**
```
┌──────────┐    ┌───────────────┐    ┌────────────┐    ┌──────────┐
│ HTTP GET │───→│ Memory Stick  │───→│ demux_lite │───→│ ME h264  │
│ (≤20 MB) │    │ tv_cache.mp4  │    │ (no_std)   │    │ hardware │
└──────────┘    └───────────────┘    └─────┬──────┘    └────┬─────┘
                                           │                │
                                     ┌─────▼──────┐   ┌────▼─────┐
                                     │ sceAudio   │   │ Display  │
                                     │ AAC decode │   │ framebuf │
                                     └────────────┘   └──────────┘
```

**Acceptance criteria:**
- Video plays on PSP with ≤1 MB decode buffer overhead
- Download size enforced at ≤20 MB (existing cap)
- Frame rate ≥10fps at 480×272 with ME, ≥10fps at 320×240 with software fallback
- Audio streaming uses existing 32KB buffer pattern — no new allocation patterns
- Graceful failure if video too large or decode unsupported

### Phase 4: UE5 Backend (P3, 2 tasks)

**Task 4a — Background decode thread:**
- Spawn thread with `SoftwareVideoDecoder::open_stream(File)`
- Double-buffer RGBA frames via `Arc<Mutex<Option<VideoFrame>>>`
- UE5 host calls `oasis_tick()` → check for new frame → blit to framebuffer

**Task 4b — FFI extensions:**
```c
// New C-ABI functions
void oasis_video_play(const char* path);
void oasis_video_stop();
int  oasis_video_is_playing();
```

### Phase 5: Testing & Quality (P1, 3 tasks)

**Task 5a — Integration test suite:**
- Test `open_stream()` with `File`, `Cursor<Vec<u8>>`, and mock `Read + Seek`
- Test seek accuracy (before first keyframe, mid-stream, near end)
- Test A/V sync measurement (decode 100 frames, check timestamp drift)
- Test graceful error handling (truncated file, corrupt NAL units, missing codec)

**Task 5b — Memory profiling:**
- Desktop: valgrind/heaptrack run with 25-min video, verify ≤25 MB peak
- PSP: memory probe via `sceKernelTotalFreeMemSize()` during playback
- Test for leaks over 10-minute continuous playback session

**Task 5c — Benchmarks:**
- Decode throughput: frames/second for 480×272 H.264 @ various bitrates
- Seek latency: time from seek call to first decoded frame
- Memory: peak RSS during playback (desktop), heap delta (PSP)
- Compare oasis-video vs ffmpeg path on same content

### Phase 6: Documentation (P1, 1 task)

- Update CLAUDE.md crate dependency graph (oasis-video → oasis-app, oasis-backend-psp)
- Architecture diagram showing streaming pipeline per platform
- Document feature flags: `video-decode`, `h264`
- Document platform-specific behavior and fallback chains
- Document memory budgets and constraints per platform

---

## Platform Matrix (Detailed)

| Platform | Source | Demuxer | Video Codec | Audio Codec | Memory Budget | Priority |
|----------|--------|---------|-------------|-------------|---------------|----------|
| SDL3 | `File` (temp) or `HttpRangeFile` | symphonia | openh264 (software) | symphonia AAC | ~5 MB fixed + 16 MB HTTP cache | P1 |
| PSP | `File` (Memory Stick) | `demux_lite` (no_std) | Media Engine (HW) or openh264 | `sceAudiocodec` (HW) | ~1 MB fixed | P2 |
| UE5 | `File` (host filesystem) | symphonia | openh264 (software) | symphonia AAC | ~5 MB fixed | P3 |
| WASM | N/A | N/A (browser) | N/A (browser) | N/A (browser) | 0 (native element) | — |

---

## Streaming Implementation Details

### Desktop Streaming Architecture (Implemented)

The desktop TV Guide uses `StreamingBuffer` — a `Read + Seek` wrapper over a
shared sliding-window buffer fed by a background download thread. This enables
true progressive playback: video starts playing while the download continues.

**Key components:**
- `StreamingInner` — shared state with `Arc<Mutex<SlidingState>>`, `Condvar`,
  atomic `decoder_pos`, `bytes_received`, and `probe_mode` flag
- `StreamingBuffer` — implements `Read + Seek + Send` for symphonia's
  `MediaSourceStream`, backed by `StreamingInner`
- `stream_download_inner()` — download thread that feeds the sliding window
  with HTTP(S) data, following archive.org redirects

**Probe phase handling:**
symphonia probes the file to find the `moov` atom. During probe, the decoder
seeks to the end of the file. `probe_mode` flag causes reads to return zeros
(skipping mdat body instantly). Once probe completes, the download thread
detects moov discovery and restarts the download from the correct offset using
a Range request through archive.org (which 302-redirects to a fresh CDN node).

**Deferred tail probe:**
For moov-at-end files, a separate thread fetches the last 8MB. This is deferred
until >8MB of body data has been received without finding moov, avoiding CDN
connection throttling (archive.org throttles concurrent HTTPS connections).

**Backpressure via `should_throttle()`:**
The download thread pauses when it gets too far ahead of the decoder:
`decoder_pos > 0 ? received > decoder_pos + 16MB : has_moov && buf_size > 16MB`

**Prebuffer gate:**
Before seeking, the decoder waits for MIN_PREBUFFER (2MB) of body data to
accumulate, preventing seeks into empty buffer regions.

**CDN failover:**
Range requests route through the original archive.org URL (not cached CDN URL)
to get a fresh 302 redirect, avoiding 401 errors from stale CDN nodes.
`open_range_connection()` follows redirect chains automatically.

```
┌──────────────┐    ┌─────────────────┐    ┌──────────────────┐
│ Download     │───→│ StreamingInner  │←───│ StreamingBuffer  │
│ Thread       │    │ (sliding window)│    │ (Read + Seek)    │
│ HTTP(S) GET  │    │ + Condvar wake  │    │ for symphonia    │
└──────┬───────┘    └────────┬────────┘    └────────┬─────────┘
       │                     │                      │
       │ seek restart        │ moov discovery        │ probe_mode
       │ via Range req       │ triggers restart      │ returns zeros
       └─────────────────────┴──────────────────────┘
```

### PSP Download Strategy

```
1. TV Guide selects video via select_smallest_for() (≤20 MB cap)
2. HTTP GET → write to ms0:/PSP/GAME/OASISOS/tv_cache.mp4
3. Open file → demux_lite parser (no symphonia, no std::sync::Once)
4. Read moov → parse stbl tables on stack (no heap for box traversal)
5. Stream mdat: read 64 KB chunks → extract NAL units → feed to ME
6. Audio: extract AAC frames → sceAudiocodec → 32 KB ring buffer
```

The PSP never has more than ~1 MB of video data in RAM at once.
The full file sits on Memory Stick (up to 20 MB).

---

## TV Guide Integration Points

The TV Guide currently selects videos from Internet Archive via a
multi-stage quality fallback:

```rust
fn select_smallest_for(files: &[ArchiveFile]) -> Option<&ArchiveFile> {
    // Stage 1: h264 derivatives (preferred — MP4/H.264)
    // Stage 2: any file under size cap
    // Stage 3: smallest file overall
    // Stage 4: first available
}
```

**Integration changes:**
- Add `video-decode` feature check: if enabled, prefer h264 derivatives (Stage 1)
  since oasis-video can decode them natively
- If feature disabled, fall back to ffmpeg-compatible formats
- PSP: enforce 20 MB size cap strictly (existing behavior)
- Desktop: relax size cap when using progressive HTTP (file never fully in memory)

---

## Fallback Chain Per Platform

**SDL3 Desktop:**
```
1. video-decode feature ON + MP4 available → oasis-video (stream from disk)
2. video-decode feature ON + Range support → oasis-video (progressive HTTP)
3. video-decode feature OFF or decode error → ffmpeg subprocess (existing)
4. ffmpeg not on PATH → error message in TV Guide UI
```

**PSP:**
```
1. File ≤20 MB + ME available → demux_lite + sceVideocodec/sceAudiocodec
2. File ≤20 MB + ME unavailable → demux_lite + openh264 software decode
3. File >20 MB → skip (display "video too large" in UI)
4. Download fails → display error, return to guide
```

**UE5:**
```
1. video-decode feature ON → oasis-video in background thread
2. Feature OFF → no video (UE5 host can provide its own video pipeline)
```

**WASM:**
```
1. Browser <video> element (unchanged, always used)
2. No fallback needed — browser handles all codec negotiation
```

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| oasis-video decoder bugs on real content | Medium | High | ffmpeg fallback (desktop), test with 10+ archive.org samples |
| symphonia Read+Seek overhead vs Vec<u8> | Low | Low | Benchmark; Cursor path still available for small files |
| CDN connection throttling on concurrent requests | Medium | High | Deferred tail probe (8MB threshold) + sequential downloads (SOLVED) |
| CDN 401 Unauthorized on stale Range requests | Medium | High | Route Range requests through archive.org for fresh redirect (SOLVED) |
| Throttle deadlock (probe_mode decoder_pos race) | Medium | Critical | Skip decoder_pos updates during probe_mode reads (SOLVED) |
| PSP std::sync::Once blocker (symphonia) | Certain | — | Skip symphonia entirely on PSP, use demux_lite |
| PSP Media Engine not emulated in PPSSPP | High | Medium | openh264 software fallback (Task 3d) |
| openh264 cross-compile to MIPS | Medium | Low | Verify early; if blocked, ME-only on real HW |
| Audio sync drift over long playback | Medium | Medium | Timestamp-based resync every 5s, hard resync on seek |
| Memory Stick I/O latency during decode | Low | Medium | Read-ahead buffer (64 KB), ME handles decode latency |
| Large moov atom at file end (slow initial load) | Medium | Medium | Deferred tail probe fetches last 8MB; moov can be 1-4MB for long videos |
| Corrupt/truncated downloads on PSP | Medium | Medium | Verify file size post-download, hash check if available |

---

## Success Criteria

### Phase 1 (Streaming API):
- [x] `SoftwareVideoDecoder::open_stream()` accepts `Box<dyn VideoSource>`
- [x] `open(Vec<u8>)` still works (backwards compatible)
- [x] File-backed decode: memory usage independent of file size
- [x] Feature flag compiles cleanly on/off

### Phase 2 (SDL3 Desktop):
- [x] TV Guide video plays via oasis-video without ffmpeg installed
- [x] Streaming playback — video starts before download completes (StreamingBuffer)
- [x] Deferred tail probe prevents CDN connection throttling
- [x] CDN failover via redirect-following Range requests
- [x] Prebuffer gate (2MB) prevents decoder starvation on seek
- [x] Backpressure throttle prevents unbounded memory growth
- [ ] Peak memory ≤25 MB during 25-minute video playback
- [x] A/V sync within ±50ms over 10-minute session (PTS-based pacing)
- [x] Seek works within keyframe boundaries (<500ms latency)
- [x] ffmpeg fallback still works when feature disabled

### Phase 3 (PSP):
- [ ] Video plays on PSP with ≤1 MB decode overhead
- [x] Downloads to Memory Stick, streams from file (demux_lite + PspFileReader)
- [x] Media Engine hardware decode works (or software fallback) — sceVideocodec stubs, audio-only on PPSSPP
- [x] Audio uses existing 32KB streaming buffer pattern (sceAudiocodec AAC)
- [ ] ≥10fps at 480×272 (ME) or 320×240 (software)

### Phase 4 (UE5):
- [x] Background thread decode with frame handoff to host
- [x] C-ABI functions exposed for host control

### Phase 5 (Quality):
- [x] No memory leaks over 10-minute continuous session (ASAN + Valgrind CI)
- [ ] All error paths tested (truncated file, bad codec, oversized file)
- [x] Benchmark results documented (Criterion benchmarks)

---

## Files Affected

**Phase 1 (Streaming API):**
- `crates/oasis-video/src/lib.rs` — new `open_stream()`, `VideoSource` trait
- `crates/oasis-video/src/demux.rs` — accept `Box<dyn VideoSource>` instead of `Vec<u8>`
- `crates/oasis-video/Cargo.toml` — no changes expected
- `Cargo.toml` (workspace) — add oasis-video to workspace deps
- `crates/oasis-app/Cargo.toml` — feature flag + oasis-video dep

**Phase 2 (SDL3):**
- `crates/oasis-app/src/video_player.rs` — `DecodeBackend` enum, streaming decode
- `crates/oasis-app/src/tv_controller.rs` — download-to-disk pipeline (or guide.rs)
- `crates/oasis-app/src/http_range.rs` — new: `HttpRangeFile` (stretch goal)

**Phase 3 (PSP):**
- `crates/oasis-video/src/demux_lite.rs` — new: no_std MP4 box parser
- `crates/oasis-backend-psp/src/video.rs` — replace stub with ME/software decode
- `crates/oasis-backend-psp/Cargo.toml` — oasis-video dep (demux_lite only)

**Phase 4 (UE5):**
- `crates/oasis-ffi/src/lib.rs` — video control C-ABI functions

**Unchanged:**
- `crates/oasis-backend-wasm/src/video.rs` — browser native path stays
- `crates/oasis-core/` — agnostic to video backend
- `crates/oasis-video/src/h264.rs` — internal, no API change
- `crates/oasis-video/src/aac.rs` — internal, no API change
- `crates/oasis-video/src/yuv.rs` — internal, no API change

---

## Implementation Order & Dependencies

```
Phase 1: Streaming API (P0)
  ├── 1a: open_stream() API
  └── 1b: Feature flags
          │
Phase 2: SDL3 Desktop (P1) ─── depends on Phase 1
  ├── 2a: DecodeBackend enum
  ├── 2b: Download-to-disk
  ├── 2c: Progressive HTTP (stretch)
  └── 2d: A/V sync + seek
          │
Phase 3: PSP (P2) ─── depends on Phase 1 (feature flags only)
  ├── 3a: demux_lite (no_std) ─── independent of Phase 2
  ├── 3b: ME H.264 ─── depends on 3a
  ├── 3c: sceAudiocodec ─── depends on 3a
  ├── 3d: openh264 MIPS ─── independent fallback
  └── 3e: Wire video.rs ─── depends on 3a + (3b or 3d) + 3c
          │
Phase 4: UE5 (P3) ─── depends on Phase 1
  ├── 4a: Background thread
  └── 4b: FFI extensions
          │
Phase 5: Testing (P1) ─── ongoing from Phase 1
  ├── 5a: Integration tests
  ├── 5b: Memory profiling
  └── 5c: Benchmarks
          │
Phase 6: Documentation (P1) ─── after Phase 2
```

**Critical path:** Phase 1 → Phase 2 (SDL3 desktop is the first user-visible milestone)

**Parallel track:** Phase 3 (PSP) can begin `demux_lite` work as soon as Phase 1 feature
flags are in place, since PSP uses its own demuxer (not symphonia).

---

## Estimated Scope

- Phase 1: 2 tasks (small, API refactor + Cargo.toml changes)
- Phase 2: 4 tasks (medium, core desktop integration)
- Phase 3: 5 tasks (large, PSP hardware integration + custom demuxer)
- Phase 4: 2 tasks (small, thread + FFI wrapper)
- Phase 5: 3 tasks (medium, testing infrastructure)
- Phase 6: 1 task (small, documentation)
- **Total: 17 tasks across 6 phases**

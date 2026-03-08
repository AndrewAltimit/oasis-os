# Internet Archive TV Guide — Implementation Plan

## Overview

Build a retro TV guide app for OASIS OS that streams video content from Internet
Archive collections, organized into 5 channels with a deterministic time-based
schedule. The UI mimics an 80s-era cable TV guide (like the concept image) with a
grid of channels × time slots and a PIP (Picture-in-Picture) preview of the
currently playing content.

---

## Phase 1: Data Layer — Channel Configuration & IA Video Catalog

### 1.1 Channel Configuration File (`/etc/tv/channels.toml`)

Define a TOML schema for TV channels, stored in VFS:

```toml
[[channel]]
number = 2
call_sign = "RETRO"
name = "Retro Cartoons"
genre = "cartoons"
# Each source is an IA item (or collection) with video files
[[channel.source]]
item_id = "adventures-of-sonic-the-hedgehog-01-x-44-the-mystery-of-the-missing-hi-tops_202402"
subfolder = "AOSTH Episodes (+ Special and Pilot)"
media_type = "video"

[[channel]]
number = 5
call_sign = "TECH"
name = "Tech & Bytes"
genre = "technology"
[[channel.source]]
item_id = "bits-and-bytes-yt"
subfolder = "Bits-and-Bytes"
media_type = "video"

[[channel]]
number = 8
call_sign = "GAME"
name = "Gaming"
genre = "gaming"
[[channel.source]]
item_id = "disney-bootlegs-jon-tron"
media_type = "video"

[[channel]]
number = 11
call_sign = "WILD"
name = "Game Shows"
genre = "game_shows"
[[channel.source]]
item_id = "003-1986-05-16"
media_type = "video"

[[channel]]
number = 13
call_sign = "DOCS"
name = "Documentaries"
genre = "documentary"
[[channel.source]]
item_id = "youtube-mTtMCoJrGxk"
media_type = "video"
[[channel.source]]
item_id = "youtube-goO-cqm0yho"
media_type = "video"
[[channel.source]]
item_id = "youtube-QVqvv-BbhmU"
media_type = "video"
```

### 1.2 New Crate or Module: `oasis-tv` (or module in `oasis-core`)

**Decision: Add as a module inside `oasis-core/src/apps/tv_guide/`** — keeps it
close to the existing app runner pattern, avoids a new crate for now.

Files to create:
- `mod.rs` — Module root, re-exports
- `channel.rs` — Channel/Source structs, TOML parsing
- `catalog.rs` — IA video catalog (like ArchiveCatalog but for video)
- `schedule.rs` — Deterministic scheduling algorithm
- `guide.rs` — TV guide grid state & rendering logic
- `pip.rs` — PIP state management (which channel, playback position)

### 1.3 Video Catalog Types

Extend the existing `ArchiveCatalog` pattern for video:

```rust
/// A single video episode from an Internet Archive item.
pub struct VideoEpisode {
    pub item_id: String,       // IA item identifier
    pub filename: String,      // e.g. "AOSTH - S01E04 - Submerged Sonic.mp4"
    pub title: String,         // Display title
    pub duration_secs: f64,    // From IA metadata "length" field
    pub width: u32,            // From IA metadata
    pub height: u32,           // From IA metadata
    pub size_bytes: u64,       // File size
}

/// A channel's video library — all episodes available for scheduling.
pub struct ChannelCatalog {
    pub channel_number: u32,
    pub episodes: Vec<VideoEpisode>,
    pub total_duration_secs: f64,  // Sum of all episode durations
}
```

### 1.4 IA Metadata Fetching for Video

Adapt `ArchiveCatalog::parse_files_response()` to also extract video files:

```
GET https://archive.org/metadata/{item_id}/files
```

Response filtering: look for `format` containing `"MPEG4"` or `"h.264"` (prefer
h.264 IA derivatives for streaming — they're optimized). Extract `length`,
`width`, `height`, `size` fields.

**Download URL pattern:**
```
https://archive.org/download/{item_id}/{percent_encoded_filename}
```

**Thumbnail URL pattern** (IA auto-generates these):
```
https://archive.org/services/img/{item_id}
```

**Rate limiting:** Include `User-Agent: OASIS-OS/1.0` header. Add delays between
bulk metadata fetches. Cache results in VFS.

---

## Phase 2: Deterministic Schedule Engine

### 2.1 Core Algorithm

The schedule must be **deterministic** — given the same Unix timestamp and channel
config, every instance of the app computes the same schedule. This means restarting
the app picks up exactly where you left off.

```rust
/// Compute what's playing on a channel at a given Unix timestamp.
pub fn schedule_at(catalog: &ChannelCatalog, unix_time: u64) -> ScheduleSlot {
    // 1. Seed a deterministic PRNG from channel number
    //    to create a shuffled playlist order.
    let seed = channel_seed(catalog.channel_number);
    let playlist = deterministic_shuffle(&catalog.episodes, seed);

    // 2. The playlist repeats infinitely. Compute total cycle duration.
    let cycle_duration = catalog.total_duration_secs as u64;

    // 3. Find position within the current cycle.
    let position_in_cycle = unix_time % cycle_duration;

    // 4. Walk the playlist to find which episode is at this position.
    let mut elapsed = 0u64;
    for episode in &playlist {
        let ep_duration = episode.duration_secs as u64;
        if elapsed + ep_duration > position_in_cycle {
            return ScheduleSlot {
                episode: episode.clone(),
                start_time: unix_time - (position_in_cycle - elapsed),
                elapsed_secs: position_in_cycle - elapsed,
                remaining_secs: ep_duration - (position_in_cycle - elapsed),
            };
        }
        elapsed += ep_duration;
    }
    unreachable!()
}
```

### 2.2 Schedule Grid Generation

For the TV guide display, we need to compute what's playing across a time window
(e.g. the current 2.5-hour block):

```rust
/// Generate schedule for a channel over a time range.
pub fn schedule_range(
    catalog: &ChannelCatalog,
    start_time: u64,
    end_time: u64,
) -> Vec<ScheduleSlot> { ... }
```

### 2.3 Time Slot Alignment

Round the grid's start time to the nearest 30-minute boundary for clean display:
```rust
let slot_start = (unix_time / 1800) * 1800; // Round down to 30-min
```

Display 5 columns of 30-minute slots (2.5 hours visible), scrollable with
Left/Right to see earlier/later times.

---

## Phase 3: TV Guide UI (SDI Rendering)

### 3.1 Layout (matching concept image)

```
┌────────────────────────────────────────────────────────────┐
│  HEADER BAR                                                │
│  ┌──────────────────────────────┐  ┌──────────────────┐   │
│  │ CH 2 RETRO                   │  │   PIP PREVIEW    │   │
│  │ Currently Playing:           │  │   (thumbnail or  │   │
│  │ Sonic S01E04 • 20:25        │  │    live frame)   │   │
│  └──────────────────────────────┘  └──────────────────┘   │
├──────┬─────────┬─────────┬─────────┬─────────┬────────────┤
│ TIME │ 8:00 PM │ 8:30 PM │ 9:00 PM │ 9:30 PM│ 10:00 PM   │
├──────┼─────────┴─────────┼─────────┼────────┼────────────┤
│[CH 2]│ Sonic S01E04      │ S01E05  │S01E10  │ S01E12     │
│RETRO │ (20:25)           │ (20:23) │(20:24) │ (20:24)    │
├──────┼───────────────────┼─────────┴────────┼────────────┤
│[CH 5]│ Bits & Bytes Ep7  │ Episode 8        │ Ep 10      │
│TECH  │ (3:01)            │ (2:35)           │ (2:24)     │
├──────┼───────────────────┴──────────────────┼────────────┤
│[CH 8]│ JonTron: Disney Bootlegs             │ Next...    │
│GAME  │ (15:32)                              │            │
├──────┼───────────────────┬──────────────────┼────────────┤
│[CH11]│ Takeshi's Castle  │ Next Episode     │ ...        │
│WILD  │                   │                  │            │
├──────┼─────────┬─────────┼──────────────────┼────────────┤
│[CH13]│ RDR2    │ Fallout │ Fallout Bombs    │ RDR2...    │
│DOCS  │ (25:00) │ (18:00) │ (12:00)          │            │
└──────┴─────────┴─────────┴──────────────────┴────────────┘
│ [↑↓ SELECT]  [→ VIEW DETAILS]  [PAGE 1/1]      [GUIDE]  │
└──────────────────────────────────────────────────────────────┘
```

### 3.2 SDI Object Naming Convention

```
tv_header_bg          — Header background
tv_header_channel     — "CH 2 RETRO" text
tv_header_playing     — "Currently Playing: ..." text
tv_pip_border         — PIP border/frame
tv_pip_image          — PIP thumbnail texture (or "LIVE" indicator)
tv_grid_bg            — Grid background
tv_time_header_{0..4} — Time slot headers
tv_row_{0..4}_label   — Channel labels (left column)
tv_row_{0..4}_cell_{0..N} — Program cells
tv_row_sel_bg         — Selection highlight (with lerp animation)
tv_footer_bg          — Footer background
tv_footer_text        — Navigation hints
```

### 3.3 Color Scheme (retro CRT aesthetic)

Based on the concept image:
- **Background:** Deep navy blue (#0a1628)
- **Grid lines:** Medium blue (#1a3a5c)
- **Time header:** Bright cyan text on dark blue (#00ccff)
- **Channel labels:** White on dark blue
- **Program cells:** Light text on dark blue (#c0d8e8)
- **Selected row:** Orange/amber highlight (#ff8c00 → #ffa500)
- **Currently playing:** Star icon + brighter text
- **PIP border:** Glowing cyan (#00ddff) with "LIVE" badge in red
- **Header text:** Large white, "Currently Playing" in cyan

These will be defined as theme overrides or hardcoded constants for the TV guide
app specifically (since it's meant to evoke a specific retro aesthetic).

### 3.4 Navigation

- **Up/Down:** Move channel selection (with smooth lerp animation)
- **Left/Right:** Scroll time window (shift by 30-min increments)
- **Confirm (X/Enter):** Tune to selected channel (start playback)
- **Triangle/Tab:** Toggle between guide view and full-screen playback
- **Cancel (O/Esc):** Exit app / return to guide from playback

---

## Phase 4: Video Playback Integration

### 4.1 Strategy: Platform-Specific

Video playback is fundamentally different per backend:

**WASM Backend (Primary Target):**
- Use the existing `IframeOverlay` system
- Construct an IA embed URL or direct MP4 URL in an HTML5 video tag
- The iframe renders on top of the canvas at the PIP region (or full-screen)
- URL format: `https://archive.org/download/{item_id}/{filename}`
- OR use IA's embed player: `https://archive.org/embed/{item_id}`
- For PIP: show iframe at small size in top-right corner
- For full-screen: expand iframe to fill content area
- Seek to correct position: append `#t={elapsed_seconds}` to MP4 URL, or use
  the IA embed player's start parameter

**SDL Desktop Backend (Implemented):**
- In-process streaming decode via `StreamingBuffer` + oasis-video (no ffmpeg required)
- `StreamingBuffer` wraps a shared sliding-window buffer fed by a background HTTPS
  download thread, implementing `Read + Seek` for symphonia's `MediaSourceStream`
- Progressive playback: video starts playing while download continues
- Deferred tail probe (8MB threshold) avoids CDN connection throttling
- Range request CDN failover through archive.org for fresh 302 redirects
- Prebuffer gate (2MB) prevents decoder starvation after seek restart
- PTS-based A/V sync with backpressure throttling
- ffmpeg subprocess fallback still available when `video-decode` feature disabled

**PSP Backend (Implemented):**
- In-memory streaming with AAC hardware decode via `sceAudiocodec`
- Audio-only on PPSSPP emulator (H.264 ME decode requires real hardware)
- TLS 1.3 via embedded-tls for HTTPS CDN nodes
- Backpressure-throttled I/O via audio command queue

### 4.2 WASM Video Playback Implementation

New file: `crates/oasis-backend-wasm/src/video.rs`

```rust
pub struct VideoOverlay {
    iframe: IframeOverlay,  // Reuse existing iframe system
}

impl VideoOverlay {
    /// Show video at PIP size (top-right corner).
    pub fn show_pip(&mut self, item_id: &str, filename: &str, seek_secs: u64);

    /// Show video full-screen (fills content area).
    pub fn show_fullscreen(&mut self, item_id: &str, filename: &str, seek_secs: u64);

    /// Hide video overlay.
    pub fn hide(&mut self);
}
```

### 4.3 Thumbnail Loading

For the PIP preview when not doing live video (SDL backend), or as loading state:

```
https://archive.org/services/img/{item_id}
```

Returns a ~200x200 thumbnail. Fetch via HTTP, decode as JPEG, load as SDI texture.

### 4.4 VFS IPC for Video State

Similar to radio, use VFS paths for communication:

```
/var/tv/status       — Current channel, show, elapsed time
/var/tv/request      — "tune 2", "guide", "fullscreen", "pip"
/var/tv/channels.toml — Channel configuration (copied from /etc/tv/)
/var/tv/cache/{item_id}.json — Cached IA metadata responses
```

---

## Phase 5: Catalog Fetching & Caching

### 5.1 Startup Flow

1. App launches → read `/etc/tv/channels.toml`
2. For each channel, check VFS cache: `/var/tv/cache/{item_id}.json`
3. If cache miss: fetch `https://archive.org/metadata/{item_id}/files`
4. Parse response, extract video episodes (MP4/h.264 files with duration)
5. Store parsed catalog in VFS cache
6. Build `ChannelCatalog` for each channel
7. Compute schedule grid for current time window
8. Render guide

### 5.2 Background Fetching

Catalog fetching should be non-blocking:
- **WASM:** Use `wasm_bindgen_futures::spawn_local()` (existing pattern)
- **SDL:** Use `std::thread::spawn()` with mpsc channel (existing pattern)

Show "Loading..." in cells while catalogs are being fetched.

### 5.3 Rate Limiting

- Fetch metadata for channels sequentially (not all 5 at once)
- Add 500ms delay between requests
- Cache responses for 24 hours (store timestamp in cache JSON)
- Include `User-Agent: OASIS-OS/1.0` in all requests

---

## Phase 6: App Registration & Integration

### 6.1 Register the App

In `crates/oasis-app/src/vfs_setup.rs`, add "TV Guide" to the app list:
```rust
"TV Guide",
```

### 6.2 App Runner Integration

In `crates/oasis-core/src/apps/runner.rs`:
- Add `"TV Guide"` case to `init_content()`
- The TV Guide app will use a **custom rendering path** (not the standard
  line-based content display) — similar to how Browser uses a widget
- Store `TvGuideState` as a field on AppRunner (like `browser: Option<BrowserWidget>`)

### 6.3 Initial VFS Setup

In `vfs_setup.rs`, seed default channel config:
```rust
vfs.mkdir("/etc/tv").ok();
vfs.mkdir("/var/tv").ok();
vfs.mkdir("/var/tv/cache").ok();
vfs.write("/etc/tv/channels.toml", DEFAULT_CHANNELS_TOML.as_bytes()).ok();
```

### 6.4 Terminal Command

Add a `tv` command to the terminal:
```
tv              — Open TV Guide app
tv list         — List channels
tv tune <ch>    — Tune to channel number
tv guide        — Show text-mode schedule grid
tv now          — Show what's playing now on all channels
```

---

## Implementation Order (Incremental Steps)

### Step 1: Data types & TOML config (no UI yet)
- Create `oasis-core/src/apps/tv_guide/mod.rs`, `channel.rs`
- Define `Channel`, `ChannelSource`, `ChannelConfig` structs
- TOML serde for channel configuration
- Populate with 5 channels from the stations list
- Unit tests for TOML round-trip
- **Deliverable:** `cargo test` passes with channel config parsing

### Step 2: Video catalog fetcher
- Create `catalog.rs` — adapt ArchiveCatalog for video files
- `VideoEpisode` struct with duration, dimensions
- Parse IA `/metadata/{item_id}/files` for MP4/h.264 files
- Unit tests with sample JSON fixtures
- **Deliverable:** Can parse IA metadata into video episode lists

### Step 3: Deterministic schedule engine
- Create `schedule.rs`
- `schedule_at()` — what's playing at time T on channel C
- `schedule_range()` — grid data for time window
- Deterministic shuffle (reuse LCG from ArchiveCatalog)
- Extensive unit tests (same input → same output, time continuity)
- **Deliverable:** Pure logic, no I/O, fully tested

### Step 4: Basic text-mode TV Guide app
- Register "TV Guide" app in vfs_setup
- Implement `init_content()` for TV Guide — show text-based schedule
- Use existing line-based rendering (like Internet Radio app)
- Channel selection with Up/Down
- Show schedule as formatted text lines
- **Deliverable:** Working text-mode TV guide in the app

### Step 5: Grid-based SDI rendering
- Create `guide.rs` — custom SDI rendering for the grid layout
- Header with channel info + "Currently Playing"
- Time slot columns
- Channel rows with program cells (variable width based on duration)
- Selection highlight with lerp animation
- Retro blue color scheme
- Navigation: Up/Down channels, Left/Right time scroll
- **Deliverable:** Visual TV guide matching concept image style

### Step 6: Catalog fetching integration (WASM)
- Wire up async metadata fetching in WASM backend
- Cache responses in VFS
- Loading states in UI
- Status publishing via VFS IPC
- **Deliverable:** Live data from archive.org populates the guide

### Step 7: Catalog fetching integration (SDL)
- Wire up threaded metadata fetching in SDL backend
- Shared fetching logic between SDL and WASM where possible
- **Deliverable:** Desktop build also shows live IA data

### Step 8: Video playback (WASM — PIP)
- Create `video.rs` in WASM backend
- Use iframe overlay to show IA video at PIP position
- Seek to correct position based on schedule
- "LIVE" badge overlay
- **Deliverable:** PIP shows actual video in WASM build

### Step 9: Video playback (WASM — full-screen)
- Confirm button expands video to full content area
- Triangle/Tab toggles between guide and full-screen
- Audio continues when returning to guide view
- **Deliverable:** Full video watching experience in WASM

### Step 10: Video playback (SDL — basic)
- On Confirm: open video URL in system browser/mpv
- PIP shows static thumbnail (fetched from IA)
- **Deliverable:** Desktop users can watch content via external player

### Step 11: Terminal integration
- Add `tv` command family to oasis-terminal
- Text-mode schedule display
- Channel tuning via command line
- **Deliverable:** Terminal access to TV guide features

### Step 12: Polish & extras
- CRT scanline effect overlay (optional theme toggle)
- Channel surfing animation (brief static/noise on channel change)
- "Now/Next" indicator on selected channel
- Page indicators for channels (if more than 5)
- Favorites / parental controls (future)

---

## Internet Archive API Reference (for this feature)

| Endpoint | Purpose | Auth |
|----------|---------|------|
| `GET /metadata/{id}` | Full item metadata | None |
| `GET /metadata/{id}/files` | File list with format/size/duration | None |
| `GET /metadata/{id}/files?start=N&count=M` | Paginated file list | None |
| `GET /download/{id}/{filename}` | Direct file download/stream | None |
| `GET /services/img/{id}` | Item thumbnail (JPEG) | None |
| `GET /embed/{id}` | Embeddable player page | None |
| `GET /advancedsearch.php?q=...&output=json` | Search API | None |

All endpoints are public and require no authentication.
User-Agent header required for automated access.

---

## Content Mapping (from stations list)

| Channel | # | Content | IA Item ID | Type |
|---------|---|---------|-----------|------|
| RETRO | 2 | Adventures of Sonic | `adventures-of-sonic-the-hedgehog-01-x-44-the-mystery-of-the-missing-hi-tops_202402` | Video series |
| TECH | 5 | Bits and Bytes | `bits-and-bytes-yt` | Video series |
| GAME | 8 | JonTron Compilation | `disney-bootlegs-jon-tron` | Video |
| WILD | 11 | Takeshi's Castle | `003-1986-05-16` | Video series |
| DOCS | 13 | Any Austin (3 videos) | `youtube-mTtMCoJrGxk`, `youtube-goO-cqm0yho`, `youtube-QVqvv-BbhmU` | Individual eps |

Audio content (This Is Your FBI) is omitted from TV channels since this is a
video-focused feature. It could be added as an audio-only channel later with a
static image/waveform visualization.

---

## Risk Assessment

| Risk | Mitigation |
|------|------------|
| IA rate limiting on metadata fetches | Cache aggressively in VFS (24h TTL) |
| Video files too large for streaming | Use h.264 IA derivatives (smaller); iframe lets browser handle buffering |
| Items removed from IA | Graceful fallback — show "Unavailable" in schedule |
| No video codec in SDL backend | SOLVED: in-process streaming decode via oasis-video + StreamingBuffer |
| PSP HTTPS/TLS failures to CDN | TLS 1.3 fallback via `embedded-tls` + `UnsecureProvider` is implemented and working; both `ia*` (HTTP) and `dn*` (HTTPS-only) CDN nodes now stream successfully |
| Schedule drift if episode durations change | Cache durations; schedule only recomputes when cache refreshes |
| Large catalogs (100+ episodes) | Paginate IA metadata fetch; limit to first 50 episodes per channel |

//! Headless end-to-end harness for the desktop shell.
//!
//! [`Harness`] boots the real [`Shell`] — the same boot sequence, input
//! dispatch (`input::handle_event`), per-frame ticking (app
//! runners, TV / radio / music controllers, window manager, transitions)
//! and renderer the `oasis-app` binary runs — against:
//!
//! - a software framebuffer ([`HeadlessBackend`]: the UE5 rasterizer plus
//!   per-frame `draw_text` recording),
//! - a recording audio output ([`RecordingAudio`]) that captures every PCM
//!   sample and byte fed to it,
//! - a virtual clock (each frame advances exactly 1/60 s) and a frozen
//!   platform clock (`2025-06-15 12:00:00`, like the screenshot CI),
//! - no network: [`AppState::offline`] is set, so TV catalogs, video
//!   downloads and radio streams are never fetched. Scenarios inject data
//!   instead (e.g. TV catalogs, decoded video audio via
//!   [`Harness::inject_video_audio`]).
//!
//! # Writing a scenario
//!
//! ```no_run
//! use oasis_app::harness::Harness;
//! use oasis_app::Mode;
//!
//! let mut h = Harness::new("classic");
//! h.settle(); // let the boot entrance transition finish
//! assert!(h.sdi_text_contains("Calculator")); // dashboard icon label
//! assert!(h.click_app_icon("Calculator"));
//! h.settle();
//! assert_eq!(h.mode(), Mode::Desktop);
//! assert!(h.find_window("Calculator").is_some());
//! assert!(h.close_window("Calculator")); // clicks the titlebar close button
//! h.settle();
//! assert_eq!(h.mode(), Mode::Dashboard);
//! ```
//!
//! Input helpers mirror what the SDL backend emits: a key press is the raw
//! [`InputEvent::Key`] followed by its gamepad twin
//! ([`Key::legacy_press`]), typing adds [`InputEvent::TextInput`], and a
//! click is cursor-move + press on one frame and release on the next.
//! Every helper runs at least one frame, so state is observable right
//! after the call. Frames are rendered exactly when the shell's idle-frame
//! elision says they are needed (as in the binary); use
//! [`Harness::render_now`] to force a present before pixel assertions.
//!
//! See `docs/testing.md` for the full API tour.

use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;

use oasis_core::apps::AppRunner;
use oasis_core::backend::{AudioBackend, AudioTrackId};
use oasis_core::error::OasisError;
use oasis_core::input::{Button, InputEvent, Key, Modifiers, Trigger};
use oasis_core::platform::SystemTime;

use crate::app_state::{AppState, Mode};
use crate::audio_out::ShellAudio;
use crate::headless::{DrawnText, HeadlessBackend};
use crate::shell::{BootObserver, BootOptions, NoSplash, Shell, StepOutcome};

/// One frame of virtual time (60 Hz).
pub const FRAME: Duration = Duration::from_micros(16_667);

/// Platform clock the harness freezes by default (matches screenshot CI).
pub const FIXED_TIME: SystemTime = SystemTime {
    year: 2025,
    month: 6,
    day: 15,
    hour: 12,
    minute: 0,
    second: 0,
};

// ---------------------------------------------------------------------------
// Recording audio fake
// ---------------------------------------------------------------------------

/// One `feed_pcm_f32` call captured by [`RecordingAudio`].
#[derive(Debug, Clone, PartialEq)]
pub struct PcmChunk {
    pub track: AudioTrackId,
    pub channels: u16,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

/// Everything that reached the audio output.
#[derive(Debug, Default)]
pub struct AudioLog {
    /// System volume (0-100) as last set by the shell.
    pub volume: u8,
    /// Decoded f32 PCM fed to streaming tracks, in order.
    pub pcm_chunks: Vec<PcmChunk>,
    /// Encoded bytes fed via `feed_data` (radio / ffmpeg MP3 path).
    pub data_bytes_fed: usize,
    /// Interleaved i16 samples queued on the UI-sound stream.
    pub sfx_samples_queued: usize,
    /// Tracks loaded (`load_track`) or opened (`load_streaming`).
    pub tracks_opened: Vec<AudioTrackId>,
    /// Tracks unloaded.
    pub tracks_unloaded: Vec<AudioTrackId>,
    /// The track currently playing, if any.
    pub playing: Option<AudioTrackId>,
    paused: bool,
    next_id: u64,
    live: HashMap<u64, bool>,
}

impl AudioLog {
    /// All f32 samples fed so far, concatenated.
    pub fn pcm_f32(&self) -> Vec<f32> {
        self.pcm_chunks
            .iter()
            .flat_map(|c| c.samples.iter().copied())
            .collect()
    }
}

/// [`ShellAudio`] fake that records instead of playing. Clone the
/// [`Rc`] from [`RecordingAudio::log`] before boxing it to keep access.
pub struct RecordingAudio {
    log: Rc<RefCell<AudioLog>>,
}

impl RecordingAudio {
    /// A fresh recorder at volume 100.
    pub fn new() -> Self {
        Self {
            log: Rc::new(RefCell::new(AudioLog {
                volume: 100,
                next_id: 1,
                ..AudioLog::default()
            })),
        }
    }

    /// Shared handle to the recorded log.
    pub fn log(&self) -> Rc<RefCell<AudioLog>> {
        Rc::clone(&self.log)
    }

    fn open(&self) -> AudioTrackId {
        let mut log = self.log.borrow_mut();
        let id = AudioTrackId(log.next_id);
        log.next_id += 1;
        log.live.insert(id.0, true);
        log.tracks_opened.push(id);
        id
    }

    fn check(&self, track: AudioTrackId) -> oasis_core::error::Result<()> {
        if self.log.borrow().live.contains_key(&track.0) {
            Ok(())
        } else {
            Err(OasisError::Backend(
                format!("unknown track {}", track.0).into(),
            ))
        }
    }
}

impl Default for RecordingAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for RecordingAudio {
    fn init(&mut self) -> oasis_core::error::Result<()> {
        Ok(())
    }

    fn load_track(&mut self, _data: &[u8]) -> oasis_core::error::Result<AudioTrackId> {
        Ok(self.open())
    }

    fn play(&mut self, track: AudioTrackId) -> oasis_core::error::Result<()> {
        self.check(track)?;
        let mut log = self.log.borrow_mut();
        log.playing = Some(track);
        log.paused = false;
        Ok(())
    }

    fn pause(&mut self) -> oasis_core::error::Result<()> {
        self.log.borrow_mut().paused = true;
        Ok(())
    }

    fn resume(&mut self) -> oasis_core::error::Result<()> {
        self.log.borrow_mut().paused = false;
        Ok(())
    }

    fn stop(&mut self) -> oasis_core::error::Result<()> {
        self.log.borrow_mut().playing = None;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> oasis_core::error::Result<()> {
        self.log.borrow_mut().volume = volume.min(100);
        Ok(())
    }

    fn get_volume(&self) -> u8 {
        self.log.borrow().volume
    }

    fn is_playing(&self) -> bool {
        let log = self.log.borrow();
        log.playing.is_some() && !log.paused
    }

    fn position_ms(&self) -> u64 {
        0
    }

    fn duration_ms(&self) -> u64 {
        0
    }

    fn unload_track(&mut self, track: AudioTrackId) -> oasis_core::error::Result<()> {
        let mut log = self.log.borrow_mut();
        log.live.remove(&track.0);
        log.tracks_unloaded.push(track);
        if log.playing == Some(track) {
            log.playing = None;
        }
        Ok(())
    }

    fn shutdown(&mut self) -> oasis_core::error::Result<()> {
        let mut log = self.log.borrow_mut();
        log.live.clear();
        log.playing = None;
        Ok(())
    }

    fn load_streaming(&mut self) -> oasis_core::error::Result<AudioTrackId> {
        Ok(self.open())
    }

    fn feed_data(&mut self, track: AudioTrackId, data: &[u8]) -> oasis_core::error::Result<()> {
        self.check(track)?;
        self.log.borrow_mut().data_bytes_fed += data.len();
        Ok(())
    }

    fn feed_pcm_f32(
        &mut self,
        track: AudioTrackId,
        samples: &[f32],
        channels: u16,
        sample_rate: u32,
    ) -> oasis_core::error::Result<()> {
        self.check(track)?;
        self.log.borrow_mut().pcm_chunks.push(PcmChunk {
            track,
            channels,
            sample_rate,
            samples: samples.to_vec(),
        });
        Ok(())
    }
}

impl ShellAudio for RecordingAudio {
    fn queue_sfx(&mut self, pcm: &[i16]) -> oasis_core::error::Result<()> {
        self.log.borrow_mut().sfx_samples_queued += pcm.len();
        Ok(())
    }

    fn sfx_queued_bytes(&self) -> u32 {
        // Report an empty queue so the SFX pump keeps mixing (every
        // queued sample is counted in `sfx_samples_queued`).
        0
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Boot configuration for [`Harness::with_options`].
#[derive(Debug, Clone)]
pub struct HarnessOptions {
    /// Skin name (built-in or external), as `resolve_skin` accepts.
    pub skin: String,
    /// Override the resolved screen size.
    pub resolution: Option<(u32, u32)>,
    /// Run the software shader-wallpaper bridge (slow in debug builds).
    pub shader_wallpaper: bool,
    /// Frozen platform clock; `None` uses the real clock.
    pub fixed_time: Option<SystemTime>,
    /// Persist user preferences to this real file, like the binary's
    /// `$OASIS_SETTINGS_FILE`: it is read at boot (the saved skin, when
    /// there is one, wins over `skin`, as it does for the binary without
    /// a skin argument; resolution, volume, font scale, reduced motion and
    /// locale are restored) and written back as settings change. `None`
    /// (the default) disables persistence. Use a per-test temp path.
    ///
    /// Note: the UI locale is process-global, so a persisted non-English
    /// locale is applied for every harness in the test binary.
    pub prefs_path: Option<std::path::PathBuf>,
}

impl HarnessOptions {
    /// Defaults for `skin`: skin resolution, no shader wallpaper, clock
    /// frozen at [`FIXED_TIME`].
    pub fn new(skin: &str) -> Self {
        Self {
            skin: skin.to_string(),
            resolution: None,
            shader_wallpaper: false,
            fixed_time: Some(FIXED_TIME),
            prefs_path: None,
        }
    }
}

/// A window as seen by a scenario (screen coordinates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    /// Outer frame `(x, y, w, h)`.
    pub frame: (i32, i32, u32, u32),
    /// Content area `(x, y, w, h)`.
    pub content: (i32, i32, u32, u32),
    /// Close button `(x, y, w, h)`, if the window has one.
    pub close_button: Option<(i32, i32, u32, u32)>,
    pub minimized: bool,
    pub fullscreen: bool,
}

impl WindowInfo {
    /// Center of the close button.
    pub fn close_point(&self) -> Option<(i32, i32)> {
        self.close_button
            .map(|(x, y, w, h)| (x + w as i32 / 2, y + h as i32 / 2))
    }
}

/// Headless driver for the real shell (see module docs).
pub struct Harness {
    pub shell: Shell<HeadlessBackend>,
    audio: Rc<RefCell<AudioLog>>,
    now: Instant,
    frames: u64,
    rendered: u64,
    last: StepOutcome,
    quit: bool,
}

impl Harness {
    /// Boot `skin` with [`HarnessOptions::new`] defaults. Panics if the
    /// skin cannot be loaded (test helper).
    pub fn new(skin: &str) -> Self {
        Self::with_options(HarnessOptions::new(skin))
            .unwrap_or_else(|e| panic!("harness boot of skin '{skin}' failed: {e:#}"))
    }

    /// Boot with explicit options.
    pub fn with_options(opts: HarnessOptions) -> Result<Self> {
        Self::with_observer(opts, &mut NoSplash)
    }

    /// Boot with explicit options, reporting boot progress (BIOS lines,
    /// status, splash wait points) to `observer` exactly as the desktop
    /// binary reports it to the animated splash.
    pub fn with_observer(
        opts: HarnessOptions,
        observer: &mut dyn BootObserver<HeadlessBackend>,
    ) -> Result<Self> {
        let mut boot = BootOptions::hermetic(&opts.skin)?;
        if let Some(path) = opts.prefs_path.as_ref() {
            let store = crate::user_prefs::load_from_disk(Some(path));
            let prefs = oasis_core::settings::UserPrefs::from_store(&store);
            if let Some(saved) = prefs.skin.as_deref() {
                boot.skin = oasis_core::skin::resolve_skin(saved)?;
            }
            prefs.patch_features(&mut boot.skin.features);
            crate::shell::resolve_screen_size(&mut boot.config, &boot.skin, &prefs);
            if store
                .get_string(oasis_core::settings::pref_keys::LOCALE)
                .is_some()
            {
                oasis_core::i18n::set_ui_locale(prefs.locale());
            }
            boot.prefs = prefs;
            boot.boot_settings = store;
            boot.settings_disk_path = Some(path.clone());
        }
        if let Some((w, h)) = opts.resolution {
            boot.config.screen_width = w;
            boot.config.screen_height = h;
        }
        boot.shader_wallpaper = opts.shader_wallpaper;
        boot.fixed_time = opts.fixed_time;
        let (w, h) = (boot.config.screen_width, boot.config.screen_height);
        let backend = HeadlessBackend::new(w, h);
        let audio = RecordingAudio::new();
        let log = audio.log();
        let shell = Shell::boot(boot, backend, move || Box::new(audio), observer)?;
        let mut harness = Self {
            shell,
            audio: log,
            now: Instant::now(),
            frames: 0,
            rendered: 0,
            last: StepOutcome {
                quit: false,
                redraw: true,
                scene_changed: true,
            },
            quit: false,
        };
        let now = harness.now;
        harness.shell.reset_clocks(now);
        Ok(harness)
    }

    // -- frames -------------------------------------------------------------

    /// Run one frame with `events`: advance the virtual clock by
    /// [`FRAME`], step the shell and render if it asks for a redraw.
    pub fn step(&mut self, events: &[InputEvent]) -> StepOutcome {
        self.now += FRAME;
        self.frames += 1;
        let outcome = self.shell.step(events, self.now);
        if outcome.quit {
            self.quit = true;
        } else if outcome.redraw {
            self.render_frame();
        }
        self.last = outcome;
        outcome
    }

    /// Run `n` frames with no input.
    pub fn run_frames(&mut self, n: u32) {
        for _ in 0..n {
            self.step(&[]);
        }
    }

    /// Run frames covering `dur` of virtual time.
    pub fn advance(&mut self, dur: Duration) {
        let n = dur.as_micros().div_ceil(FRAME.as_micros());
        self.run_frames(u32::try_from(n).unwrap_or(u32::MAX));
    }

    /// Run frames until the UI is at rest: no entrance/launch transition,
    /// no window animation, and the SDI scene unchanged for 3 consecutive
    /// frames (start-menu slides, hover lifts and page fades all mutate
    /// the scene while they run). Perpetual animations that paint outside
    /// the scene graph (shader wallpapers, animated vector layers) don't
    /// hold it up. Caps at 5 s of virtual time. Returns the frames run.
    pub fn settle(&mut self) -> u32 {
        let mut n = 0;
        let mut still = 0;
        while n < 300 {
            let out = self.step(&[]);
            n += 1;
            if out.quit {
                break;
            }
            still = if out.scene_changed { 0 } else { still + 1 };
            let st = &self.shell.state;
            let busy = st.active_transition.is_some() || st.wm.is_animating();
            if !busy && still >= 3 {
                break;
            }
        }
        n
    }

    /// Present a frame now, even if the shell would elide it.
    pub fn render_now(&mut self) {
        self.render_frame();
    }

    fn render_frame(&mut self) {
        if let Err(e) = self.shell.render(self.now) {
            panic!("render failed at frame {}: {e:#}", self.frames);
        }
        self.rendered += 1;
    }

    /// Frames stepped so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Frames actually rendered (the rest were elided as idle).
    pub fn rendered_frames(&self) -> u64 {
        self.rendered
    }

    /// Outcome of the last frame.
    pub fn last_outcome(&self) -> StepOutcome {
        self.last
    }

    /// Whether the shell asked to quit.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// The virtual clock.
    pub fn now(&self) -> Instant {
        self.now
    }

    // -- input --------------------------------------------------------------

    /// Send raw events in one frame.
    pub fn send(&mut self, events: &[InputEvent]) -> StepOutcome {
        self.step(events)
    }

    /// Move the pointer to `(x, y)`.
    pub fn move_to(&mut self, x: i32, y: i32) {
        self.step(&[InputEvent::CursorMove { x, y }]);
    }

    /// Left click at `(x, y)`: move + press this frame, release next.
    pub fn click(&mut self, x: i32, y: i32) {
        self.step(&[
            InputEvent::CursorMove { x, y },
            InputEvent::PointerClick { x, y },
        ]);
        self.step(&[InputEvent::PointerRelease { x, y }]);
    }

    /// Press and drag from `from` to `to` in `steps` moves, then release.
    pub fn drag(&mut self, from: (i32, i32), to: (i32, i32), steps: u32) {
        let (fx, fy) = from;
        self.step(&[
            InputEvent::CursorMove { x: fx, y: fy },
            InputEvent::PointerClick { x: fx, y: fy },
        ]);
        let steps = steps.max(1) as i32;
        for i in 1..=steps {
            let x = fx + (to.0 - fx) * i / steps;
            let y = fy + (to.1 - fy) * i / steps;
            self.step(&[InputEvent::CursorMove { x, y }]);
        }
        self.step(&[InputEvent::PointerRelease { x: to.0, y: to.1 }]);
    }

    /// Scroll the wheel (`delta > 0` scrolls down).
    pub fn scroll(&mut self, delta: i32) {
        self.step(&[InputEvent::MouseWheel { delta }]);
    }

    /// Press and release `key` with no modifiers, as the SDL backend
    /// reports it.
    pub fn key(&mut self, key: Key) {
        self.key_with(key, Modifiers::NONE);
    }

    /// Press and release `key` with `mods`.
    pub fn key_with(&mut self, key: Key, mods: Modifiers) {
        let mut down = vec![InputEvent::Key { key, mods }];
        down.extend(key.legacy_press(mods));
        self.step(&down);
        let up: Vec<InputEvent> = key.legacy_release().into_iter().collect();
        self.step(&up);
    }

    /// Type `text`: per character, the key event (for keys the SDL
    /// keymap has) followed by the `TextInput` event, one frame each.
    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            let mut events = Vec::new();
            let key = match ch {
                ' ' => Some(Key::Space),
                c if c.is_ascii_graphic() => Some(Key::Char(c.to_ascii_lowercase())),
                _ => None,
            };
            let mods = if ch.is_ascii_uppercase() {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
            if let Some(key) = key {
                events.push(InputEvent::Key { key, mods });
                events.extend(key.legacy_press(mods));
            }
            events.push(InputEvent::TextInput(ch));
            self.step(&events);
        }
    }

    /// Press and release a gamepad-style button (PSP face / d-pad).
    pub fn button(&mut self, button: Button) {
        self.step(&[InputEvent::ButtonPress(button)]);
        self.step(&[InputEvent::ButtonRelease(button)]);
    }

    /// Press and release a shoulder trigger.
    pub fn trigger(&mut self, trigger: Trigger) {
        self.step(&[InputEvent::TriggerPress(trigger)]);
        self.step(&[InputEvent::TriggerRelease(trigger)]);
    }

    // -- shell-level actions -------------------------------------------------

    /// Current UI mode.
    pub fn mode(&self) -> Mode {
        self.shell.state.mode
    }

    /// Launch a dashboard app by title (case-insensitive) through the
    /// same path as `OASIS_APP` auto-launch, then run one frame. Returns
    /// `false` if the dashboard has no such app.
    pub fn open_app(&mut self, title: &str) -> bool {
        let shell = &mut self.shell;
        let ok = crate::shell::launch_by_title(&mut shell.state, &mut shell.sdi, &shell.vfs, title);
        self.step(&[]);
        ok
    }

    /// Titles of the apps on the current dashboard page.
    pub fn dashboard_apps(&self) -> Vec<String> {
        self.shell
            .state
            .ui
            .dashboard
            .current_page_apps()
            .iter()
            .map(|a| a.title.clone())
            .collect()
    }

    /// Screen rect `(x, y, w, h)` of the dashboard icon for `title` on
    /// the current page.
    pub fn app_icon_rect(&self, title: &str) -> Option<(i32, i32, u32, u32)> {
        let dash = &self.shell.state.ui.dashboard;
        let idx = dash
            .current_page_apps()
            .iter()
            .position(|a| a.title.eq_ignore_ascii_case(title))?;
        dash.icon_rect(idx)
    }

    /// Click the dashboard icon for `title` (current page only), like a
    /// user would. Returns `false` if the icon is not on this page.
    pub fn click_app_icon(&mut self, title: &str) -> bool {
        let Some((x, y, w, h)) = self.app_icon_rect(title) else {
            return false;
        };
        self.click(x + w as i32 / 2, y + h as i32 / 2);
        true
    }

    /// All open windows, bottom to top.
    pub fn windows(&self) -> Vec<WindowInfo> {
        let wm = &self.shell.state.wm;
        wm.windows()
            .iter()
            .map(|w| WindowInfo {
                id: w.id.as_str().to_string(),
                title: w.title.clone(),
                frame: (w.x, w.y, w.outer_w, w.outer_h),
                content: w.content_rect(wm.theme()),
                close_button: w.close_btn_rect(wm.theme()),
                minimized: w.state == oasis_core::wm::window::WindowState::Minimized,
                fullscreen: w.fullscreen_kiosk,
            })
            .collect()
    }

    /// The open window titled `title` (case-insensitive), else the one
    /// whose id is `title`.
    pub fn find_window(&self, title: &str) -> Option<WindowInfo> {
        // Windows whose title follows their content (the browser shows
        // the page title) are still found by their id.
        let windows = self.windows();
        let by_title = windows
            .iter()
            .position(|w| w.title.eq_ignore_ascii_case(title));
        let idx = by_title.or_else(|| {
            windows
                .iter()
                .position(|w| w.id.eq_ignore_ascii_case(title))
        });
        idx.map(|i| windows[i].clone())
    }

    /// Click the close button of the window titled `title`. Returns
    /// `false` if there is no such window or it has no close button.
    pub fn close_window(&mut self, title: &str) -> bool {
        let Some((x, y)) = self.find_window(title).and_then(|w| w.close_point()) else {
            return false;
        };
        self.click(x, y);
        true
    }

    /// The runner of the open app titled `title` (windowed or
    /// fullscreen).
    pub fn app_runner(&mut self, title: &str) -> Option<&mut AppRunner> {
        let content = &mut self.shell.state.content;
        if let Some(r) = content.app_runner.as_mut()
            && r.title.eq_ignore_ascii_case(title)
        {
            return Some(r);
        }
        content
            .open_runners
            .iter_mut()
            .map(|(_, r)| r)
            .find(|r| r.title.eq_ignore_ascii_case(title))
    }

    /// Open the terminal (Start button), type `command`, press Enter.
    pub fn terminal(&mut self, command: &str) {
        if self.mode() != Mode::Terminal {
            self.button(Button::Start);
        }
        self.type_text(command);
        self.key(Key::Enter);
    }

    /// Queue decoded video audio on the active (injected) TV playback
    /// session, as the software decoder thread would. Returns `false` when
    /// no injected session is running (tune first).
    #[cfg(feature = "_video")]
    pub fn inject_video_audio(
        &mut self,
        pcm_f32: Vec<f32>,
        channels: u16,
        sample_rate: u32,
    ) -> bool {
        self.shell
            .state
            .video_player
            .inject_audio(pcm_f32, channels, sample_rate)
    }

    /// Queue a decoded RGBA video frame on the injected TV session.
    #[cfg(feature = "_video")]
    pub fn inject_video_frame(&mut self, rgba: Vec<u8>, w: u32, h: u32, pts_secs: f64) -> bool {
        self.shell
            .state
            .video_player
            .inject_frame(rgba, w, h, pts_secs)
    }

    /// Shut the shell down the way the binary does on exit (persists any
    /// settings changed since the last periodic sync, stops media).
    pub fn shutdown(self) -> Result<()> {
        self.shell.shutdown()
    }

    // -- observation --------------------------------------------------------

    /// Shell state.
    pub fn state(&self) -> &AppState {
        &self.shell.state
    }

    /// Mutable shell state (for injecting fixtures).
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.shell.state
    }

    /// Framebuffer size.
    pub fn size(&self) -> (u32, u32) {
        self.shell.backend.dimensions()
    }

    /// Copy of the framebuffer (RGBA8, row-major).
    pub fn screenshot(&self) -> Vec<u8> {
        self.shell.backend.pixels().to_vec()
    }

    /// RGBA of pixel `(x, y)`; panics when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let (w, h) = self.size();
        assert!(x < w && y < h, "pixel ({x},{y}) outside {w}x{h}");
        let i = ((y * w + x) * 4) as usize;
        let px = self.shell.backend.pixels();
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    /// Number of distinct colors in the framebuffer (a blank frame has 1).
    pub fn distinct_colors(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        for px in self.shell.backend.pixels().as_chunks::<4>().0 {
            seen.insert([px[0], px[1], px[2]]);
        }
        seen.len()
    }

    /// Strings drawn with `draw_text` in the last presented frame
    /// (includes window content painted outside the SDI scene).
    pub fn frame_text(&self) -> Vec<String> {
        self.shell
            .backend
            .frame_text()
            .iter()
            .map(|t| t.text.clone())
            .collect()
    }

    /// `draw_text` calls of the last presented frame, with positions.
    pub fn frame_text_calls(&self) -> &[DrawnText] {
        self.shell.backend.frame_text()
    }

    /// Whether any string drawn in the last presented frame contains
    /// `needle`.
    pub fn text_drawn_contains(&self, needle: &str) -> bool {
        self.shell
            .backend
            .frame_text()
            .iter()
            .any(|t| t.text.contains(needle))
    }

    /// Text of every visible SDI object.
    pub fn sdi_texts(&self) -> Vec<String> {
        let sdi = &self.shell.sdi;
        sdi.names()
            .filter_map(|name| sdi.get(name).ok())
            .filter(|o| o.visible)
            .filter_map(|o| o.text.clone())
            .collect()
    }

    /// Whether any visible SDI object's text contains `needle`.
    pub fn sdi_text_contains(&self, needle: &str) -> bool {
        self.sdi_texts().iter().any(|t| t.contains(needle))
    }

    /// Rect `(x, y, w, h)` of the visible SDI object `name`.
    pub fn sdi_rect(&self, name: &str) -> Option<(i32, i32, u32, u32)> {
        let o = self.shell.sdi.get(name).ok()?;
        o.visible.then_some((o.x, o.y, o.w, o.h))
    }

    /// Rect of the topmost visible SDI text object whose text is exactly
    /// `text` (falls back to the first one containing it).
    pub fn find_sdi_text(&self, text: &str) -> Option<(i32, i32, u32, u32)> {
        let sdi = &self.shell.sdi;
        let visible: Vec<_> = sdi
            .names()
            .filter_map(|n| sdi.get(n).ok())
            .filter(|o| o.visible && o.text.is_some())
            .collect();
        let pick = |exact: bool| {
            visible
                .iter()
                .filter(|o| {
                    let t = o.text.as_deref().unwrap_or_default();
                    if exact { t == text } else { t.contains(text) }
                })
                .max_by_key(|o| o.z)
                .map(|o| (o.x, o.y, o.w, o.h))
        };
        pick(true).or_else(|| pick(false))
    }

    /// Click the SDI text object labelled `text` (a few pixels inside its
    /// top-left corner, since text objects often have no size). Returns
    /// `false` when no such text is visible.
    pub fn click_sdi_text(&mut self, text: &str) -> bool {
        let Some((x, y, w, h)) = self.find_sdi_text(text) else {
            return false;
        };
        let cx = x + (w as i32 / 2).max(3);
        let cy = y + (h as i32 / 2).max(3);
        self.click(cx, cy);
        true
    }

    /// The shell's virtual file system.
    pub fn vfs(&self) -> &oasis_core::vfs::MemoryVfs {
        &self.shell.vfs
    }

    /// Mutable VFS (e.g. to post a Settings IPC request).
    pub fn vfs_mut(&mut self) -> &mut oasis_core::vfs::MemoryVfs {
        &mut self.shell.vfs
    }

    /// Everything the audio output received.
    pub fn audio(&self) -> Ref<'_, AudioLog> {
        self.audio.borrow()
    }

    /// All decoded f32 PCM fed to the audio output so far.
    pub fn audio_fed(&self) -> Vec<f32> {
        self.audio.borrow().pcm_f32()
    }

    /// Write the framebuffer to a PNG (debugging aid for failing tests).
    pub fn save_png(&self, path: &std::path::Path) -> Result<()> {
        let (w, h) = self.size();
        let file = std::fs::File::create(path)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?
            .write_image_data(self.shell.backend.pixels())?;
        Ok(())
    }
}

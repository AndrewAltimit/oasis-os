//! OASIS_OS desktop entry point.
//!
//! PSIX-style UI with wallpaper, mouse cursor, status bar, 6x3 icon grid
//! dashboard, and bottom bar with media category tabs.
//! L trigger cycles top tabs, R trigger cycles media categories,
//! D-pad navigates the grid. Click to select/launch icons.
//! Press F1 to toggle terminal, F2 to toggle on-screen keyboard, Escape to quit.
//!
//! The shell itself lives in the `oasis_app` library ([`oasis_app::Shell`]);
//! this binary hosts it in an SDL3 window with the SDL audio device and the
//! animated boot splash.

use anyhow::Result;

use oasis_app::boot_splash::{BootSplash, SplashTheme};
use oasis_app::{BootObserver, BootOptions, Shell};
use oasis_backend_sdl::{SdlAudioBackend, SdlBackend};
use oasis_core::backend::{AudioBackend, Color, InputBackend, SdiCore};

/// Target iteration period while frames are being elided. Without a
/// present, vsync no longer paces the loop; `frame_counter` feeds blink
/// and shader clocks that assume ~60 fps, so idle iterations sleep
/// toward the same cadence instead of spinning.
const IDLE_FRAME_PERIOD: std::time::Duration = std::time::Duration::from_micros(16_667);

/// Minimum idle sleep, so a slow iteration can never turn the elided
/// path into a busy spin.
const IDLE_MIN_SLEEP: std::time::Duration = std::time::Duration::from_millis(4);

/// Forwards boot progress to the animated splash (no-op when skipped).
struct SplashObserver {
    splash: Option<BootSplash>,
}

impl BootObserver<SdlBackend> for SplashObserver {
    fn status(&mut self, text: &str) {
        if let Some(sp) = self.splash.as_mut() {
            sp.set_status(text);
        }
    }

    fn bios_line(&mut self, idx: usize, text: String) {
        if let Some(sp) = self.splash.as_mut() {
            sp.set_bios_line(idx, text);
        }
    }

    fn wait_until(&mut self, backend: &mut SdlBackend, secs: f32) {
        // Skipping inside the splash consumes it so later calls are cheap.
        if let Some(sp) = self.splash.as_mut()
            && let Err(e) = sp.run_until(backend, secs)
        {
            log::warn!("Boot splash frame failed: {e}");
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let opts = BootOptions::from_env()?;
    let (w, h) = (opts.config.screen_width, opts.config.screen_height);

    let mut backend = SdlBackend::new(&opts.config.window_title, w, h)?;
    backend.init(w, h)?;

    // Show a black frame immediately so the window isn't frozen during init.
    backend.clear(Color::rgb(0, 0, 0))?;
    backend.swap_buffers()?;

    // Functional boot: the splash animation runs in the foreground while
    // real init work happens between BIOS-line reveals. Each BIOS line
    // reflects a completed probe or registration step. After the last
    // line, the splash phase (3.5–6.5s) warms heavy textures (wallpaper,
    // cursor, shader bridge, SDI layout, audio) so the dashboard's first
    // frame is hitch-free.
    //
    // Skip with OASIS_SKIP_SPLASH=1 for fast development iteration —
    // the same init work still runs, just with no animation.
    let skip_splash = std::env::var("OASIS_SKIP_SPLASH").as_deref() == Ok("1");
    let splash = if skip_splash {
        None
    } else {
        match BootSplash::start_themed(
            &mut backend,
            w,
            h,
            SplashTheme::from_skin_theme(&opts.skin.theme),
        ) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("Boot splash init failed: {e}");
                None
            },
        }
    };
    let mut observer = SplashObserver { splash };

    let mut shell = Shell::boot(
        opts,
        backend,
        || {
            let mut ab = SdlAudioBackend::new();
            ab.init().ok();
            Box::new(ab)
        },
        &mut observer,
    )?;

    // Finish the splash: run the rest of the animation (or skip to end)
    // then fade out and release GPU textures.
    if let Some(mut sp) = observer.splash.take() {
        if let Err(e) = sp.run_to_end(&mut shell.backend) {
            log::warn!("Boot splash tail failed: {e}");
        }
        if let Err(e) = sp.finish(&mut shell.backend) {
            log::warn!("Boot splash finish failed: {e}");
        }
    }

    shell.reset_clocks(std::time::Instant::now());
    loop {
        let iter_start = std::time::Instant::now();
        let events = shell.backend.poll_events();
        let outcome = shell.step(&events, std::time::Instant::now());
        if outcome.quit {
            break;
        }
        if !outcome.redraw {
            // Without a present, vsync no longer paces the loop; sleep
            // toward the normal ~60 Hz cadence (never less than the
            // anti-spin minimum) so frame-counter-driven clocks keep
            // wall-time semantics and input polling stays responsive.
            std::thread::sleep(
                IDLE_FRAME_PERIOD
                    .saturating_sub(iter_start.elapsed())
                    .max(IDLE_MIN_SLEEP),
            );
            continue;
        }
        shell.render(std::time::Instant::now())?;
    }

    shell.shutdown()
}

//! End-to-end test harness for [`App`] implementations (feature `testing`).
//!
//! [`AppHarness`] hosts one app the way the desktop shell does: it routes
//! keyboard input through `handle_key` -> gamepad twin -> `TextInput`
//! (honouring [`App::accepts_text`]), forwards content-local clicks and
//! calls `refresh` afterwards, pumps `tick` / `apply_vfs_ops` /
//! `take_close_request` once per frame, and draws the app into a window
//! with a [`ClipAuditBackend`] that fails the test if anything visible
//! escapes the window's content rectangle.
//!
//! [`fuzz_app`] is the generic property test: thousands of seeded random
//! events at random window sizes and themes, asserting no panic and no
//! clip escape.
//!
//! ```ignore
//! let mut h = AppHarness::new(Box::new(CalculatorApp::new("/apps/calc")));
//! h.click_text("7");
//! h.type_text("+3");
//! h.key(Key::Enter);
//! assert!(h.draw().visible_text().contains("10"));
//! ```

use oasis_skin::ActiveTheme;
use oasis_test_backend::ClipAuditBackend;
use oasis_types::input::{Button, InputEvent, Key, Modifiers};
use oasis_vfs::{MemoryVfs, Vfs};

use crate::{App, AppAction};

/// Window sizes every app is exercised at: tiny, PSP native and XGA.
pub const SIZES: &[(u32, u32)] = &[(96, 64), (480, 272), (1024, 768)];

/// Built-in skins used for theme coverage (dark, light, high-contrast,
/// classic PSIX-style and a heavily stylised one).
pub const THEMES: &[&str] = &["classic", "paper", "highcontrast", "xp", "vaporwave"];

/// Load a built-in skin's theme by name (panics on an unknown name).
#[must_use]
pub fn theme(name: &str) -> ActiveTheme {
    match oasis_skin::builtin::load_builtin(name) {
        Ok(skin) => ActiveTheme::from_skin(&skin.theme),
        Err(e) => panic!("unknown built-in skin {name}: {e}"),
    }
}

/// Deterministic xorshift64* PRNG (no external dependency).
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Seeded generator (a zero seed is remapped).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform value in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    /// Uniform `i32` in `lo..hi` (`hi > lo`).
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo).max(1) as u64) as i32
    }

    /// Pick an element of a non-empty slice.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Hosts one app for an end-to-end test.
pub struct AppHarness {
    app: Box<dyn App>,
    vfs: MemoryVfs,
    theme: ActiveTheme,
    screen: (u32, u32),
    window: (i32, i32, u32, u32),
    closed: bool,
    actions: Vec<AppAction>,
}

impl std::fmt::Debug for AppHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppHarness")
            .field("app", &self.app.title())
            .field("window", &self.window)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl AppHarness {
    /// Host `app` in a 480x272 window at (8, 20) on a 1100x800 screen with
    /// an empty [`MemoryVfs`] and the default theme.
    #[must_use]
    pub fn new(app: Box<dyn App>) -> Self {
        Self::with_vfs(app, MemoryVfs::new())
    }

    /// Like [`Self::new`] with a prepared VFS.
    #[must_use]
    pub fn with_vfs(app: Box<dyn App>, vfs: MemoryVfs) -> Self {
        Self {
            app,
            vfs,
            theme: ActiveTheme::default(),
            screen: (1100, 800),
            window: (8, 20, 480, 272),
            closed: false,
            actions: Vec::new(),
        }
    }

    /// Resize the app's content area (keeps the window origin).
    pub fn set_size(&mut self, w: u32, h: u32) -> &mut Self {
        self.window.2 = w;
        self.window.3 = h;
        self
    }

    /// Current content size.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.window.2, self.window.3)
    }

    /// Switch theme to a built-in skin.
    pub fn set_theme(&mut self, name: &str) -> &mut Self {
        self.theme = theme(name);
        self
    }

    /// The hosted app.
    #[must_use]
    pub fn app(&self) -> &dyn App {
        self.app.as_ref()
    }

    /// The hosted app, mutably.
    pub fn app_mut(&mut self) -> &mut dyn App {
        self.app.as_mut()
    }

    /// Downcast the hosted app.
    #[must_use]
    pub fn app_as<T: 'static>(&self) -> &T {
        match self.app.as_any().downcast_ref::<T>() {
            Some(a) => a,
            None => panic!("app is not a {}", std::any::type_name::<T>()),
        }
    }

    /// Downcast the hosted app mutably.
    pub fn app_as_mut<T: 'static>(&mut self) -> &mut T {
        match self.app.as_any_mut().downcast_mut::<T>() {
            Some(a) => a,
            None => panic!("app is not a {}", std::any::type_name::<T>()),
        }
    }

    /// Replace the hosted app, keeping VFS, size and theme (e.g. to "reopen"
    /// an app after closing it).
    pub fn replace_app(&mut self, app: Box<dyn App>) {
        self.app = app;
        self.closed = false;
    }

    /// The VFS the app sees.
    #[must_use]
    pub fn vfs(&self) -> &MemoryVfs {
        &self.vfs
    }

    /// The VFS, mutably (to seed files mid-test).
    pub fn vfs_mut(&mut self) -> &mut MemoryVfs {
        &mut self.vfs
    }

    /// Whether the app asked to close (Exit action or close request).
    #[must_use]
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// Every non-`None` action the app returned, oldest first.
    #[must_use]
    pub fn actions(&self) -> &[AppAction] {
        &self.actions
    }

    fn note(&mut self, action: AppAction) -> AppAction {
        if action == AppAction::Exit {
            self.closed = true;
        }
        if action != AppAction::None {
            self.actions.push(action.clone());
        }
        action
    }

    /// Press a gamepad button (PSP-style input path).
    pub fn press(&mut self, button: Button) -> AppAction {
        let action = self.app.handle_input(&button, &self.vfs);
        self.note(action)
    }

    /// Press a key with no modifiers (see [`Self::key_mods`]).
    pub fn key(&mut self, key: Key) -> AppAction {
        self.key_mods(key, Modifiers::NONE)
    }

    /// Press a keyboard key exactly as the desktop host routes it:
    /// `handle_key`, then (unless consumed, or suppressed because the app
    /// accepts text and the key types) the gamepad twin, then the
    /// `TextInput` for typing keys. Calls `refresh` after a consumed key.
    pub fn key_mods(&mut self, key: Key, mods: Modifiers) -> AppAction {
        let mut result = AppAction::None;
        let consumed = self.app.handle_key(&key, mods, &self.vfs);
        if let Some(action) = consumed {
            self.app.refresh(&self.vfs);
            result = self.note(action);
        } else if !(self.app.accepts_text() && key.produces_text(mods)) {
            match key.legacy_press(mods) {
                Some(InputEvent::ButtonPress(b)) => {
                    result = self.press(b);
                },
                Some(InputEvent::Backspace) => self.app.handle_backspace(),
                _ => {},
            }
        }
        if self.closed {
            return result;
        }
        if key.produces_text(mods) {
            let ch = match key {
                Key::Space => ' ',
                Key::Char(c) if mods.shift() => c.to_ascii_uppercase(),
                Key::Char(c) => c,
                _ => return result,
            };
            self.app.handle_text_input(ch);
        }
        result
    }

    /// Press `Ctrl+<c>`.
    pub fn ctrl(&mut self, c: char) -> AppAction {
        self.key_mods(Key::Char(c), Modifiers::CTRL)
    }

    /// Type a string. ASCII letters, digits and Space go through the full
    /// key path ([`Self::key_mods`]); everything else arrives as a bare
    /// `TextInput` like an IME / on-screen keyboard would deliver it.
    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            if self.closed {
                return;
            }
            if ch == ' ' {
                self.key(Key::Space);
            } else if ch.is_ascii_alphanumeric() {
                let mods = if ch.is_ascii_uppercase() {
                    Modifiers::SHIFT
                } else {
                    Modifiers::NONE
                };
                self.key_mods(Key::Char(ch.to_ascii_lowercase()), mods);
            } else if ch == '\n' {
                self.key(Key::Enter);
            } else {
                // Symbols: the key event carries the glyph too, then text.
                let consumed = self
                    .app
                    .handle_key(&Key::Char(ch), Modifiers::NONE, &self.vfs);
                if let Some(action) = consumed {
                    self.app.refresh(&self.vfs);
                    self.note(action);
                }
                if !self.closed {
                    self.app.handle_text_input(ch);
                }
            }
        }
    }

    /// Click at content-local coordinates.
    pub fn click(&mut self, lx: i32, ly: i32) -> AppAction {
        let (_, _, w, h) = self.window;
        let action = self.app.handle_click(lx, ly, w, h, false);
        self.app.refresh(&self.vfs);
        self.note(action)
    }

    /// Draw, find the first visible text equal to `label` and click its
    /// centre. Panics (listing what is on screen) if it is not drawn.
    pub fn click_text(&mut self, label: &str) -> AppAction {
        let b = self.draw();
        let Some(t) = b.find_text(label) else {
            panic!(
                "no visible text {label:?} in {}; on screen:\n{}",
                self.app.title(),
                b.visible_text()
            );
        };
        let (x, y) = t.center();
        self.click(x - self.window.0, y - self.window.1)
    }

    /// Like [`Self::click_text`] but clicks the *last* drawn match (e.g. a
    /// keypad button whose label also appears in a display above it).
    pub fn click_text_last(&mut self, label: &str) -> AppAction {
        let b = self.draw();
        let Some(t) = b
            .texts()
            .iter()
            .rev()
            .find(|t| t.visible && t.text == label)
        else {
            panic!(
                "no visible text {label:?} in {}; on screen:\n{}",
                self.app.title(),
                b.visible_text()
            );
        };
        let (x, y) = t.center();
        self.click(x - self.window.0, y - self.window.1)
    }

    /// Like [`Self::click_text`] but matches a substring.
    pub fn click_text_containing(&mut self, needle: &str) -> AppAction {
        let b = self.draw();
        let Some(t) = b.find_text_containing(needle) else {
            panic!(
                "no visible text containing {needle:?} in {}; on screen:\n{}",
                self.app.title(),
                b.visible_text()
            );
        };
        let (x, y) = t.center();
        self.click(x - self.window.0, y - self.window.1)
    }

    /// Run one host frame: `tick(dt_ms)`, `apply_vfs_ops`, and the close
    /// request poll. Returns whether the app reported a visible change.
    pub fn frame(&mut self, dt_ms: u32) -> bool {
        let mut changed = self.app.tick(dt_ms, &self.vfs);
        changed |= self.app.apply_vfs_ops(&mut self.vfs);
        if self.app.take_close_request() {
            self.closed = true;
        }
        changed
    }

    /// Run `n` frames of `dt_ms` each.
    pub fn frames(&mut self, n: u32, dt_ms: u32) {
        for _ in 0..n {
            self.frame(dt_ms);
        }
    }

    /// Draw the app windowed and return the audit backend without
    /// checking for violations.
    #[must_use]
    pub fn draw_unchecked(&self) -> ClipAuditBackend {
        let (x, y, w, h) = self.window;
        let mut b = ClipAuditBackend::new(self.screen.0, self.screen.1, (x, y, w, h));
        b.push_host_clip();
        if let Err(e) = self.app.draw_windowed(x, y, w, h, &mut b, &self.theme) {
            panic!("{} draw_windowed failed at {w}x{h}: {e}", self.app.title());
        }
        b.pop_host_clip();
        b
    }

    /// Draw the app windowed and panic if anything visible escaped the
    /// window's content rectangle.
    #[must_use]
    pub fn draw(&self) -> ClipAuditBackend {
        let b = self.draw_unchecked();
        if !b.violations().is_empty() {
            panic!(
                "{} drew outside its {}x{} window: {:#?}",
                self.app.title(),
                self.window.2,
                self.window.3,
                &b.violations()[..b.violations().len().min(8)]
            );
        }
        b
    }

    /// Visible text of a fresh (checked) draw.
    #[must_use]
    pub fn screen_text(&self) -> String {
        self.draw().visible_text()
    }

    /// Draw at every size in [`SIZES`] under every theme in [`THEMES`],
    /// checking clip containment each time, then restore the size/theme.
    pub fn draw_all_sizes_and_themes(&mut self) {
        let size = self.size();
        let theme = self.theme.clone();
        for &(w, h) in SIZES {
            for name in THEMES {
                self.set_size(w, h).set_theme(name);
                let _ = self.draw();
            }
        }
        self.set_size(size.0, size.1);
        self.theme = theme;
    }
}

/// Keys the fuzzer presses (navigation, editing, shortcuts).
const FUZZ_KEYS: &[Key] = &[
    Key::Up,
    Key::Down,
    Key::Left,
    Key::Right,
    Key::Enter,
    Key::Escape,
    Key::Tab,
    Key::Backspace,
    Key::Delete,
    Key::Home,
    Key::End,
    Key::PageUp,
    Key::PageDown,
    Key::Space,
    Key::Insert,
    Key::F(1),
    Key::F(2),
    Key::F(5),
    Key::Char('a'),
    Key::Char('z'),
    Key::Char('s'),
    Key::Char('n'),
    Key::Char('o'),
    Key::Char('c'),
    Key::Char('v'),
    Key::Char('x'),
    Key::Char('y'),
    Key::Char('f'),
    Key::Char('1'),
    Key::Char('0'),
    Key::Char('q'),
    Key::Char('e'),
];

const FUZZ_MODS: &[Modifiers] = &[
    Modifiers::NONE,
    Modifiers::NONE,
    Modifiers::NONE,
    Modifiers::SHIFT,
    Modifiers::CTRL,
    Modifiers::ALT,
];

const FUZZ_CHARS: &[char] = &[
    'a',
    'Z',
    '0',
    '7',
    '.',
    '+',
    '-',
    '*',
    '/',
    '(',
    ')',
    '=',
    ' ',
    '\u{e9}',
    '\u{4e2d}',
    '\u{1f600}',
    '\t',
    '%',
    '^',
    ',',
    '"',
    '\\',
];

const FUZZ_BUTTONS: &[Button] = &[
    Button::Up,
    Button::Down,
    Button::Left,
    Button::Right,
    Button::Confirm,
    Button::Cancel,
    Button::Triangle,
    Button::Square,
    Button::Start,
    Button::Select,
];

/// Feed `events` seeded random input events to apps built by `make`,
/// asserting no panic and that every draw stays inside the window.
///
/// The window is resized to a random size (including degenerate ones)
/// every few hundred events, the theme cycles through [`THEMES`], and the
/// app is rebuilt (same VFS) whenever it exits. `Escape` / `Cancel` are
/// rarer than other input so sessions get deep before closing.
pub fn fuzz_app(make: &dyn Fn(&dyn Vfs) -> Box<dyn App>, vfs: MemoryVfs, seed: u64, events: u32) {
    let mut rng = Rng::new(seed);
    let first = make(&vfs);
    let mut h = AppHarness::with_vfs(first, vfs);
    let mut theme_idx = 0usize;
    for i in 0..events {
        if i % 250 == 0 {
            let (w, hh) = match rng.below(6) {
                0 => (rng.below(40) as u32, rng.below(40) as u32),
                1 => *rng.pick(SIZES),
                _ => (24 + rng.below(1200) as u32, 24 + rng.below(900) as u32),
            };
            h.set_size(w, hh);
            h.set_theme(THEMES[theme_idx % THEMES.len()]);
            theme_idx += 1;
        }
        if h.closed() {
            let app = make(&h.vfs);
            h.replace_app(app);
        }
        let (w, hh) = h.size();
        match rng.below(100) {
            0..=29 => {
                let x = rng.range(-8, w as i32 + 8);
                let y = rng.range(-8, hh as i32 + 8);
                h.click(x, y);
            },
            30..=54 => {
                let key = *rng.pick(FUZZ_KEYS);
                if matches!(key, Key::Escape) && rng.below(4) != 0 {
                    continue;
                }
                let mods = *rng.pick(FUZZ_MODS);
                h.key_mods(key, mods);
            },
            55..=69 => {
                let ch = *rng.pick(FUZZ_CHARS);
                h.app_mut().handle_text_input(ch);
            },
            70..=84 => {
                let b = *rng.pick(FUZZ_BUTTONS);
                if matches!(b, Button::Cancel) && rng.below(4) != 0 {
                    continue;
                }
                h.press(b);
            },
            85..=89 => h.app_mut().handle_backspace(),
            _ => {
                let dt = *rng.pick(&[0u32, 1, 16, 33, 250, 1000, 5000]);
                h.frame(dt);
            },
        }
        if i % 7 == 0 {
            h.frame(16);
            let _ = h.draw();
        }
    }
    h.frame(16);
    let _ = h.draw();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_bounded() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            let v = a.below(10);
            assert_eq!(v, b.below(10));
            assert!(v < 10);
        }
        let r = a.range(-5, 5);
        assert!((-5..5).contains(&r));
    }

    #[test]
    fn all_fuzz_themes_load() {
        for name in THEMES {
            let _ = theme(name);
        }
    }
}

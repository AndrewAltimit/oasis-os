//! Platform-agnostic input event types.
//!
//! Every backend maps its native input to these enums. The core framework
//! never sees raw platform input.
//!
//! # Two layers of keyboard input
//!
//! Input is modelled in two complementary layers:
//!
//! 1. **Gamepad-style events** ([`InputEvent::ButtonPress`],
//!    [`InputEvent::TriggerPress`], [`InputEvent::Backspace`],
//!    [`InputEvent::Tab`], ...). Every backend emits these; the PSP and
//!    gamepad paths emit *only* these. Desktop keyboards synthesize them
//!    from physical keys via [`Key::legacy_press`] / [`Key::legacy_release`]
//!    (arrows -> d-pad, Enter -> Confirm, Space -> Triangle, Q/E -> L/R
//!    triggers, ...).
//! 2. **Raw keyboard events** ([`InputEvent::Key`]): the physical key plus
//!    the [`Modifiers`] held at the time. Keyboard backends (SDL, WASM, FFI
//!    hosts) emit these for every key-down *in addition to* the
//!    gamepad-style event, so shortcuts such as Ctrl+S, Delete, Home/End or
//!    Alt+Tab can be recognised. Printable characters additionally arrive
//!    as [`InputEvent::TextInput`].
//!
//! # Backend contract for synthesized events
//!
//! When a backend synthesizes a gamepad-style event from a key press it
//! must push the [`InputEvent::Key`] event **immediately before** the
//! synthesized "twin" (which must equal `key.legacy_press(mods)`). Hosts
//! use [`KeyTwinFilter`] to drop that twin when the key was already
//! consumed -- either by an app's `handle_key` shortcut handler, or because
//! a text-entry window has focus and the key merely typed a character
//! (so typing "q" in the text editor no longer switches virtual desktops).

use serde::{Deserialize, Serialize};

/// A platform-agnostic input event.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// Cursor / analog stick moved to absolute position.
    CursorMove { x: i32, y: i32 },
    /// A face / d-pad button pressed.
    ButtonPress(Button),
    /// A face / d-pad button released.
    ButtonRelease(Button),
    /// Shoulder trigger pressed.
    TriggerPress(Trigger),
    /// Shoulder trigger released.
    TriggerRelease(Trigger),
    /// Character typed (on-screen keyboard or physical keyboard).
    TextInput(char),
    /// Backspace / delete-left.
    Backspace,
    /// Pointer click at absolute position (mouse or touch).
    PointerClick { x: i32, y: i32 },
    /// Pointer released.
    PointerRelease { x: i32, y: i32 },
    /// The OS instance gained focus.
    FocusGained,
    /// The OS instance lost focus.
    FocusLost,
    /// Mouse scroll wheel. Positive = scroll down, negative = scroll up.
    MouseWheel { delta: i32 },
    /// Toggle fullscreen kiosk mode for the active window.
    ToggleFullscreen,
    /// Tab key pressed (cycle focus forward).
    Tab,
    /// Shift+Tab key pressed (cycle focus backward).
    ShiftTab,
    /// User requested quit (window close, etc.).
    Quit,
    /// A physical keyboard key was pressed (including auto-repeat).
    ///
    /// Emitted by keyboard backends in addition to the gamepad-style event
    /// the key maps to (see the module docs for the ordering contract).
    /// Never emitted by the PSP / gamepad paths.
    Key {
        /// The key that was pressed.
        key: Key,
        /// Modifier keys held while the key was pressed.
        mods: Modifiers,
    },
}

/// Buttons that map across all platforms.
///
/// On PSP: maps to face buttons and d-pad. On desktop: maps to keyboard keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Button {
    /// D-pad up / arrow key up.
    Up,
    /// D-pad down / arrow key down.
    Down,
    /// D-pad left / arrow key left.
    Left,
    /// D-pad right / arrow key right.
    Right,
    /// Confirm action (PSP Cross / Enter).
    Confirm,
    /// Cancel action (PSP Circle / Escape).
    Cancel,
    /// PSP Triangle / keyboard shortcut.
    Triangle,
    /// PSP Square / keyboard shortcut.
    Square,
    /// Start / menu button.
    Start,
    /// Select / back button.
    Select,
}

/// Shoulder / trigger buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Trigger {
    /// Left shoulder button (L1 / LB).
    Left,
    /// Right shoulder button (R1 / RB).
    Right,
}

/// A physical keyboard key, independent of the backend.
///
/// Letters are always reported lowercase in [`Key::Char`]; check
/// [`Modifiers::shift`] for the shifted form. Text entry should use
/// [`InputEvent::TextInput`] rather than reconstructing characters from
/// keys -- `Key` is meant for shortcuts and navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    /// A printable key (letter, digit or symbol), e.g. `Key::Char('s')`.
    ///
    /// Letters are lowercase. Never holds `' '` (see [`Key::Space`]) or a
    /// control character. Symbol keys report the unshifted glyph where the
    /// backend knows it (SDL keycodes, DOM `Digit*` codes).
    Char(char),
    /// Space bar.
    Space,
    /// Enter / Return (including keypad Enter).
    Enter,
    /// Escape.
    Escape,
    /// Tab.
    Tab,
    /// Backspace (delete left).
    Backspace,
    /// Delete (delete right).
    Delete,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Function key `F1`..=`F12` (the payload is the 1-based number).
    F(u8),
}

impl Key {
    /// Whether pressing this key with `mods` held types a character (and
    /// therefore also produces an [`InputEvent::TextInput`]).
    ///
    /// True for [`Key::Char`] and [`Key::Space`] unless Ctrl, Alt or Super
    /// is held (those combinations are shortcuts, not typing).
    pub fn produces_text(self, mods: Modifiers) -> bool {
        matches!(self, Key::Char(_) | Key::Space) && !mods.has_command()
    }

    /// The gamepad-style event a keyboard backend synthesizes for this key
    /// press, if any.
    ///
    /// This is the canonical desktop keyboard layout shared by the SDL and
    /// WASM backends: arrows -> d-pad, Enter -> Confirm, Escape -> Cancel,
    /// Space -> Triangle, F1 -> Start, F2 -> Select, Q/E -> L/R triggers,
    /// Backspace -> [`InputEvent::Backspace`], Tab / Shift+Tab ->
    /// [`InputEvent::Tab`] / [`InputEvent::ShiftTab`], F11 ->
    /// [`InputEvent::ToggleFullscreen`].
    pub fn legacy_press(self, mods: Modifiers) -> Option<InputEvent> {
        let ev = match self {
            Key::Up => InputEvent::ButtonPress(Button::Up),
            Key::Down => InputEvent::ButtonPress(Button::Down),
            Key::Left => InputEvent::ButtonPress(Button::Left),
            Key::Right => InputEvent::ButtonPress(Button::Right),
            Key::Enter => InputEvent::ButtonPress(Button::Confirm),
            Key::Escape => InputEvent::ButtonPress(Button::Cancel),
            Key::Space => InputEvent::ButtonPress(Button::Triangle),
            Key::F(1) => InputEvent::ButtonPress(Button::Start),
            Key::F(2) => InputEvent::ButtonPress(Button::Select),
            Key::F(11) => InputEvent::ToggleFullscreen,
            Key::Backspace => InputEvent::Backspace,
            Key::Tab if mods.shift() => InputEvent::ShiftTab,
            Key::Tab => InputEvent::Tab,
            Key::Char('q') => InputEvent::TriggerPress(Trigger::Left),
            Key::Char('e') => InputEvent::TriggerPress(Trigger::Right),
            _ => return None,
        };
        Some(ev)
    }

    /// The gamepad-style release event a keyboard backend synthesizes when
    /// this key is released, if any. Mirrors [`Key::legacy_press`] for
    /// buttons and triggers; Backspace, Tab and F11 are press-only.
    pub fn legacy_release(self) -> Option<InputEvent> {
        let ev = match self {
            Key::Up => InputEvent::ButtonRelease(Button::Up),
            Key::Down => InputEvent::ButtonRelease(Button::Down),
            Key::Left => InputEvent::ButtonRelease(Button::Left),
            Key::Right => InputEvent::ButtonRelease(Button::Right),
            Key::Enter => InputEvent::ButtonRelease(Button::Confirm),
            Key::Escape => InputEvent::ButtonRelease(Button::Cancel),
            Key::Space => InputEvent::ButtonRelease(Button::Triangle),
            Key::F(1) => InputEvent::ButtonRelease(Button::Start),
            Key::F(2) => InputEvent::ButtonRelease(Button::Select),
            Key::Char('q') => InputEvent::TriggerRelease(Trigger::Left),
            Key::Char('e') => InputEvent::TriggerRelease(Trigger::Right),
            _ => return None,
        };
        Some(ev)
    }
}

/// Set of modifier keys held during a key press (bitflags-style).
///
/// Left and right variants are merged. Combine with `|`:
/// `Modifiers::CTRL | Modifiers::SHIFT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifiers held.
    pub const NONE: Self = Self(0);
    /// Shift.
    pub const SHIFT: Self = Self(1);
    /// Control.
    pub const CTRL: Self = Self(1 << 1);
    /// Alt / Option.
    pub const ALT: Self = Self(1 << 2);
    /// Super / Windows / Command.
    pub const SUPER: Self = Self(1 << 3);
    const ALL_BITS: u8 = 0b1111;
    const COMMAND_BITS: u8 = Self::CTRL.0 | Self::ALT.0 | Self::SUPER.0;

    /// Build from raw bits (`SHIFT = 1`, `CTRL = 2`, `ALT = 4`,
    /// `SUPER = 8`). Unknown bits are discarded.
    pub const fn from_bits_truncate(bits: u8) -> Self {
        Self(bits & Self::ALL_BITS)
    }

    /// Raw bit representation (see [`Modifiers::from_bits_truncate`]).
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True when no modifier is held.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when *all* modifiers in `other` are held.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// True when *any* modifier in `other` is held.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Shift held.
    pub const fn shift(self) -> bool {
        self.intersects(Self::SHIFT)
    }

    /// Control held.
    pub const fn ctrl(self) -> bool {
        self.intersects(Self::CTRL)
    }

    /// Alt held.
    pub const fn alt(self) -> bool {
        self.intersects(Self::ALT)
    }

    /// Super / Windows / Command held.
    pub const fn super_key(self) -> bool {
        self.intersects(Self::SUPER)
    }

    /// True when Ctrl, Alt or Super is held, i.e. the key press is a
    /// shortcut rather than typing. Shift alone does not count.
    pub const fn has_command(self) -> bool {
        self.0 & Self::COMMAND_BITS != 0
    }

    /// True when exactly the modifiers in `other` are held (no more, no
    /// less). Handy for shortcut matching: `mods.only(Modifiers::CTRL)`
    /// matches Ctrl+S but not Ctrl+Shift+S.
    pub const fn only(self, other: Self) -> bool {
        self.0 == other.0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Host-side filter that drops the gamepad-style "twin" a keyboard backend
/// synthesizes right after an [`InputEvent::Key`] (see the module docs).
///
/// Usage in a host event loop:
///
/// ```
/// use oasis_types::input::{InputEvent, Key, KeyTwinFilter, Modifiers, Trigger};
///
/// let mut filter = KeyTwinFilter::default();
/// let text_focus = true; // e.g. the focused window accepts text
/// let events = [
///     InputEvent::Key { key: Key::Char('q'), mods: Modifiers::NONE },
///     InputEvent::TriggerPress(Trigger::Left),
///     InputEvent::TextInput('q'),
/// ];
/// let mut delivered = Vec::new();
/// for ev in &events {
///     if filter.should_drop(ev) {
///         continue;
///     }
///     if let InputEvent::Key { key, mods } = ev {
///         let consumed_by_app = false; // result of App::handle_key
///         if consumed_by_app || (text_focus && key.produces_text(*mods)) {
///             filter.suppress_twin(*key, *mods);
///         }
///     }
///     delivered.push(ev.clone());
/// }
/// assert_eq!(delivered.len(), 2); // Key + TextInput, no trigger
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyTwinFilter {
    pending: Option<InputEvent>,
}

impl KeyTwinFilter {
    /// Arm the filter so the synthesized twin of `key` (if it has one) is
    /// dropped when it arrives as the very next event.
    pub fn suppress_twin(&mut self, key: Key, mods: Modifiers) {
        self.pending = key.legacy_press(mods);
    }

    /// Returns `true` when `event` is the armed twin and must be dropped.
    ///
    /// Every call disarms the filter, so only the event immediately after
    /// the `Key` can ever be dropped -- gamepad input arriving later is
    /// never affected.
    pub fn should_drop(&mut self, event: &InputEvent) -> bool {
        self.pending.take().is_some_and(|twin| twin == *event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- InputEvent variant construction and equality --

    #[test]
    fn cursor_move_event() {
        let e = InputEvent::CursorMove { x: 100, y: 200 };
        assert_eq!(e, InputEvent::CursorMove { x: 100, y: 200 });
    }

    #[test]
    fn cursor_move_negative_coords() {
        let e = InputEvent::CursorMove { x: -10, y: -20 };
        if let InputEvent::CursorMove { x, y } = e {
            assert_eq!(x, -10);
            assert_eq!(y, -20);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn button_press_all_variants() {
        let buttons = [
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
        for btn in buttons {
            let e = InputEvent::ButtonPress(btn);
            assert_eq!(e, InputEvent::ButtonPress(btn));
        }
    }

    #[test]
    fn button_release_differs_from_press() {
        let press = InputEvent::ButtonPress(Button::Confirm);
        let release = InputEvent::ButtonRelease(Button::Confirm);
        assert_ne!(press, release);
    }

    #[test]
    fn trigger_press_both_variants() {
        let left = InputEvent::TriggerPress(Trigger::Left);
        let right = InputEvent::TriggerPress(Trigger::Right);
        assert_ne!(left, right);
    }

    #[test]
    fn trigger_release_differs_from_press() {
        let press = InputEvent::TriggerPress(Trigger::Left);
        let release = InputEvent::TriggerRelease(Trigger::Left);
        assert_ne!(press, release);
    }

    #[test]
    fn text_input_ascii() {
        let e = InputEvent::TextInput('A');
        assert_eq!(e, InputEvent::TextInput('A'));
    }

    #[test]
    fn text_input_unicode() {
        let e = InputEvent::TextInput('\u{1F600}');
        if let InputEvent::TextInput(ch) = e {
            assert_eq!(ch, '\u{1F600}');
        }
    }

    #[test]
    fn backspace_event() {
        let e = InputEvent::Backspace;
        assert_eq!(e, InputEvent::Backspace);
    }

    #[test]
    fn pointer_click_event() {
        let e = InputEvent::PointerClick { x: 240, y: 136 };
        if let InputEvent::PointerClick { x, y } = e {
            assert_eq!(x, 240);
            assert_eq!(y, 136);
        }
    }

    #[test]
    fn pointer_release_event() {
        let e = InputEvent::PointerRelease { x: 0, y: 0 };
        assert_eq!(e, InputEvent::PointerRelease { x: 0, y: 0 });
    }

    #[test]
    fn focus_and_quit_events() {
        assert_eq!(InputEvent::FocusGained, InputEvent::FocusGained);
        assert_eq!(InputEvent::FocusLost, InputEvent::FocusLost);
        assert_eq!(InputEvent::Quit, InputEvent::Quit);
        assert_ne!(InputEvent::FocusGained, InputEvent::FocusLost);
        assert_ne!(InputEvent::FocusGained, InputEvent::Quit);
    }

    // -- Button properties --

    #[test]
    fn button_clone_and_copy() {
        let b = Button::Confirm;
        let b2 = b;
        let b3 = b.clone();
        assert_eq!(b, b2);
        assert_eq!(b, b3);
    }

    #[test]
    fn button_debug_format() {
        let dbg = format!("{:?}", Button::Triangle);
        assert_eq!(dbg, "Triangle");
    }

    #[test]
    fn button_hash_distinct() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Button::Up);
        set.insert(Button::Down);
        set.insert(Button::Up);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn button_serde_roundtrip() {
        let b = Button::Start;
        let json = serde_json::to_string(&b).unwrap();
        let b2: Button = serde_json::from_str(&json).unwrap();
        assert_eq!(b, b2);
    }

    // -- Trigger properties --

    #[test]
    fn trigger_clone_and_copy() {
        let t = Trigger::Right;
        let t2 = t;
        assert_eq!(t, t2);
    }

    #[test]
    fn trigger_hash_distinct() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Trigger::Left);
        set.insert(Trigger::Right);
        set.insert(Trigger::Left);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn trigger_serde_roundtrip() {
        let t = Trigger::Left;
        let json = serde_json::to_string(&t).unwrap();
        let t2: Trigger = serde_json::from_str(&json).unwrap();
        assert_eq!(t, t2);
    }

    // -- InputEvent clone --

    #[test]
    fn input_event_clone() {
        let e = InputEvent::CursorMove { x: 42, y: 99 };
        let e2 = e.clone();
        assert_eq!(e, e2);
    }

    // -- All variants are distinguishable --

    #[test]
    fn all_event_variants_distinct() {
        let events: Vec<InputEvent> = vec![
            InputEvent::CursorMove { x: 0, y: 0 },
            InputEvent::ButtonPress(Button::Up),
            InputEvent::ButtonRelease(Button::Up),
            InputEvent::TriggerPress(Trigger::Left),
            InputEvent::TriggerRelease(Trigger::Left),
            InputEvent::TextInput('x'),
            InputEvent::Backspace,
            InputEvent::PointerClick { x: 0, y: 0 },
            InputEvent::PointerRelease { x: 0, y: 0 },
            InputEvent::MouseWheel { delta: 1 },
            InputEvent::ToggleFullscreen,
            InputEvent::Tab,
            InputEvent::ShiftTab,
            InputEvent::FocusGained,
            InputEvent::FocusLost,
            InputEvent::Quit,
            InputEvent::Key {
                key: Key::Delete,
                mods: Modifiers::NONE,
            },
        ];
        for (i, a) in events.iter().enumerate() {
            for (j, b) in events.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "variants {i} and {j} should differ");
                }
            }
        }
    }

    // -- Key / Modifiers --

    #[test]
    fn modifiers_bit_ops() {
        let m = Modifiers::CTRL | Modifiers::SHIFT;
        assert!(m.ctrl() && m.shift());
        assert!(!m.alt() && !m.super_key());
        assert!(m.contains(Modifiers::CTRL));
        assert!(!m.contains(Modifiers::CTRL | Modifiers::ALT));
        assert!(m.intersects(Modifiers::CTRL | Modifiers::ALT));
        assert!(m.has_command());
        assert!(!Modifiers::SHIFT.has_command());
        assert!(Modifiers::NONE.is_empty());
        assert!(Modifiers::CTRL.only(Modifiers::CTRL));
        assert!(!m.only(Modifiers::CTRL));
        let mut acc = Modifiers::NONE;
        acc |= Modifiers::SUPER;
        assert!(acc.super_key());
    }

    #[test]
    fn modifiers_bits_roundtrip() {
        let m = Modifiers::ALT | Modifiers::SUPER;
        assert_eq!(Modifiers::from_bits_truncate(m.bits()), m);
        assert_eq!(Modifiers::from_bits_truncate(0xF0), Modifiers::NONE);
    }

    #[test]
    fn produces_text_rules() {
        assert!(Key::Char('a').produces_text(Modifiers::NONE));
        assert!(Key::Char('a').produces_text(Modifiers::SHIFT));
        assert!(Key::Space.produces_text(Modifiers::NONE));
        assert!(!Key::Char('s').produces_text(Modifiers::CTRL));
        assert!(!Key::Char('s').produces_text(Modifiers::ALT));
        assert!(!Key::Delete.produces_text(Modifiers::NONE));
        assert!(!Key::Enter.produces_text(Modifiers::NONE));
    }

    #[test]
    fn legacy_press_map() {
        let none = Modifiers::NONE;
        let press = |k: Key| k.legacy_press(none);
        assert_eq!(press(Key::Up), Some(InputEvent::ButtonPress(Button::Up)));
        assert_eq!(
            press(Key::Enter),
            Some(InputEvent::ButtonPress(Button::Confirm))
        );
        assert_eq!(
            press(Key::Space),
            Some(InputEvent::ButtonPress(Button::Triangle))
        );
        assert_eq!(
            press(Key::Char('q')),
            Some(InputEvent::TriggerPress(Trigger::Left))
        );
        assert_eq!(
            Key::Char('e').legacy_press(Modifiers::SHIFT),
            Some(InputEvent::TriggerPress(Trigger::Right))
        );
        assert_eq!(press(Key::Tab), Some(InputEvent::Tab));
        assert_eq!(
            Key::Tab.legacy_press(Modifiers::SHIFT),
            Some(InputEvent::ShiftTab)
        );
        assert_eq!(press(Key::Backspace), Some(InputEvent::Backspace));
        assert_eq!(press(Key::F(11)), Some(InputEvent::ToggleFullscreen));
        assert_eq!(press(Key::Delete), None);
        assert_eq!(press(Key::Home), None);
        assert_eq!(press(Key::Char('a')), None);
        assert_eq!(press(Key::F(5)), None);
    }

    #[test]
    fn legacy_release_mirrors_press() {
        let keys = [
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Enter,
            Key::Escape,
            Key::Space,
            Key::F(1),
            Key::F(2),
            Key::Char('q'),
            Key::Char('e'),
        ];
        for key in keys {
            match (key.legacy_press(Modifiers::NONE), key.legacy_release()) {
                (Some(InputEvent::ButtonPress(a)), Some(InputEvent::ButtonRelease(b))) => {
                    assert_eq!(a, b)
                },
                (Some(InputEvent::TriggerPress(a)), Some(InputEvent::TriggerRelease(b))) => {
                    assert_eq!(a, b)
                },
                other => panic!("{key:?}: unexpected {other:?}"),
            }
        }
        assert_eq!(Key::Backspace.legacy_release(), None);
        assert_eq!(Key::F(11).legacy_release(), None);
    }

    #[test]
    fn key_serde_roundtrip() {
        for key in [Key::Char('z'), Key::F(12), Key::PageDown] {
            let json = serde_json::to_string(&key).unwrap();
            let back: Key = serde_json::from_str(&json).unwrap();
            assert_eq!(key, back);
        }
    }

    #[test]
    fn twin_filter_drops_only_matching_next_event() {
        let mut f = KeyTwinFilter::default();
        f.suppress_twin(Key::Space, Modifiers::NONE);
        assert!(f.should_drop(&InputEvent::ButtonPress(Button::Triangle)));
        // Disarmed after one event.
        assert!(!f.should_drop(&InputEvent::ButtonPress(Button::Triangle)));

        // A non-matching event disarms without being dropped.
        f.suppress_twin(Key::Char('q'), Modifiers::NONE);
        assert!(!f.should_drop(&InputEvent::TextInput('q')));
        assert!(!f.should_drop(&InputEvent::TriggerPress(Trigger::Left)));

        // Keys without a twin never drop anything.
        f.suppress_twin(Key::Char('a'), Modifiers::NONE);
        assert!(!f.should_drop(&InputEvent::ButtonPress(Button::Up)));
    }
}

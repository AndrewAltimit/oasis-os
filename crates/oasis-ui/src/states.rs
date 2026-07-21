//! Uniform interaction-state color resolution for interactive widgets.
//!
//! Every interactive widget in the toolkit paints itself differently
//! depending on whether the pointer is hovering, the control is being
//! pressed, or it is disabled. Historically each widget hand-picked the
//! relevant [`Theme`] fields inline, which made the
//! coverage inconsistent: some honored every state, others ignored them
//! and painted a fixed color.
//!
//! [`WidgetStateColors`] centralizes that mapping so state coloring is
//! *uniform* and *themeable*. When a skin overrides the per-state theme
//! fields (via `[widget_states.*]`, which flow into the `ui::Theme`
//! `button_bg_hover` / `accent_pressed` / … fields during derivation)
//! every widget routed through this helper picks the new colors up for
//! free.
//!
//! # Pixel-identity guarantee
//!
//! With the default (un-overridden) theme the resolver reproduces each
//! widget's previous appearance byte-for-byte: the mapping is a pure
//! rename of the exact `Theme` fields the widgets already referenced.
//! This is asserted by the tests at the bottom of the module.

use crate::theme::Theme;
use oasis_types::backend::Color;

/// Interaction state used to resolve themed widget colors.
///
/// The ordering used by [`WidgetState::from_flags`] is
/// `Disabled` > `Pressed` > `Hover` > `Normal`: a disabled control never
/// shows a hover/press tint, and an actively-pressed control wins over a
/// mere hover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidgetState {
    /// Idle / resting state.
    #[default]
    Normal,
    /// Pointer is over the control.
    Hover,
    /// Control is being actively pressed / activated.
    Pressed,
    /// Control is disabled and non-interactive.
    Disabled,
}

impl WidgetState {
    /// Collapse a set of interaction booleans into a single state,
    /// applying the priority `Disabled` > `Pressed` > `Hover` > `Normal`.
    pub fn from_flags(hover: bool, pressed: bool, disabled: bool) -> Self {
        if disabled {
            WidgetState::Disabled
        } else if pressed {
            WidgetState::Pressed
        } else if hover {
            WidgetState::Hover
        } else {
            WidgetState::Normal
        }
    }

    /// Whether this state is [`WidgetState::Disabled`].
    pub fn is_disabled(self) -> bool {
        matches!(self, WidgetState::Disabled)
    }
}

/// Resolved neutral-surface colors for a widget in a given state.
///
/// Produced by [`WidgetStateColors::resolve`]. The three fields cover the
/// common trio an interactive control needs: a background fill, a
/// foreground text/content color, and a border color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetStateColors {
    /// Neutral (button-like) background fill.
    pub background: Color,
    /// Foreground text / content color.
    pub text: Color,
    /// Border / outline color.
    pub border: Color,
}

impl WidgetStateColors {
    /// Resolve the neutral-surface color trio for `state`.
    ///
    /// Equivalent to reading [`neutral_bg`](Self::neutral_bg),
    /// [`content_text`](Self::content_text) and [`border`](Self::border)
    /// individually.
    pub fn resolve(theme: &Theme, state: WidgetState) -> Self {
        Self {
            background: Self::neutral_bg(theme, state),
            text: Self::content_text(theme, state),
            border: Self::border(theme, state),
        }
    }

    /// Neutral (button) background for the given state.
    ///
    /// Maps to `button_bg` / `button_bg_hover` / `button_bg_pressed` /
    /// `button_bg_disabled`.
    pub fn neutral_bg(theme: &Theme, state: WidgetState) -> Color {
        match state {
            WidgetState::Normal => theme.button_bg,
            WidgetState::Hover => theme.button_bg_hover,
            WidgetState::Pressed => theme.button_bg_pressed,
            WidgetState::Disabled => theme.button_bg_disabled,
        }
    }

    /// Accent-filled background for the given state.
    ///
    /// Maps to `accent` / `accent_hover` / `accent_pressed`, falling back
    /// to `button_bg_disabled` when disabled (a disabled accent control
    /// reads as a greyed-out neutral surface, matching the original
    /// primary-button behavior).
    pub fn accent_bg(theme: &Theme, state: WidgetState) -> Color {
        match state {
            WidgetState::Normal => theme.accent,
            WidgetState::Hover => theme.accent_hover,
            WidgetState::Pressed => theme.accent_pressed,
            WidgetState::Disabled => theme.button_bg_disabled,
        }
    }

    /// Foreground text/content color for the given state.
    ///
    /// Delegates to [`Theme::interactive_text`] so the disabled-text
    /// mapping lives in exactly one place.
    pub fn content_text(theme: &Theme, state: WidgetState) -> Color {
        theme.interactive_text(state.is_disabled())
    }

    /// Border color for the given state.
    ///
    /// Delegates to [`Theme::interactive_border`] with `selected = false`.
    pub fn border(theme: &Theme, state: WidgetState) -> Color {
        theme.interactive_border(state.is_disabled(), false)
    }

    /// Focus-ring color for interactive widgets.
    ///
    /// Uses `input_border_focus` (the standard focus-indicator color),
    /// which skins may override independently of the resting accent.
    pub fn focus_ring(theme: &Theme) -> Color {
        theme.input_border_focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_themes() -> Vec<Theme> {
        vec![
            Theme::dark(),
            Theme::light(),
            Theme::classic(),
            Theme::high_contrast(),
            Theme::colorblind(),
            Theme::protanopia(),
            Theme::tritanopia(),
        ]
    }

    #[test]
    fn from_flags_priority() {
        assert_eq!(
            WidgetState::from_flags(false, false, false),
            WidgetState::Normal
        );
        assert_eq!(
            WidgetState::from_flags(true, false, false),
            WidgetState::Hover
        );
        assert_eq!(
            WidgetState::from_flags(false, true, false),
            WidgetState::Pressed
        );
        // Pressed beats hover.
        assert_eq!(
            WidgetState::from_flags(true, true, false),
            WidgetState::Pressed
        );
        // Disabled beats everything.
        assert_eq!(
            WidgetState::from_flags(true, true, true),
            WidgetState::Disabled
        );
    }

    #[test]
    fn default_is_normal() {
        assert_eq!(WidgetState::default(), WidgetState::Normal);
    }

    #[test]
    fn is_disabled_flag() {
        assert!(WidgetState::Disabled.is_disabled());
        assert!(!WidgetState::Hover.is_disabled());
    }

    // -- Pixel-identity: neutral_bg reproduces the exact button fields --

    #[test]
    fn neutral_bg_matches_theme_fields_all_themes() {
        for t in all_themes() {
            assert_eq!(
                WidgetStateColors::neutral_bg(&t, WidgetState::Normal),
                t.button_bg
            );
            assert_eq!(
                WidgetStateColors::neutral_bg(&t, WidgetState::Hover),
                t.button_bg_hover
            );
            assert_eq!(
                WidgetStateColors::neutral_bg(&t, WidgetState::Pressed),
                t.button_bg_pressed
            );
            assert_eq!(
                WidgetStateColors::neutral_bg(&t, WidgetState::Disabled),
                t.button_bg_disabled
            );
        }
    }

    #[test]
    fn accent_bg_matches_theme_fields_all_themes() {
        for t in all_themes() {
            assert_eq!(
                WidgetStateColors::accent_bg(&t, WidgetState::Normal),
                t.accent
            );
            assert_eq!(
                WidgetStateColors::accent_bg(&t, WidgetState::Hover),
                t.accent_hover
            );
            assert_eq!(
                WidgetStateColors::accent_bg(&t, WidgetState::Pressed),
                t.accent_pressed
            );
            // Disabled accent falls back to the neutral disabled bg.
            assert_eq!(
                WidgetStateColors::accent_bg(&t, WidgetState::Disabled),
                t.button_bg_disabled
            );
        }
    }

    #[test]
    fn content_text_matches_interactive_text() {
        for t in all_themes() {
            assert_eq!(
                WidgetStateColors::content_text(&t, WidgetState::Normal),
                t.interactive_text(false)
            );
            assert_eq!(
                WidgetStateColors::content_text(&t, WidgetState::Disabled),
                t.interactive_text(true)
            );
            assert_eq!(
                WidgetStateColors::content_text(&t, WidgetState::Normal),
                t.text_primary
            );
            assert_eq!(
                WidgetStateColors::content_text(&t, WidgetState::Disabled),
                t.text_disabled
            );
        }
    }

    #[test]
    fn border_matches_interactive_border() {
        for t in all_themes() {
            assert_eq!(
                WidgetStateColors::border(&t, WidgetState::Normal),
                t.input_border
            );
            assert_eq!(
                WidgetStateColors::border(&t, WidgetState::Disabled),
                t.border_subtle
            );
        }
    }

    #[test]
    fn focus_ring_is_input_border_focus() {
        for t in all_themes() {
            assert_eq!(WidgetStateColors::focus_ring(&t), t.input_border_focus);
        }
    }

    #[test]
    fn resolve_bundles_the_trio() {
        let t = Theme::dark();
        let c = WidgetStateColors::resolve(&t, WidgetState::Hover);
        assert_eq!(c.background, t.button_bg_hover);
        assert_eq!(c.text, t.text_primary);
        assert_eq!(c.border, t.input_border);
    }
}

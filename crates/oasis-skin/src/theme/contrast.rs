//! Contrast math and the WCAG report behind `skin lint`'s contrast checks.

use oasis_types::backend::Color;

use super::SkinTheme;

/// Compute the WCAG 2.x contrast ratio between two colors.
///
/// Returns a value between 1.0 (identical) and 21.0 (black vs white).
/// Alpha is ignored; composite translucent colors over their backdrop
/// with [`composite_over`] first.
pub fn contrast_ratio(a: Color, b: Color) -> f64 {
    oasis_types::color::contrast_ratio(a, b)
}

/// Alpha-composite `fg` over an opaque `bg` (source-over).
///
/// Used to resolve translucent theme colors (bar backgrounds, selection
/// highlights) to the effective on-screen color before measuring
/// contrast. The result is opaque.
pub fn composite_over(fg: Color, bg: Color) -> Color {
    if fg.a == 255 {
        return Color::rgb(fg.r, fg.g, fg.b);
    }
    let a = fg.a as u32;
    let inv = 255 - a;
    let blend = |f: u8, b: u8| ((f as u32 * a + b as u32 * inv + 127) / 255) as u8;
    Color::rgb(blend(fg.r, bg.r), blend(fg.g, bg.g), blend(fg.b, bg.b))
}

/// Make a *derived* text color readable on an opaque backdrop.
///
/// Returns `fg` unchanged when it already reaches `min` contrast on `bg`
/// (so every skin whose derived colors were fine stays pixel-identical).
/// Otherwise `fg` is mixed toward whichever of black / white contrasts more
/// with `bg`, in 5% steps, and the first opaque mix reaching `min` is
/// returned -- the hue is kept as far as the contrast allows. Explicit skin
/// colors are never passed through this: authors keep what they wrote, and
/// `skin lint` reports it instead.
pub(crate) fn readable_on(fg: Color, bg: Color, min: f64) -> Color {
    let bg = Color::rgb(bg.r, bg.g, bg.b);
    let opaque = composite_over(fg, bg);
    if contrast_ratio(opaque, bg) >= min {
        return fg;
    }
    let target = if contrast_ratio(Color::BLACK, bg) >= contrast_ratio(Color::WHITE, bg) {
        Color::BLACK
    } else {
        Color::WHITE
    };
    (1..=20)
        .map(|step| opaque.lerp(target, step as f32 / 20.0))
        .map(|c| Color::rgb(c.r, c.g, c.b))
        .find(|&c| contrast_ratio(c, bg) >= min)
        .unwrap_or(target)
}

/// Effective opaque backdrop of a bar: its (possibly translucent) flat fill
/// over the shell background, or -- for a vertical gradient -- the gradient
/// midpoint, where the text baseline sits.
pub(crate) fn bar_backdrop(
    flat: Color,
    top: Option<Color>,
    bottom: Option<Color>,
    background: Color,
) -> Color {
    let flat = composite_over(flat, background);
    match (top, bottom) {
        (Some(t), Some(b)) => composite_over(t, flat).lerp(composite_over(b, flat), 0.5),
        _ => flat,
    }
}

/// A contrast warning for a text/background color pair.
#[derive(Debug, Clone)]
pub struct ContrastWarning {
    /// Human-readable label for the pair (e.g. "text on background").
    pub pair: String,
    /// The computed contrast ratio.
    pub ratio: f64,
    /// WCAG AA minimum (4.5 for body text, 3.0 for large/secondary text).
    pub required: f64,
}

impl SkinTheme {
    /// Validate key text/background pairs against WCAG AA contrast
    /// minimums (4.5:1 for body text, 3:1 for large/secondary text).
    ///
    /// Checks the base palette plus the derived surfaces that actually
    /// render text: buttons (`ui::Theme`), the selection highlight and
    /// status/bottom bars (`ActiveTheme`). Translucent colors are
    /// composited over their backdrop first.
    ///
    /// These are advisory: stylized skins may fail intentionally, so
    /// callers surface them as warnings, never errors.
    ///
    /// Assumes the clock sits in the top status bar; use
    /// [`validate_contrast_placed`](Self::validate_contrast_placed) when
    /// the skin's `clock_in_bottombar` feature is known.
    pub fn validate_contrast(&self) -> Vec<ContrastWarning> {
        self.validate_contrast_placed(false)
    }

    /// [`validate_contrast`](Self::validate_contrast), checking the clock
    /// against the bar it is actually drawn on (`clock_in_bottombar`).
    pub fn validate_contrast_placed(&self, clock_in_bottombar: bool) -> Vec<ContrastWarning> {
        self.contrast_report(clock_in_bottombar)
            .into_iter()
            .filter(|c| c.ratio < c.required)
            .collect()
    }

    /// Contrast ratio of every checked text/background pair, whether or
    /// not it meets its WCAG minimum (`required`).
    ///
    /// `clock_in_bottombar` selects which bar backdrop the clock is
    /// measured against; the other bar is then checked with its remaining
    /// text (battery label for the status bar).
    pub fn contrast_report(&self, clock_in_bottombar: bool) -> Vec<ContrastWarning> {
        let bg = self.background_color();
        let ui = self.to_ui_theme();
        let at = crate::active_theme::ActiveTheme::from_skin(self);

        // Effective (opaque) backdrops for surfaces with alpha.
        let statusbar_bg = bar_backdrop(
            at.bar.statusbar_bg,
            at.bar.statusbar_gradient_top,
            at.bar.statusbar_gradient_bottom,
            bg,
        );
        let bottombar_bg = bar_backdrop(at.bar.bg, at.bar.gradient_top, at.bar.gradient_bottom, bg);
        let app_bg = composite_over(at.app.bg, bg);
        let selection_bg = composite_over(at.app.selected_bg, app_bg);
        // Windowed apps draw their body text straight onto the window
        // manager's content fill (`draw_content_windowed` paints no bg).
        let window_bg = composite_over(self.build_wm_theme().content_bg_color, bg);

        let (clock_label, clock_bg) = if clock_in_bottombar {
            ("clock on bottom bar", bottombar_bg)
        } else {
            ("status bar text on status bar", statusbar_bg)
        };
        let (status_label, status_fg) = if clock_in_bottombar {
            ("status bar text on status bar", at.bar.battery_color)
        } else {
            ("battery text on status bar", at.bar.battery_color)
        };

        let mut pairs: Vec<(&str, Color, Color, f64)> = vec![
            // Base palette on the shell background.
            ("text on background", self.text_color(), bg, 4.5),
            ("dim_text on background", self.dim_text_color(), bg, 3.0),
            ("error on background", self.error_color(), bg, 3.0),
            // The terminal draws on the app surface, not the wallpaper
            // background (`TerminalColors::from_theme`).
            (
                "terminal output on app background",
                Color::rgb(
                    at.app.terminal_output_color.r,
                    at.app.terminal_output_color.g,
                    at.app.terminal_output_color.b,
                ),
                app_bg,
                4.5,
            ),
            (
                "terminal prompt on app background",
                at.app.terminal_prompt_color,
                app_bg,
                3.0,
            ),
            ("app text on window content", at.app.text, window_bg, 4.5),
            // Derived widget surfaces.
            (
                "button text on button background",
                ui.button_text,
                ui.button_bg,
                4.5,
            ),
            // Selection rows carry a non-color cue (the highlight fill
            // and accent bar), so hold them to the large-text minimum.
            (
                "selected text on selection highlight",
                at.app.selected_text,
                selection_bg,
                3.0,
            ),
            (clock_label, at.bar.clock_color, clock_bg, 3.0),
            (status_label, status_fg, statusbar_bg, 3.0),
            (
                "bottom bar text on bottom bar",
                at.bar.url_color,
                bottombar_bg,
                3.0,
            ),
        ];
        // A `url_text` label sits on the bottom bar's chrome bezel plate
        // (`oasis_sdi::BezelStyle::chrome`), not on the bare bar.
        if !at.bar.url_text.is_empty() {
            let bezel = composite_over(oasis_sdi::helpers::BezelStyle::chrome().fill, bottombar_bg);
            pairs.push(("bar url text on its bezel", at.bar.url_color, bezel, 3.0));
        }

        pairs
            .iter()
            .map(|&(label, fg, backdrop, required)| ContrastWarning {
                pair: label.to_string(),
                ratio: contrast_ratio(composite_over(fg, backdrop), backdrop),
                required,
            })
            .collect()
    }
}

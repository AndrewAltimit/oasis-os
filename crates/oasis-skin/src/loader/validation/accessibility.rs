//! Accessibility lints: colorblind-unsafe semantic color pairs.

use oasis_types::backend::Color;

use crate::theme::contrast_ratio;

use super::super::Skin;

/// Signed position of a color on the red-vs-green axis.
///
/// Positive => red-dominant, negative => green-dominant, near-zero => neutral.
fn red_green_axis(c: Color) -> i32 {
    i32::from(c.r) - i32::from(c.g)
}

/// Whether two colors are distinguished *primarily* by red-vs-green hue: one is
/// clearly red-dominant and the other clearly green-dominant. Colors on opposite
/// ends of the red/green axis are exactly what deuteranopia/protanopia collapse.
fn red_green_opposed(a: Color, b: Color) -> bool {
    const MIN_AXIS: i32 = 40;
    // A color only sits on the red/green confusion axis when blue is not its
    // dominant channel. Without this guard a blue such as #4488CC (where the
    // green channel exceeds red purely by channel math) is misread as
    // "green-dominant" and falsely paired with a red -- deuteranopia does not
    // collapse blue against red.
    let on_axis = |c: Color| {
        red_green_axis(c).abs() >= MIN_AXIS && i32::from(c.b) <= i32::from(c.r).max(i32::from(c.g))
    };
    on_axis(a) && on_axis(b) && (red_green_axis(a).signum() != red_green_axis(b).signum())
}

impl Skin {
    /// Flag semantic color pairs that a user must tell apart but which are
    /// distinguished *only* by red-vs-green hue with little luminance
    /// separation -- the classic deuteranopia/protanopia failure mode. Warn
    /// only when the two colors are both red/green-opposed AND close in
    /// luminance (inter-color contrast below 1.5:1), so a normal high-contrast
    /// green/red pair (e.g. a bright prompt vs a dim error) does not warn.
    ///
    /// Advisory only: never blocks loading.
    pub(super) fn check_colorblind_pairs(&self, warnings: &mut Vec<String>) {
        let error = self.theme.error_color();
        // (label_a, color_a, label_b, color_b) -- pairs that carry meaning.
        let pairs: &[(&str, Color, &str, Color)] = &[
            ("prompt", self.theme.prompt_color(), "error", error),
            ("output", self.theme.output_color(), "error", error),
            ("primary", self.theme.primary_color(), "error", error),
        ];

        const MIN_LUMA_RATIO: f64 = 1.5;
        for &(la, ca, lb, cb) in pairs {
            if red_green_opposed(ca, cb) && contrast_ratio(ca, cb) < MIN_LUMA_RATIO {
                warnings.push(format!(
                    "accessibility: '{la}' and '{lb}' differ mainly by red/green hue \
                     with low luminance separation ({:.2}:1); risky for \
                     deuteranopia/protanopia",
                    contrast_ratio(ca, cb)
                ));
            }
        }
    }
}

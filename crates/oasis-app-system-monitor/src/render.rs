//! Windowed rendering for the System Monitor: an info card (platform,
//! backend, VFS) followed by one themed gauge row per metric.
//!
//! Colors come from the active theme via [`SysmonColors`]; bars are
//! oasis-ui [`ProgressBar`]s, and unavailable metrics draw an empty track
//! with an explicit "N/A" label.

use oasis_skin::ActiveTheme;
use oasis_types::backend::{Color, SdiBackend};
use oasis_ui::progress_bar::ProgressBar;
use oasis_ui::{DrawContext, Widget};

use crate::SystemMonitorApp;
use crate::status::Level;

/// Outer padding.
const PAD: i32 = 6;
/// Info row height.
const INFO_ROW_H: i32 = 13;
/// Gauge label / value line height.
const GAUGE_TEXT_H: i32 = 13;
/// Gauge bar height.
const BAR_H: u32 = 8;
/// Vertical space between gauge rows.
const GAUGE_GAP: i32 = 7;
/// Width reserved for info-row labels.
const INFO_LABEL_W: i32 = 60;

/// System Monitor color roles, derived from the active theme. Skins can
/// override any slot via `[app_themes.system_monitor]` in theme.toml.
#[derive(Debug, Clone)]
pub struct SysmonColors {
    /// Content background.
    pub bg: Color,
    /// Info card background / border.
    pub card_bg: Color,
    pub card_border: Color,
    /// Labels (info-row keys, gauge names).
    pub label: Color,
    /// Values.
    pub text: Color,
    /// Secondary text (N/A values, footer).
    pub dim_text: Color,
    /// Empty track of an unavailable gauge.
    pub na_track: Color,
    /// Value text for warning / critical readings.
    pub warning: Color,
    pub critical: Color,
}

impl SysmonColors {
    /// Build colors from the active theme, honouring per-app overrides.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| at.app_color("system_monitor", key).unwrap_or(default);
        let ui = &at.ui_theme;
        Self {
            bg: c("bg", at.app.bg),
            card_bg: c("card_bg", ui.surface),
            card_border: c("card_border", ui.border_subtle),
            label: c("label", ui.text_secondary),
            text: c("text", ui.text_primary),
            dim_text: c("dim_text", at.app.dim_text),
            na_track: c("na_track", ui.border_subtle),
            warning: c("warning", ui.warning),
            critical: c("critical", ui.error),
        }
    }

    fn value_color(&self, level: Level) -> Color {
        match level {
            Level::Normal => self.text,
            Level::Warning => self.warning,
            Level::Critical => self.critical,
            Level::Unavailable => self.dim_text,
        }
    }
}

impl SystemMonitorApp {
    /// Draw the whole System Monitor UI into the content rect.
    pub(crate) fn draw_monitor(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let colors = SysmonColors::from_theme(at);
        let font = at.font_hint;
        backend.fill_rect(cx, cy, cw, ch, colors.bg)?;
        let inner_w = cw.saturating_sub(2 * PAD as u32);
        let x = cx + PAD;
        let mut y = cy + PAD;

        // Info card.
        let rows = self.info_rows();
        let card_h = (rows.len() as i32 * INFO_ROW_H + 6) as u32;
        let radius = at.ui_theme.border_radius_md;
        backend.fill_rounded_rect(x, y, inner_w, card_h, radius, colors.card_bg)?;
        backend.stroke_rounded_rect(x, y, inner_w, card_h, radius, 1, colors.card_border)?;
        for (i, (label, value)) in rows.iter().enumerate() {
            let ry = y + 3 + i as i32 * INFO_ROW_H;
            backend.draw_text(label, x + 6, ry, font, colors.label)?;
            backend.draw_text(value, x + 6 + INFO_LABEL_W, ry, font, colors.text)?;
        }
        y += card_h as i32 + GAUGE_GAP + 2;

        // Gauges: label + value on one line, the bar underneath.
        let mut ctx = DrawContext::new(backend, &at.ui_theme);
        for g in self.status.gauges() {
            ctx.backend.draw_text(g.label, x, y, font, colors.label)?;
            let vw = ctx.backend.measure_text(&g.text, font) as i32;
            let vx = (x + inner_w as i32 - vw).max(x + INFO_LABEL_W);
            ctx.backend
                .draw_text(&g.text, vx, y, font, colors.value_color(g.level))?;
            let bar_y = y + GAUGE_TEXT_H;
            match g.fraction {
                Some(f) => ProgressBar::new(f).draw(&mut ctx, x, bar_y, inner_w, BAR_H)?,
                None => ctx.backend.fill_rounded_rect(
                    x,
                    bar_y,
                    inner_w,
                    BAR_H,
                    BAR_H as u16 / 2,
                    colors.na_track,
                )?,
            }
            y = bar_y + BAR_H as i32 + GAUGE_GAP;
        }

        // Footer.
        let footer_y = (cy + ch as i32 - PAD - GAUGE_TEXT_H).max(y);
        backend.draw_text(self.footer(), x, footer_y, font, colors.dim_text)
    }
}

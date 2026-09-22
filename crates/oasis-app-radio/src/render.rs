//! Windowed rendering for the Internet Radio: now-playing panel (state
//! badge, station, track, volume bar, Vol-/Vol+/Stop buttons), station
//! list and key-hint footer.
//!
//! All geometry comes from [`RadioLayout`] so drawing and hit-testing
//! agree; all colors come from the active theme via [`RadioColors`].

use oasis_skin::ActiveTheme;
use oasis_types::backend::{Color, SdiBackend};
use oasis_ui::button::{Button as UiButton, ButtonStyle};
use oasis_ui::progress_bar::ProgressBar;
use oasis_ui::{DrawContext, Widget};

use crate::layout::{LIST_HEADER_H, PanelButton, ROW_H, RadioLayout};
use crate::{PlayState, RadioApp};

/// Radio color roles, derived from the active theme. Skins can override
/// any slot via `[app_themes.radio]` in theme.toml.
#[derive(Debug, Clone)]
pub struct RadioColors {
    /// Content background.
    pub bg: Color,
    /// Now-playing panel background / border.
    pub panel_bg: Color,
    pub panel_border: Color,
    /// Primary text (station name, list rows).
    pub text: Color,
    /// Secondary text (track, genre, hints).
    pub dim_text: Color,
    /// State badge fills.
    pub playing: Color,
    pub loading: Color,
    pub stopped: Color,
    pub error: Color,
    /// Text drawn on a state badge.
    pub badge_text: Color,
    /// List header strip background / text.
    pub header_bg: Color,
    pub header_text: Color,
    /// Selected station row background / text.
    pub selected_bg: Color,
    pub selected_text: Color,
    /// Favorite star.
    pub favorite: Color,
}

impl RadioColors {
    /// Build colors from the active theme, honouring per-app overrides.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| at.app_color("radio", key).unwrap_or(default);
        let ui = &at.ui_theme;
        Self {
            bg: c("bg", at.app.bg),
            panel_bg: c("panel_bg", ui.surface),
            panel_border: c("panel_border", ui.border_subtle),
            text: c("text", ui.text_primary),
            dim_text: c("dim_text", ui.text_secondary),
            playing: c("playing", ui.success),
            loading: c("loading", ui.warning),
            stopped: c("stopped", ui.button_bg_disabled),
            error: c("error", ui.error),
            badge_text: c("badge_text", ui.text_on_accent),
            header_bg: c("header_bg", at.app.title_bar_bg),
            header_text: c("header_text", at.app.title_bar_text),
            selected_bg: c("selected_bg", at.app.selected_bg),
            selected_text: c("selected_text", at.app.selected_text),
            favorite: c("favorite", ui.warning),
        }
    }

    fn badge(&self, state: PlayState) -> Color {
        match state {
            PlayState::Playing => self.playing,
            PlayState::Loading => self.loading,
            PlayState::Stopped => self.stopped,
            PlayState::Error => self.error,
        }
    }
}

/// Cut `text` to fit `max_w` pixels, ending with `..` when shortened.
fn fit(backend: &dyn SdiBackend, text: &str, font: u16, max_w: u32) -> String {
    if backend.measure_text(text, font) <= max_w {
        return text.to_string();
    }
    let mut end = text.len();
    while end > 0 {
        end = text.floor_char_boundary(end - 1);
        let candidate = format!("{}..", &text[..end]);
        if backend.measure_text(&candidate, font) <= max_w {
            return candidate;
        }
    }
    String::new()
}

impl RadioApp {
    /// Draw the whole radio UI into the content rect.
    pub(crate) fn draw_radio(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let colors = RadioColors::from_theme(at);
        let l = RadioLayout::compute(cx, cy, cw, ch);
        self.visible_rows.set(l.visible_rows());
        backend.fill_rect(cx, cy, cw, ch, colors.bg)?;
        self.draw_panel(&l, backend, at, &colors)?;
        self.draw_list(&l, backend, at, &colors)?;
        let hint = "Enter=Tune  Space=Fav  Esc/Start=Stop  +/-=Vol";
        let hint = fit(backend, hint, at.font_hint, l.footer.w);
        backend.draw_text(
            &hint,
            l.footer.x,
            l.footer.y + 1,
            at.font_hint,
            colors.dim_text,
        )
    }

    fn draw_panel(
        &self,
        l: &RadioLayout,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
        colors: &RadioColors,
    ) -> oasis_types::error::Result<()> {
        let p = l.panel;
        let radius = at.ui_theme.border_radius_md;
        let font = at.font_hint;
        backend.fill_rounded_rect(p.x, p.y, p.w, p.h, radius, colors.panel_bg)?;
        backend.stroke_rounded_rect(p.x, p.y, p.w, p.h, radius, 1, colors.panel_border)?;

        // State badge + station name.
        let state = self.status.state;
        let label = state.label();
        let badge_w = backend.measure_text(label, font) + 8;
        backend.fill_rounded_rect(p.x + 6, p.y + 5, badge_w, 12, 3, colors.badge(state))?;
        backend.draw_text(label, p.x + 10, p.y + 7, font, colors.badge_text)?;
        let name_x = p.x + 12 + badge_w as i32;
        let name_w = (p.x + p.w as i32 - 6 - name_x).max(0) as u32;
        let station = self.status.station.as_deref().unwrap_or("No station");
        let station = fit(backend, station, at.font_body, name_w);
        backend.draw_text(&station, name_x, p.y + 6, at.font_body, colors.text)?;

        // Track / error line.
        let (line, color) = match (&self.status.error, &self.status.now_playing) {
            (Some(err), _) if state == PlayState::Error => (err.as_str(), colors.error),
            (_, Some(np)) => (np.as_str(), colors.dim_text),
            _ if state == PlayState::Stopped => {
                ("Select a station and press Enter", colors.dim_text)
            },
            _ => (self.status.state_text.as_str(), colors.dim_text),
        };
        let line = fit(backend, line, font, p.w.saturating_sub(12));
        backend.draw_text(&line, p.x + 6, p.y + 22, font, color)?;

        // Volume bar + buttons.
        let bar = l.volume_bar;
        let volume = self.volume();
        let vol_text = format!("Vol {volume}%");
        backend.draw_text(&vol_text, p.x + 6, bar.y - 1, font, colors.dim_text)?;
        let mut ctx = DrawContext::new(backend, &at.ui_theme);
        ProgressBar::new(volume as f32 / 100.0).draw(&mut ctx, bar.x, bar.y, bar.w, bar.h)?;
        for (i, kind) in PanelButton::ALL.iter().enumerate() {
            let r = l.buttons[i];
            let mut button = UiButton::new(kind.label());
            button.style = if *kind == PanelButton::Stop {
                ButtonStyle::Primary
            } else {
                ButtonStyle::Secondary
            };
            button.draw(&mut ctx, r.x, r.y, r.w, r.h)?;
        }
        Ok(())
    }

    fn draw_list(
        &self,
        l: &RadioLayout,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
        colors: &RadioColors,
    ) -> oasis_types::error::Result<()> {
        let font = at.font_hint;
        let h = l.list_header;
        backend.fill_rect(h.x, h.y, h.w, LIST_HEADER_H, colors.header_bg)?;
        let title = format!("Stations ({})", self.stations.len());
        backend.draw_text(&title, h.x + 4, h.y + 2, font, colors.header_text)?;

        let info_w = (l.list.w / 4).max(30);
        let genre_w = (l.list.w / 5).max(30);
        let name_w = l.list.w.saturating_sub(info_w + genre_w + 24);
        for (row, idx) in (self.list_scroll..self.stations.len())
            .take(l.visible_rows())
            .enumerate()
        {
            let s = &self.stations[idx];
            let r = l.row_rect(row);
            let text_color = if idx == self.selected {
                backend.fill_rect(r.x, r.y, r.w, ROW_H, colors.selected_bg)?;
                colors.selected_text
            } else {
                colors.text
            };
            let ty = r.y + 3;
            if s.favorite {
                backend.draw_text("*", r.x + 4, ty, font, colors.favorite)?;
            }
            let name = fit(backend, &s.name, font, name_w);
            backend.draw_text(&name, r.x + 14, ty, font, text_color)?;
            let genre_x = r.x + 18 + name_w as i32;
            let genre = fit(backend, &s.genre, font, genre_w);
            backend.draw_text(&genre, genre_x, ty, font, colors.dim_text)?;
            let info = fit(backend, &s.info, font, info_w);
            let info_x = r.x + r.w as i32 - 4 - backend.measure_text(&info, font) as i32;
            backend.draw_text(&info, info_x, ty, font, colors.dim_text)?;
        }
        Ok(())
    }
}

//! Conversion from `SkinTheme` to `ui::Theme` and `WmTheme`.

use oasis_types::backend::Color;
use oasis_types::color::{darken, lighten, with_alpha};
use oasis_types::shadow::Shadow;
use oasis_ui::theme::Theme;
use oasis_wm::WmTheme;

use super::overrides::WmThemeOverrides;
use super::{SkinTheme, parse_hex_color, readable_on};

impl SkinTheme {
    /// Convert the 9-color skin palette into a full `ui::Theme`.
    ///
    /// Derives all 50+ fields from the base colors using lighten/darken.
    /// Optional extended fields (`surface`, `accent_hover`, etc.) override
    /// the derived values when present.
    pub fn to_ui_theme(&self) -> Theme {
        let bg = self.background_color();
        let primary = self.primary_color();
        let secondary = self.secondary_color();
        let text = self.text_color();
        let dim = self.dim_text_color();
        let err = self.error_color();

        // Surface variants: lighten background by 5% and 10%.
        let surface = self
            .surface
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or_else(|| lighten(bg, 0.05));
        let surface_variant = lighten(bg, 0.10);

        // Accent variants: explicit `accent` override, else primary.
        let accent = self
            .accent
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or(primary);
        let accent_hover = self
            .accent_hover
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or_else(|| lighten(accent, 0.15));
        let accent_pressed = darken(accent, 0.85);
        let accent_subtle = with_alpha(accent, 30);
        // Labels drawn on the accent fill (primary buttons) must stay readable
        // even when the base text color is close to the accent (white on a
        // bright green or cyan primary was 1.2-1.6:1 in several skins).
        let on_accent = readable_on(text, accent, 4.5);

        // Border radius and shadow from extended fields.
        let radius = self.border_radius.unwrap_or(4);
        let shadow_level = self.shadow_intensity.unwrap_or(1);

        // Focus ring: only explicit `[geometry]` values flow through.
        // Unset fields stay `None` so `FocusStyle` keeps deriving from
        // the accent color, exactly as before this field existed.
        let geometry = self.geometry.as_ref();
        let focus_ring_color = geometry
            .and_then(|g| g.focus_ring_color.as_ref())
            .and_then(|s| parse_hex_color(s));
        let focus_ring_width = geometry.and_then(|g| g.focus_ring_width);
        let focus_ring_offset = geometry.and_then(|g| g.focus_ring_offset);

        let success = self
            .success
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or_else(|| Color::rgb(80, 200, 120));
        let warning = self
            .warning
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or_else(|| Color::rgb(255, 180, 50));

        let mut theme = Theme {
            background: bg,
            surface,
            surface_variant,
            overlay: Color::rgba(0, 0, 0, 180),

            text_primary: text,
            text_secondary: dim,
            text_disabled: darken(dim, 0.6),
            text_on_accent: on_accent,

            accent,
            accent_hover,
            accent_pressed,
            accent_subtle,

            success,
            warning,
            error: err,
            info: accent,

            border: secondary,
            border_subtle: darken(secondary, 0.7),
            border_strong: primary,

            button_bg: secondary,
            button_bg_hover: lighten(secondary, 0.15),
            button_bg_pressed: darken(secondary, 0.85),
            button_bg_disabled: darken(secondary, 0.5),
            // Resolved below, once `[widget_states.button]` has had its say.
            button_text: text,
            input_bg: darken(bg, 0.8),
            input_border: secondary,
            input_border_focus: primary,
            scrollbar_track: Color::rgba(255, 255, 255, 10),
            scrollbar_thumb: Color::rgba(255, 255, 255, 40),
            scrollbar_thumb_hover: Color::rgba(255, 255, 255, 80),
            toggle_track_off: Color::rgba(255, 255, 255, 10),
            toggle_track_on: accent,
            toggle_thumb: text,
            // Slider slots default to the fields the widget used to
            // borrow (input_bg / accent / surface).
            slider_track: darken(bg, 0.8),
            slider_fill: accent,
            slider_thumb: surface,
            // Menu slots keep the MenuBar widget's classic Win95 grays
            // until a skin overrides `[widget_states.menu]`.
            menu_bg: Color::rgb(240, 240, 240),
            menu_border: Color::rgb(180, 180, 180),
            menu_text: Color::rgb(30, 30, 30),
            menu_hover_bg: Color::rgb(49, 106, 197),
            menu_hover_text: Color::rgb(255, 255, 255),
            menu_dropdown_bg: Color::rgb(236, 236, 236),
            menu_dropdown_border_light: Color::rgb(255, 255, 255),
            menu_dropdown_border_dark: Color::rgb(105, 105, 105),
            menu_item_text: Color::rgb(20, 20, 20),
            menu_disabled_text: Color::rgb(150, 150, 150),
            menu_separator: Color::rgb(170, 170, 170),
            tooltip_bg: lighten(bg, 0.15),
            tooltip_text: text,

            focus_ring_color,
            focus_ring_width,
            focus_ring_offset,

            font_size_xs: 8,
            font_size_sm: 8,
            font_size_md: 8,
            font_size_lg: 16,
            font_size_xl: 16,
            font_size_xxl: 24,

            spacing_xs: 2,
            spacing_sm: 4,
            spacing_md: 8,
            spacing_lg: 12,
            spacing_xl: 16,

            border_radius_sm: (radius / 2).max(1),
            border_radius_md: radius,
            border_radius_lg: radius * 2,
            border_radius_xl: radius * 3,

            shadow_card: Shadow::elevation(shadow_level.min(1)),
            shadow_dropdown: Shadow::elevation(shadow_level.min(2)),
            shadow_modal: Shadow::elevation(shadow_level.min(3)),
            shadow_tooltip: Shadow::elevation(shadow_level.min(2)),

            reduced_motion: false,
            font_scale: 1.0,
            text_direction: oasis_types::text_direction::TextDirection::Ltr,
        };

        if let Some(ref typo) = self.typography {
            apply_typography(&mut theme, typo);
        }
        if let Some(ref states) = self.widget_states {
            apply_widget_states(&mut theme, states);
        }
        // Button labels: an explicit `[widget_states.button] text` is used
        // as given; otherwise the base text colour, pushed towards black or
        // white until it reads on the (possibly overridden) button fill.
        theme.button_text = widget_state_color(self.widget_states.as_ref(), "button", "text")
            .unwrap_or_else(|| readable_on(text, theme.button_bg, 4.5));

        // Density scales the resolved spacing ladder only; `comfortable`
        // (the default) is an exact integer identity, so unset skins are
        // pixel-identical.
        let density = self.density_preset();
        theme.spacing_xs = density.scale(theme.spacing_xs);
        theme.spacing_sm = density.scale(theme.spacing_sm);
        theme.spacing_md = density.scale(theme.spacing_md);
        theme.spacing_lg = density.scale(theme.spacing_lg);
        theme.spacing_xl = density.scale(theme.spacing_xl);

        // Accessibility `text_scale` multiplies the resolved font-size ladder
        // globally. At the default `1.0` these are unchanged.
        theme.font_size_xs = self.scale_font(theme.font_size_xs);
        theme.font_size_sm = self.scale_font(theme.font_size_sm);
        theme.font_size_md = self.scale_font(theme.font_size_md);
        theme.font_size_lg = self.scale_font(theme.font_size_lg);
        theme.font_size_xl = self.scale_font(theme.font_size_xl);
        theme.font_size_xxl = self.scale_font(theme.font_size_xxl);

        theme
    }

    /// Build a `WmTheme` from the defaults plus any overrides.
    pub fn build_wm_theme(&self) -> WmTheme {
        let mut theme = WmTheme::default();
        if let Some(ref ov) = self.wm_theme {
            apply_wm_overrides(&mut theme, ov);
        }
        // Default inactive title text to the active text color when unset,
        // so a custom `titlebar_text` carries over to unfocused windows.
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.titlebar_text_inactive.as_ref())
            .is_none()
        {
            theme.titlebar_text_inactive_color = theme.titlebar_text_color;
        }
        // Default glyph colors to titlebar_text_color if not explicitly set.
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.glyph_close_color.as_ref())
            .is_none()
        {
            theme.glyph_close_color = theme.titlebar_text_color;
        }
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.glyph_minimize_color.as_ref())
            .is_none()
        {
            theme.glyph_minimize_color = theme.titlebar_text_color;
        }
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.glyph_maximize_color.as_ref())
            .is_none()
        {
            theme.glyph_maximize_color = theme.titlebar_text_color;
        }
        // Default hover colors to lighten(btn_color, 0.15) if not explicitly set.
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.btn_close_hover.as_ref())
            .is_none()
        {
            theme.btn_close_hover = lighten(theme.btn_close_color, 0.15);
        }
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.btn_minimize_hover.as_ref())
            .is_none()
        {
            theme.btn_minimize_hover = lighten(theme.btn_minimize_color, 0.15);
        }
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.btn_maximize_hover.as_ref())
            .is_none()
        {
            theme.btn_maximize_hover = lighten(theme.btn_maximize_color, 0.15);
        }
        theme
    }
}

/// Apply the `[typography]` scale onto a derived `Theme`.
///
/// Unset fields keep the derivation's defaults, so skins written before this
/// existed produce a byte-identical `Theme`.
fn apply_typography(theme: &mut Theme, typo: &super::TypographyOverrides) {
    for (slot, value) in [
        (&mut theme.font_size_xs, typo.font_size_xs),
        (&mut theme.font_size_sm, typo.font_size_sm),
        (&mut theme.font_size_md, typo.font_size_md),
        (&mut theme.font_size_lg, typo.font_size_lg),
        (&mut theme.font_size_xl, typo.font_size_xl),
        (&mut theme.font_size_xxl, typo.font_size_xxl),
        (&mut theme.spacing_xs, typo.spacing_xs),
        (&mut theme.spacing_sm, typo.spacing_sm),
        (&mut theme.spacing_md, typo.spacing_md),
        (&mut theme.spacing_lg, typo.spacing_lg),
        (&mut theme.spacing_xl, typo.spacing_xl),
    ] {
        if let Some(v) = value {
            *slot = v;
        }
    }
}

/// Apply `[widget_states.*]` color overrides onto a derived `Theme`.
///
/// Recognized slots (unknown widgets/keys are ignored; `skin lint` warns):
/// - `button`: `normal_bg`, `hover_bg`, `pressed_bg`, `disabled_bg`,
///   `disabled_text`
/// - `input`: `bg`, `border`, `focus_border`
/// - `toggle`: `track_off`, `track_on`, `thumb`
/// - `scrollbar`: `track`, `thumb`, `thumb_hover`
/// - `slider`: `track`, `fill`, `thumb`
/// - `menu`: `bg`, `border`, `text`, `hover_bg`, `hover_text`,
///   `dropdown_bg`, `dropdown_border_light`, `dropdown_border_dark`,
///   `item_text`, `disabled_text`, `separator`
fn apply_widget_states(
    theme: &mut Theme,
    states: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
) {
    let get = |widget: &str, key: &str| widget_state_color(Some(states), widget, key);
    for (slot, key, widget_key) in [
        (&mut theme.button_bg, "button", "normal_bg"),
        (&mut theme.button_bg_hover, "button", "hover_bg"),
        (&mut theme.button_bg_pressed, "button", "pressed_bg"),
        (&mut theme.button_bg_disabled, "button", "disabled_bg"),
        (&mut theme.text_disabled, "button", "disabled_text"),
        (&mut theme.input_bg, "input", "bg"),
        (&mut theme.input_border, "input", "border"),
        (&mut theme.input_border_focus, "input", "focus_border"),
        (&mut theme.toggle_track_off, "toggle", "track_off"),
        (&mut theme.toggle_track_on, "toggle", "track_on"),
        (&mut theme.toggle_thumb, "toggle", "thumb"),
        (&mut theme.scrollbar_track, "scrollbar", "track"),
        (&mut theme.scrollbar_thumb, "scrollbar", "thumb"),
        (&mut theme.scrollbar_thumb_hover, "scrollbar", "thumb_hover"),
        (&mut theme.slider_track, "slider", "track"),
        (&mut theme.slider_fill, "slider", "fill"),
        (&mut theme.slider_thumb, "slider", "thumb"),
        (&mut theme.menu_bg, "menu", "bg"),
        (&mut theme.menu_border, "menu", "border"),
        (&mut theme.menu_text, "menu", "text"),
        (&mut theme.menu_hover_bg, "menu", "hover_bg"),
        (&mut theme.menu_hover_text, "menu", "hover_text"),
        (&mut theme.menu_dropdown_bg, "menu", "dropdown_bg"),
        (
            &mut theme.menu_dropdown_border_light,
            "menu",
            "dropdown_border_light",
        ),
        (
            &mut theme.menu_dropdown_border_dark,
            "menu",
            "dropdown_border_dark",
        ),
        (&mut theme.menu_item_text, "menu", "item_text"),
        (&mut theme.menu_disabled_text, "menu", "disabled_text"),
        (&mut theme.menu_separator, "menu", "separator"),
    ] {
        if let Some(c) = get(key, widget_key) {
            *slot = c;
        }
    }
}

/// The colour `[widget_states.<widget>] <key>` sets, if any.
fn widget_state_color(
    states: Option<&std::collections::HashMap<String, std::collections::HashMap<String, String>>>,
    widget: &str,
    key: &str,
) -> Option<Color> {
    states?
        .get(widget)
        .and_then(|m| m.get(key))
        .and_then(|hex| parse_hex_color(hex))
}

/// Apply all WM theme overrides to a `WmTheme`.
///
/// Unset fields keep the derived value; color strings that do not parse
/// are ignored here (`skin lint` reports them).
fn apply_wm_overrides(theme: &mut WmTheme, ov: &WmThemeOverrides) {
    // `ov.field` (Copy) -> `theme.target`.
    macro_rules! copy {
        ($($field:ident => $target:ident),* $(,)?) => {$(
            if let Some(v) = ov.$field {
                theme.$target = v;
            }
        )*};
    }
    // `ov.field` (String) -> `theme.target`.
    macro_rules! string {
        ($($field:ident),* $(,)?) => {$(
            if let Some(ref v) = ov.$field {
                theme.$field.clone_from(v);
            }
        )*};
    }
    // `ov.field` (hex) -> `theme.target` (a `Color`).
    macro_rules! color {
        ($($field:ident => $target:ident),* $(,)?) => {$(
            if let Some(c) = ov.$field.as_deref().and_then(parse_hex_color) {
                theme.$target = c;
            }
        )*};
    }
    // `ov.field` (hex) -> `theme.field` (an `Option<Color>`).
    macro_rules! opt_color {
        ($($field:ident),* $(,)?) => {$(
            if let Some(c) = ov.$field.as_deref().and_then(parse_hex_color) {
                theme.$field = Some(c);
            }
        )*};
    }

    copy!(
        titlebar_height => titlebar_height,
        border_width => border_width,
        button_size => button_size,
        resize_handle_size => resize_handle_size,
        titlebar_font_size => titlebar_font_size,
        titlebar_radius => titlebar_radius,
        titlebar_gradient => titlebar_gradient,
        frame_shadow_level => frame_shadow_level,
        frame_border_radius => frame_border_radius,
        button_radius => button_radius,
        separator_enabled => separator_enabled,
        button_spacing => button_spacing,
        title_text_shadow => title_text_shadow,
        content_stroke_width => content_stroke_width,
        maximize_top_inset => maximize_top_inset,
        maximize_bottom_inset => maximize_bottom_inset,
        inactive_frame_alpha => inactive_frame_alpha,
    );
    string!(
        button_side,
        glyph_close,
        glyph_minimize,
        glyph_maximize,
        title_align
    );
    color!(
        titlebar_active => titlebar_active_color,
        titlebar_inactive => titlebar_inactive_color,
        titlebar_text => titlebar_text_color,
        // `titlebar_text_active` is a synonym that takes precedence.
        titlebar_text_active => titlebar_text_color,
        titlebar_text_inactive => titlebar_text_inactive_color,
        frame_color => frame_color,
        content_bg => content_bg_color,
        btn_close => btn_close_color,
        btn_minimize => btn_minimize_color,
        btn_maximize => btn_maximize_color,
        separator_color => separator_color,
        glyph_close_color => glyph_close_color,
        glyph_minimize_color => glyph_minimize_color,
        glyph_maximize_color => glyph_maximize_color,
        btn_close_hover => btn_close_hover,
        btn_minimize_hover => btn_minimize_hover,
        btn_maximize_hover => btn_maximize_hover,
        title_text_shadow_color => title_text_shadow_color,
        content_stroke_color => content_stroke_color,
        modal_overlay_color => modal_overlay_color,
    );
    opt_color!(
        titlebar_gradient_top,
        titlebar_gradient_bottom,
        titlebar_inactive_gradient_top,
        titlebar_inactive_gradient_bottom,
    );
    if let Some(ref np) = ov.titlebar_nine_patch {
        theme.titlebar_nine_patch = Some((np.image.clone(), np.insets));
    }
    if let Some(ref np) = ov.frame_nine_patch {
        theme.frame_nine_patch = Some((np.image.clone(), np.insets));
    }
}

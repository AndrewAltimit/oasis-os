#[cfg(test)]
mod tests {
    use crate::SkinTheme;
    use crate::active_theme::ActiveTheme;
    use oasis_types::backend::Color;

    #[test]
    fn default_matches_legacy_theme() {
        let at = ActiveTheme::default();
        assert_eq!(at.bar.statusbar_bg, Color::rgba(0, 0, 0, 80));
        assert_eq!(at.bar.bg, Color::rgba(0, 0, 0, 90));
        assert_eq!(at.bar.battery_color, Color::rgb(120, 255, 120));
        assert_eq!(at.icon.border_radius, 4);
        assert_eq!(at.icon.cursor_border_radius, 6);
    }

    #[test]
    fn with_features_propagates_reduced_motion() {
        let skin = SkinTheme::default();
        let at = ActiveTheme::from_skin(&skin);
        assert!(!at.ui_theme.reduced_motion);

        let mut features = crate::loader::SkinFeatures::default();
        features.reduced_motion = true;
        let at = ActiveTheme::from_skin(&skin).with_features(&features);
        assert!(at.ui_theme.reduced_motion);
    }

    #[test]
    fn from_skin_derives_colors() {
        let skin = SkinTheme::default();
        let at = ActiveTheme::from_skin(&skin);
        // Primary is #3264C8 -- tab_active_fill should use primary with alpha 30.
        assert_eq!(at.bar.tab_active_fill.a, 30);
        // Cursor color should use primary with alpha 80.
        assert_eq!(at.icon.cursor_color.a, 80);
        // Text color drives version/clock.
        assert_eq!(at.bar.version_color, skin.text_color());
        assert_eq!(at.bar.clock_color, skin.text_color());
    }

    #[test]
    fn from_skin_respects_bar_overrides() {
        let toml = r##"
background = "#000000"
primary = "#FF0000"
[bar_overrides]
battery_color = "#00FF00"
tab_active_alpha = 200
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.bar.battery_color, Color::rgb(0, 255, 0));
        assert_eq!(at.bar.tab_active_alpha, 200);
    }

    #[test]
    fn from_skin_respects_icon_overrides() {
        let toml = r##"
[icon_overrides]
body_color = "#AABBCC"
cursor_border_radius = 10
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.icon.body_color, Color::rgb(0xAA, 0xBB, 0xCC));
        assert_eq!(at.icon.cursor_border_radius, 10);
    }

    #[test]
    fn from_skin_custom_theme() {
        let toml = r##"
background = "#000000"
primary = "#FF0000"
secondary = "#333333"
text = "#00FF00"
dim_text = "#006600"
status_bar = "#111111"
prompt = "#00FF00"
output = "#00CC00"
error = "#FF0000"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        // Text-derived fields should be green.
        assert_eq!(at.bar.clock_color, Color::rgb(0, 255, 0));
        assert_eq!(at.bar.media_tab_active, Color::rgb(0, 255, 0));
    }

    #[test]
    fn app_color_returns_none_by_default() {
        let at = ActiveTheme::default();
        assert!(at.app_color("tv_guide", "bg").is_none());
    }

    #[test]
    fn app_color_from_theme_toml() {
        let toml = r##"
[app_themes.tv_guide]
bg = "#0A1628"
grid_line = "#1A3A5C"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.app_color("tv_guide", "bg"), Some(Color::rgb(10, 22, 40)));
        assert_eq!(
            at.app_color("tv_guide", "grid_line"),
            Some(Color::rgb(26, 58, 92))
        );
        assert!(at.app_color("tv_guide", "missing").is_none());
        assert!(at.app_color("unknown", "bg").is_none());
    }

    #[test]
    fn gradient_returns_none_by_default() {
        let at = ActiveTheme::default();
        assert!(at.gradient("primary").is_none());
    }

    #[test]
    fn gradient_from_theme_toml() {
        let toml = r##"
[gradients.primary]
from = "#0066FF"
to = "#0044AA"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        let (from, to) = at.gradient("primary").unwrap();
        assert_eq!(from, Color::rgb(0x00, 0x66, 0xFF));
        assert_eq!(to, Color::rgb(0x00, 0x44, 0xAA));
    }

    #[test]
    fn animation_returns_none_by_default() {
        let at = ActiveTheme::default();
        assert!(at.animation("button_press").is_none());
    }

    #[test]
    fn animation_from_theme_toml() {
        let toml = r##"
[animations.button_press]
duration_ms = 100
easing = "ease_out_quad"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        let (dur, easing) = at.animation("button_press").unwrap();
        assert_eq!(dur, 100);
        assert_eq!(easing, "ease_out_quad");
    }

    #[test]
    fn resolve_animation_uses_preset() {
        let toml = r##"
[animations.cursor_move]
duration_ms = 150
easing = "ease_out_cubic"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        let (dur, easing_fn) = at.resolve_animation("cursor_move", 200);
        assert_eq!(dur, 150);
        let val = easing_fn(0.5);
        assert!(val > 0.5, "ease_out_cubic at 0.5 should be > 0.5");
    }

    #[test]
    fn resolve_animation_falls_back() {
        let at = ActiveTheme::default();
        let (dur, easing_fn) = at.resolve_animation("nonexistent", 300);
        assert_eq!(dur, 300);
        assert!((easing_fn(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn widget_state_color_returns_none_by_default() {
        let at = ActiveTheme::default();
        assert!(at.widget_state_color("button", "hover_bg").is_none());
    }

    #[test]
    fn widget_state_color_from_theme_toml() {
        let toml = r##"
[widget_states.button]
normal_bg = "#505050"
hover_bg = "#656565"
pressed_bg = "#353535"
disabled_text = "#555555"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(
            at.widget_state_color("button", "hover_bg"),
            Some(Color::rgb(0x65, 0x65, 0x65))
        );
        assert_eq!(
            at.widget_state_color("button", "disabled_text"),
            Some(Color::rgb(0x55, 0x55, 0x55))
        );
        assert!(at.widget_state_color("button", "missing_key").is_none());
        assert!(at.widget_state_color("slider", "hover_bg").is_none());
    }

    #[test]
    fn from_base_colors_produces_valid_theme() {
        let at = ActiveTheme::from_base_colors(
            Color::rgb(0x0E, 0x0E, 0x1C), // background
            Color::rgb(0x44, 0x88, 0xCC), // primary
            Color::rgb(0x2A, 0x2A, 0x3E), // secondary
            Color::rgb(0xE0, 0xE0, 0xF0), // text
            Color::rgb(0x70, 0x70, 0x90), // dim_text
            Color::rgb(0x18, 0x18, 0x2C), // status_bar
            Color::rgb(0x44, 0xCC, 0x88), // prompt
            Color::rgb(0xC0, 0xC0, 0xD8), // output
            Color::rgb(0xFF, 0x44, 0x66), // error
        );
        // Bar theme: statusbar_bg derived from status_bar with alpha 80.
        assert_eq!(at.bar.statusbar_bg.r, 0x18);
        assert_eq!(at.bar.statusbar_bg.a, 80);
        // Tab active fill from primary with alpha 30.
        assert_eq!(at.bar.tab_active_fill.a, 30);
        // Icon body color matches text.
        assert_eq!(at.icon.body_color, Color::rgb(0xE0, 0xE0, 0xF0));
        // Terminal colors match prompt/output.
        assert_eq!(at.app.terminal_prompt_color, Color::rgb(0x44, 0xCC, 0x88));
        assert_eq!(at.app.terminal_output_color, Color::rgb(0xC0, 0xC0, 0xD8));
        // UI theme accent matches primary.
        assert_eq!(at.ui_theme.accent, Color::rgb(0x44, 0x88, 0xCC));
        // Toast error from error color.
        assert_eq!(at.toast.error_bg.r, 0xFF);
        assert_eq!(at.toast.error_bg.a, 220);
        // Screen defaults.
        assert_eq!(at.screen_w, 480);
        assert_eq!(at.screen_h, 272);
    }

    #[test]
    fn from_base_colors_matches_from_skin_defaults() {
        // The SkinTheme defaults produce specific colors; from_base_colors
        // with the same 9 colors should produce the same derivation for
        // shared fields (no overrides path).
        let skin = SkinTheme::default();
        let from_skin = ActiveTheme::from_skin(&skin);
        let from_colors = ActiveTheme::from_base_colors(
            skin.background_color(),
            skin.primary_color(),
            skin.secondary_color(),
            skin.text_color(),
            skin.dim_text_color(),
            crate::theme::parse_hex_color(&skin.status_bar).unwrap(),
            skin.prompt_color(),
            skin.output_color(),
            skin.error_color(),
        );
        // Core derivation: tab fill, cursor color, battery color.
        assert_eq!(
            from_skin.bar.tab_active_fill,
            from_colors.bar.tab_active_fill
        );
        assert_eq!(from_skin.icon.cursor_color, from_colors.icon.cursor_color);
        assert_eq!(from_skin.bar.battery_color, from_colors.bar.battery_color);
        assert_eq!(from_skin.bar.version_color, from_colors.bar.version_color);
        assert_eq!(
            from_skin.app.terminal_prompt_color,
            from_colors.app.terminal_prompt_color
        );
    }
}

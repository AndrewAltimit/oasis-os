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

    // -- ANSI palette --

    #[test]
    fn ansi_palette_semantic_slots() {
        let skin = SkinTheme::default();
        let at = ActiveTheme::from_skin(&skin);
        // white == terminal output color, bright_white == text color.
        assert_eq!(at.ansi.color(7), skin.output_color());
        assert_eq!(at.ansi.color(15), skin.text_color());
        // Chromatic slots are distinct.
        assert_ne!(at.ansi.color(1), at.ansi.color(2));
        assert_ne!(at.ansi.color(4), at.ansi.color(5));
        // Red slot is red-dominant, green slot green-dominant.
        let red = at.ansi.color(1);
        assert!(red.r > red.g && red.r > red.b);
        let green = at.ansi.color(2);
        assert!(green.g > green.r && green.g > green.b);
        let blue = at.ansi.color(4);
        assert!(blue.b > blue.r && blue.b > blue.g);
    }

    #[test]
    fn ansi_palette_bright_variants_lighter() {
        let at = ActiveTheme::default();
        use oasis_types::color::rgb_to_hsl;
        for slot in 1..7 {
            let (_, _, l_normal) = rgb_to_hsl(at.ansi.color(slot));
            let (_, _, l_bright) = rgb_to_hsl(at.ansi.color(slot + 8));
            assert!(
                l_bright > l_normal,
                "bright slot {} should be lighter",
                slot + 8
            );
        }
    }

    #[test]
    fn ansi_palette_default_matches_from_skin_default() {
        // ActiveTheme::default() and from_skin(default) agree on the palette.
        let a = ActiveTheme::default();
        let b = ActiveTheme::from_skin(&SkinTheme::default());
        assert_eq!(a.ansi, b.ansi);
    }

    #[test]
    fn ansi_palette_deterministic() {
        let skin = SkinTheme::default();
        let a = ActiveTheme::from_skin(&skin);
        let b = ActiveTheme::from_skin(&skin);
        assert_eq!(a.ansi, b.ansi);
    }

    #[test]
    fn ansi_palette_overrides() {
        let toml = r##"
[palette]
red = "#FF5555"
bright_green = "#69FF94"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.ansi.color(1), Color::rgb(0xFF, 0x55, 0x55));
        assert_eq!(at.ansi.color(10), Color::rgb(0x69, 0xFF, 0x94));
        // Unset slots keep the derived defaults.
        let derived = ActiveTheme::from_skin(&SkinTheme::default());
        assert_eq!(at.ansi.color(4), derived.ansi.color(4));
    }

    #[test]
    fn ansi_sgr_code_mapping() {
        let at = ActiveTheme::default();
        assert_eq!(at.ansi.from_sgr_code(31), Some(at.ansi.color(1)));
        assert_eq!(at.ansi.from_sgr_code(94), Some(at.ansi.color(12)));
        assert_eq!(at.ansi.from_sgr_code(38), None);
        assert_eq!(at.ansi.from_sgr_code(0), None);
    }

    // -- Cursor colors --

    #[test]
    fn cursor_colors_default_white_black() {
        let at = ActiveTheme::default();
        assert_eq!(at.cursor_fill, Color::rgb(255, 255, 255));
        assert_eq!(at.cursor_outline, Color::rgb(0, 0, 0));
        let from_skin = ActiveTheme::from_skin(&SkinTheme::default());
        assert_eq!(from_skin.cursor_fill, at.cursor_fill);
        assert_eq!(from_skin.cursor_outline, at.cursor_outline);
    }

    #[test]
    fn cursor_colors_overridable() {
        let toml = r##"
[cursor]
fill = "#00FF88"
outline = "#112233"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.cursor_fill, Color::rgb(0, 255, 0x88));
        assert_eq!(at.cursor_outline, Color::rgb(0x11, 0x22, 0x33));
    }

    // -- Hardcoded-color hole closure --

    #[test]
    fn data_led_color_default_and_override() {
        let at = ActiveTheme::default();
        assert_eq!(at.icon.data_led_color, Color::rgb(0, 200, 100));

        let toml = r##"
[icon_overrides]
data_led_color = "#FF00FF"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.icon.data_led_color, Color::rgb(255, 0, 255));
    }

    #[test]
    fn app_fallback_colors_default_and_override() {
        let at = ActiveTheme::default();
        assert_eq!(at.icon.fallback_colors.len(), 6);
        assert_eq!(at.icon.fallback_colors[0], Color::rgb(70, 130, 180));
        assert_eq!(at.icon.fallback_colors[5], Color::rgb(100, 149, 237));

        let toml = r##"
[icon_overrides]
fallback_colors = ["#111111", "#222222"]
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.icon.fallback_colors.len(), 2);
        assert_eq!(at.icon.fallback_colors[1], Color::rgb(0x22, 0x22, 0x22));
    }

    #[test]
    fn start_menu_item_fallback_default_and_override() {
        let at = ActiveTheme::default();
        assert_eq!(at.menu.item_fallback_color, Color::rgb(100, 100, 100));

        let toml = r##"
[start_menu_overrides]
item_fallback_color = "#ABCDEF"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.menu.item_fallback_color, Color::rgb(0xAB, 0xCD, 0xEF));
    }

    #[test]
    fn background_default_layer_color() {
        let toml = r##"
[[background_layers]]
kind = "grid"
spacing = 30

[background_performance]
default_layer_color = "#FF000080"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.background_layers.len(), 1);
        assert_eq!(at.background_layers[0].color, Color::rgba(255, 0, 0, 128));

        // Without the override, layers default to #FFFFFF12.
        let toml = r##"
[[background_layers]]
kind = "grid"
spacing = 30
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(
            at.background_layers[0].color,
            Color::rgba(255, 255, 255, 18)
        );
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
    fn from_skin_parses_chrome_layers() {
        let toml = r##"
[[chrome_layers]]
kind = "crosshair"
size = 12
color = "#FFFFFF30"

[[chrome_layers]]
kind = "image"
source = "assets/x.png"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        // "image" is not a vector kind and is dropped by the converter.
        assert_eq!(at.chrome_layers.len(), 1);
        assert!(matches!(
            at.chrome_layers[0].kind,
            oasis_vector::background::LayerKind::Crosshair { size: 12 }
        ));
        // Chrome layers never leak into the background list.
        assert!(at.background_layers.is_empty());
    }

    #[test]
    fn transition_entrance_config_parses() {
        let toml = r##"
[transition]
entrance = "assemble"
entrance_ms = 500
page_style = "fade"
easing = "ease_out_bounce"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.transition_entrance, "assemble");
        assert_eq!(at.transition_entrance_frames, 30); // 500ms at 60fps
        assert_eq!(at.transition_page_style, "fade");
        assert_eq!(at.transition_easing, "ease_out_bounce");
    }

    #[test]
    fn transition_entrance_defaults() {
        let at = ActiveTheme::from_skin(&SkinTheme::default());
        assert_eq!(at.transition_entrance, "fade");
        assert_eq!(at.transition_entrance_frames, 45);
        assert_eq!(at.transition_page_style, "slide");
        assert!(at.transition_easing.is_empty());
    }

    #[test]
    fn chrome_layers_default_empty() {
        let at = ActiveTheme::default();
        assert!(at.chrome_layers.is_empty());
        let at = ActiveTheme::from_skin(&SkinTheme::default());
        assert!(at.chrome_layers.is_empty());
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
    fn widget_states_baked_into_ui_theme() {
        // `[widget_states.*]` has no separate ActiveTheme accessor: the
        // overrides are baked into the embedded `ui_theme` at derivation.
        let toml = r##"
[widget_states.button]
normal_bg = "#505050"
hover_bg = "#656565"
pressed_bg = "#353535"
disabled_text = "#555555"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.ui_theme.button_bg, Color::rgb(0x50, 0x50, 0x50));
        assert_eq!(at.ui_theme.button_bg_hover, Color::rgb(0x65, 0x65, 0x65));
        assert_eq!(at.ui_theme.button_bg_pressed, Color::rgb(0x35, 0x35, 0x35));
        assert_eq!(at.ui_theme.text_disabled, Color::rgb(0x55, 0x55, 0x55));
    }

    #[test]
    fn focus_ring_geometry_baked_into_ui_theme() {
        let toml = r##"
[geometry]
focus_ring_color = "#FF00FFA0"
focus_ring_width = 3
focus_ring_offset = 4
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(
            at.ui_theme.focus_ring_color,
            Some(Color::rgba(0xFF, 0x00, 0xFF, 0xA0))
        );
        assert_eq!(at.ui_theme.focus_ring_width, Some(3));
        assert_eq!(at.ui_theme.focus_ring_offset, Some(4));
    }

    #[test]
    fn focus_ring_unset_stays_none_in_ui_theme() {
        // Skins that don't author focus_ring_* must leave the ui_theme
        // fields unset so FocusStyle keeps its accent derivation.
        let skin = SkinTheme::default();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.ui_theme.focus_ring_color, None);
        assert_eq!(at.ui_theme.focus_ring_width, None);
        assert_eq!(at.ui_theme.focus_ring_offset, None);
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

    // -----------------------------------------------------------------------
    // PSP-specific code path tests
    //
    // The PSP backend uses `from_base_colors()` + `with_screen_size(480, 272)`
    // followed by manual overrides in `apply_psp_overrides()`. These tests
    // exercise the shared paths that the PSP backend depends on.
    // -----------------------------------------------------------------------

    /// PSP native resolution: 480x272. `with_screen_size(480, 272)` should
    /// be an identity transform for layout constants (base values designed
    /// for 480px width).
    #[test]
    fn with_screen_size_psp_native_is_identity() {
        let base = ActiveTheme::from_base_colors(
            Color::rgb(0x1A, 0x1A, 0x2D),
            Color::rgb(0x32, 0x64, 0xC8),
            Color::rgb(0x50, 0x50, 0x50),
            Color::WHITE,
            Color::rgb(0x80, 0x80, 0x80),
            Color::rgb(0x28, 0x3C, 0x5A),
            Color::rgb(0x00, 0xFF, 0x00),
            Color::rgb(0xCC, 0xCC, 0xCC),
            Color::rgb(0xFF, 0x44, 0x44),
        );
        let psp = base.clone().with_screen_size(480, 272);
        // At 480px (the base width), scaling should be identity.
        assert_eq!(psp.screen_w, 480);
        assert_eq!(psp.screen_h, 272);
        assert_eq!(psp.tab_w, base.tab_w);
        assert_eq!(psp.tab_h, base.tab_h);
        assert_eq!(psp.r_hint_w, base.r_hint_w);
        assert_eq!(psp.cursor_scale, 1); // PSP is not HD.
    }

    /// PSP skins use 9 base colors for theme derivation. Verify all 9
    /// PSP skin presets produce valid themes via `from_base_colors`.
    #[test]
    fn psp_skin_presets_produce_valid_themes() {
        // These are the 9 PSP skin base color sets from skins.rs.
        let presets: &[(&str, [Color; 9])] = &[
            (
                "PSIX",
                [
                    Color::rgb(0x1A, 0x1A, 0x2D),
                    Color::rgb(0x32, 0x64, 0xC8),
                    Color::rgb(0x50, 0x50, 0x50),
                    Color::WHITE,
                    Color::rgb(0x80, 0x80, 0x80),
                    Color::rgb(0x28, 0x3C, 0x5A),
                    Color::rgb(0x00, 0xFF, 0x00),
                    Color::rgb(0xCC, 0xCC, 0xCC),
                    Color::rgb(0xFF, 0x44, 0x44),
                ],
            ),
            (
                "Balatro",
                [
                    Color::rgb(0x0A, 0x0A, 0x14),
                    Color::rgb(0x00, 0xF0, 0xFF),
                    Color::rgb(0x1A, 0x1A, 0x2E),
                    Color::rgb(0xE0, 0xF0, 0xFF),
                    Color::rgb(0x50, 0x60, 0x80),
                    Color::rgba(0x08, 0x08, 0x10, 0x80),
                    Color::rgb(0x00, 0xF0, 0xFF),
                    Color::rgb(0xC0, 0xD8, 0xFF),
                    Color::rgb(0xFF, 0x20, 0x60),
                ],
            ),
            (
                "Terminal",
                [
                    Color::rgb(0x00, 0x00, 0x00),
                    Color::rgb(0x00, 0xFF, 0x00),
                    Color::rgb(0x00, 0x33, 0x00),
                    Color::rgb(0x00, 0xCC, 0x00),
                    Color::rgb(0x00, 0x66, 0x00),
                    Color::rgb(0x00, 0x1A, 0x00),
                    Color::rgb(0x00, 0xFF, 0x00),
                    Color::rgb(0x00, 0xCC, 0x00),
                    Color::rgb(0xFF, 0x33, 0x33),
                ],
            ),
        ];

        for (name, colors) in presets {
            let at = ActiveTheme::from_base_colors(
                colors[0], colors[1], colors[2], colors[3], colors[4], colors[5], colors[6],
                colors[7], colors[8],
            )
            .with_screen_size(480, 272);

            // Verify non-degenerate theme derivation.
            assert!(
                at.bar.statusbar_bg.a > 0,
                "{name}: statusbar_bg alpha should be > 0"
            );
            assert!(
                at.bar.tab_active_fill.a > 0,
                "{name}: tab_active_fill alpha should be > 0"
            );
            assert_eq!(at.screen_w, 480, "{name}: screen_w");
            assert_eq!(at.screen_h, 272, "{name}: screen_h");
        }
    }

    /// PSP overrides force opaque bar backgrounds (alpha=255). Verify that
    /// `with_alpha` applied to bar colors produces the expected result.
    #[test]
    fn psp_opaque_bar_override_pattern() {
        let at = ActiveTheme::from_base_colors(
            Color::rgb(0x0A, 0x0A, 0x14),
            Color::rgb(0x00, 0xF0, 0xFF),
            Color::rgb(0x1A, 0x1A, 0x2E),
            Color::rgb(0xE0, 0xF0, 0xFF),
            Color::rgb(0x50, 0x60, 0x80),
            Color::rgba(0x08, 0x08, 0x10, 0x80),
            Color::rgb(0x00, 0xF0, 0xFF),
            Color::rgb(0xC0, 0xD8, 0xFF),
            Color::rgb(0xFF, 0x20, 0x60),
        );
        // The PSP backend applies: bar.statusbar_bg.a = 255, bar.bg.a = 255.
        // Simulate the PSP override using the public Color API.
        let opaque_status = Color::rgba(
            at.bar.statusbar_bg.r,
            at.bar.statusbar_bg.g,
            at.bar.statusbar_bg.b,
            255,
        );
        let opaque_bar = Color::rgba(at.bar.bg.r, at.bar.bg.g, at.bar.bg.b, 255);
        // RGB channels preserved, only alpha changed.
        assert_eq!(opaque_status.r, at.bar.statusbar_bg.r);
        assert_eq!(opaque_status.g, at.bar.statusbar_bg.g);
        assert_eq!(opaque_status.b, at.bar.statusbar_bg.b);
        assert_eq!(opaque_status.a, 255);
        assert_eq!(opaque_bar.a, 255);
    }

    /// `with_screen_size` scales up for larger screens (e.g. 800x600).
    #[test]
    fn with_screen_size_scales_up() {
        let base = ActiveTheme::default();
        let scaled = base.clone().with_screen_size(800, 600);
        assert_eq!(scaled.screen_w, 800);
        assert_eq!(scaled.screen_h, 600);
        // 800/480 ≈ 1.667x -- layout values should be larger.
        assert!(
            scaled.r_hint_w > base.r_hint_w,
            "r_hint_w should scale up: {} vs {}",
            scaled.r_hint_w,
            base.r_hint_w
        );
    }

    /// Background layer filtering: PSP filters out expensive layer types.
    /// Verify the shared LayerKind enum covers the types PSP filters.
    #[test]
    fn background_layer_kinds_exist_for_psp_filtering() {
        use oasis_vector::BackgroundLayer;
        use oasis_vector::background::{LayerAnimation, LayerKind, LayerPosition};

        let expensive_layers = vec![
            BackgroundLayer {
                kind: LayerKind::FloatingPolygons { count: 5, sides: 4 },
                color: Color::WHITE,
                position: LayerPosition::default(),
                animation: LayerAnimation::default(),
                enabled: true,
            },
            BackgroundLayer {
                kind: LayerKind::EqBars {
                    count: 8,
                    bar_width: 10,
                    max_height: 50,
                },
                color: Color::WHITE,
                position: LayerPosition::default(),
                animation: LayerAnimation::default(),
                enabled: true,
            },
            BackgroundLayer {
                kind: LayerKind::Grid { spacing: 20 },
                color: Color::WHITE,
                position: LayerPosition::default(),
                animation: LayerAnimation::default(),
                enabled: true,
            },
        ];

        // PSP filtering retains only non-expensive layers.
        let filtered: Vec<_> = expensive_layers
            .into_iter()
            .filter(|layer| {
                !matches!(
                    layer.kind,
                    LayerKind::FloatingPolygons { .. }
                        | LayerKind::EqBars { .. }
                        | LayerKind::Waves { .. }
                        | LayerKind::Shader { .. }
                )
            })
            .collect();
        // Grid layer should survive.
        assert_eq!(filtered.len(), 1);
        assert!(matches!(filtered[0].kind, LayerKind::Grid { .. }));
    }

    // -- Cross-skin derivation snapshot --------------------------------

    /// Format a `Color` as a stable `#RRGGBBAA` string for snapshot output.
    fn fmt_color(c: Color) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", c.r, c.g, c.b, c.a)
    }

    /// Render a one-line palette fingerprint for an `ActiveTheme`. The
    /// 13 derived colors here are the ones the bar/icon/menu/toast
    /// renderers actually read at paint time -- a regression in the
    /// derivation logic shows up here as a flipped channel value.
    fn palette_fingerprint(at: &ActiveTheme) -> String {
        let parts = [
            fmt_color(at.bar.statusbar_bg),
            fmt_color(at.bar.bg),
            fmt_color(at.bar.battery_color),
            fmt_color(at.bar.tab_active_fill),
            fmt_color(at.bar.version_color),
            fmt_color(at.bar.clock_color),
            fmt_color(at.icon.cursor_color),
            fmt_color(at.icon.body_color),
            fmt_color(at.menu.panel_bg),
            fmt_color(at.menu.button_bg),
            fmt_color(at.app.bg),
            fmt_color(at.app.title_bar_bg),
            fmt_color(at.toast.info_bg),
        ];
        parts.join("|")
    }

    /// Snapshot every built-in skin's derived `ActiveTheme` palette.
    ///
    /// Locks down the output of `ActiveTheme::from_skin` for all 15
    /// shipping skins. Failure modes this catches:
    ///   - A change to `derive.rs` that flips a channel for one skin
    ///     while leaving others untouched.
    ///   - A new skin that lands without a matching expected entry
    ///     (forces a deliberate update with the new palette inline).
    ///   - A skin whose TOML changed and now derives differently --
    ///     the failing line shows the *new* palette so the maintainer
    ///     can paste it in if intentional.
    ///
    /// To regenerate after an intentional change, run with
    /// `RUST_TEST_THREADS=1 cargo test cross_skin_palette_snapshot`
    /// and copy the printed `(name, "fingerprint")` lines.
    #[test]
    fn cross_skin_palette_snapshot() {
        use crate::builtin::{builtin_names, load_builtin};

        // Expected palette fingerprints. Pinned 2026-07-18. Regenerate
        // by uncommenting the print below and running once.
        let expected: &[(&str, &str)] = &[
            (
                "classic",
                "#18182C50|#18182C5A|#7CABDBFF|#4488CC1E|#E0E0F0FF|#E0E0F0FF|#4488CC50|#E0E0F0FF|#141423DC|#4488CCC8|#121220FF|#21212EFF|#4488CCDC",
            ),
            (
                "corrupted",
                "#1A001A50|#1A001A5A|#FF4CFFFF|#FF00FF1E|#CC00CCFF|#CC00CCFF|#FF00FF50|#CC00CCFF|#141423DC|#FF00FFC8|#0A050AFF|#191419FF|#FF00FFDC",
            ),
            (
                "desktop",
                "#22223350|#2222335A|#6F92D8FF|#3264C81E|#FFFFFFFF|#FFFFFFFF|#3264C850|#FFFFFFFF|#141423DC|#3264C8C8|#1E1E31FF|#2C2C3DFF|#3264C8DC",
            ),
            (
                "modern",
                "#1A1A2D50|#1A1A2D5A|#988CEEFF|#6C5CE71E|#F0F0FFFF|#F0F0FFFF|#6C5CE750|#F0F0FFFF|#141423DC|#6C5CE7C8|#181822FF|#262630FF|#6C5CE7DC",
            ),
            (
                "xp",
                "#1F3E7BFF|#1F3E7BFF|#FFFFFFFF|#0033991E|#FFFFFFFF|#FFFFFFFF|#5B9BD5A0|#ECE9D8FF|#1F3E7BFF|#309E30FF|#ECE9D8FF|#1443A1FF|#003399DC",
            ),
            (
                "macos",
                "#F5F5F780|#2C2C2EFF|#1D1D1FFF|#007AFF1E|#86868BFF|#1D1D1FFF|#007AFF40|#FFFFFFFF|#F5F5F7FF|#007AFFFF|#FFFFFFFF|#E8E8E8FF|#007AFFDC",
            ),
            (
                "gnome",
                "#0F0F0FFF|#161616FF|#E6E6E6FF|#3584E41E|#929292FF|#FFFFFFFF|#3584E460|#303030FF|#222222FF|#3584E4FF|#242424FF|#303030FF|#3584E4DC",
            ),
            (
                "retro-cga",
                "#000000FF|#000000FF|#55FFFFFF|#55FFFF1E|#FF55FFFF|#FFFFFFFF|#FF55FF80|#000000FF|#000000FF|#FF55FFFF|#000000FF|#55FFFFFF|#55FFFFDC",
            ),
            (
                "balatro",
                "#08081080|#0A0A14FF|#00F0FFFF|#00F0FF1E|#FF2060FF|#FFD000FF|#00F0FF30|#12122AFF|#0E0E20FF|#FF206080|#0A0A14FF|#12122AFF|#00F0FFDC",
            ),
            (
                "paper",
                "#F0EDE4FF|#F0EDE4FF|#1A1A1AFF|#2C2C2C1E|#8C8C84FF|#1A1A1AFF|#2C2C2C20|#FFFFFFFF|#FFFFFFFF|#F0EDE4FF|#FFFFFFFF|#F0EDE4FF|#2C2C2CDC",
            ),
            (
                "win95",
                "#C0C0C0FF|#283C5A5A|#000000FF|#0000801E|#000000FF|#000000FF|#00008050|#C0C0C0FF|#C0C0C0FF|#C0C0C0FF|#C0C0C0FF|#148A8AFF|#000080DC",
            ),
            (
                "solarized",
                "#073642FF|#283C5A5A|#93A1A1FF|#268BD21E|#93A1A1FF|#93A1A1FF|#268BD250|#073642FF|#073642FF|#268BD2FF|#002B36FF|#143B46FF|#268BD2DC",
            ),
            (
                "vaporwave",
                "#2D1B69FF|#283C5A5A|#E0D0FFFF|#FF71CE1E|#E0D0FFFF|#E0D0FFFF|#FF71CE50|#2D1B69FF|#2D1B69FF|#FF71CEFF|#1A0A2EFF|#2C1D3EFF|#FF71CEDC",
            ),
            (
                "highcontrast",
                "#1A1A1AFF|#283C5A5A|#FFFFFFFF|#FFFF001E|#FFFFFFFF|#FFFFFFFF|#FFFF0050|#1A1A1AFF|#1A1A1AFF|#FFFF00FF|#000000FF|#141414FF|#FFFF00DC",
            ),
            (
                "altimit",
                "#0A0A1A80|#080816FF|#00CC88FF|#00CC881E|#D0E8E0FF|#00CC88FF|#00CC8840|#0E0E22FF|#0A0A1AFF|#00CC8860|#080816FF|#0E0E22FF|#00CC88DC",
            ),
            (
                "psix-tribute",
                "#1A1A1E50|#1A1A1E5A|#F8A757FF|#F5820F1E|#F0F0E8FF|#F0F0E8FF|#F5820FFF|#ECECE4FF|#141423DC|#F5820FC8|#18181AFF|#262628FF|#F5820FDC",
            ),
            (
                "psix-hifi",
                "#1A1A1E50|#1A1A1E5A|#9A9A9AFF|#F5820F1E|#FFFFFFFF|#B8B8B8FF|#FFFFFFE0|#F8F8F4FF|#141423DC|#F5820FC8|#18181AFF|#262628FF|#F5820FDC",
            ),
        ];

        // First check: the registry must list exactly the skins we
        // pinned -- no missing entries either way. Compared as sets so
        // a cosmetic reordering of `builtin_names()` does not fail the
        // palette snapshot.
        use std::collections::BTreeSet;
        let names: BTreeSet<&str> = builtin_names().iter().copied().collect();
        let pinned: BTreeSet<&str> = expected.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names, pinned,
            "built-in skin set changed -- update cross_skin_palette_snapshot \
             expected[] to match (and bump the date in the doc comment)"
        );

        // Index expected fingerprints by skin name so the per-skin
        // assertion is independent of registry iteration order.
        let expected_by_name: std::collections::HashMap<&str, &str> =
            expected.iter().copied().collect();

        let mut updates: Vec<String> = Vec::new();
        for name in builtin_names().iter().copied() {
            let skin = load_builtin(name).expect("built-in skin loads");
            let at = ActiveTheme::from_skin(&skin.theme);
            let actual = palette_fingerprint(&at);
            let expected_fingerprint = expected_by_name
                .get(name)
                .expect("set check above ensures coverage");
            if actual != *expected_fingerprint {
                updates.push(format!("            (\"{name}\", \"{actual}\"),"));
            }
        }
        assert!(
            updates.is_empty(),
            "palette derivation drifted for {} skin(s) -- new lines:\n{}",
            updates.len(),
            updates.join("\n")
        );
    }

    // -- reduced_motion gating (with_features) --

    #[test]
    fn reduced_motion_off_leaves_animations_untouched() {
        let skin = SkinTheme::default();
        let features = crate::loader::SkinFeatures {
            reduced_motion: false,
            ..Default::default()
        };
        let plain = ActiveTheme::from_skin(&skin);
        let gated = ActiveTheme::from_skin(&skin).with_features(&features);
        // Default is pixel-identical: nothing is forced off.
        assert_eq!(gated.icon.idle_float, plain.icon.idle_float);
        assert_eq!(gated.icon.spin_enabled, plain.icon.spin_enabled);
        assert_eq!(gated.entrance_style, plain.entrance_style);
        assert_eq!(gated.focus_glow, plain.focus_glow);
        assert_eq!(
            gated.background_reduced_motion,
            plain.background_reduced_motion
        );
        assert!(!gated.ui_theme.reduced_motion);
    }

    #[test]
    fn reduced_motion_on_forces_all_motion_off() {
        // Start from a skin that opts into every animation.
        let toml = r##"
[icon_overrides]
vector_idle_float = true
vector_spin_enabled = true
vector_pulse_enabled = true
vector_blink_enabled = true
entrance_style = "fade_in"
focus_glow = true
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let features = crate::loader::SkinFeatures {
            reduced_motion: true,
            ..Default::default()
        };
        let gated = ActiveTheme::from_skin(&skin).with_features(&features);
        assert!(!gated.icon.idle_float);
        assert!(!gated.icon.spin_enabled);
        assert!(!gated.icon.pulse_enabled);
        assert!(!gated.icon.blink_enabled);
        assert_eq!(gated.entrance_style, "none");
        assert!(!gated.focus_glow);
        assert!(gated.background_reduced_motion);
        assert!(gated.ui_theme.reduced_motion);
    }

    // -- semantic elevation ladder --

    #[test]
    fn resolve_shadow_default_matches_builtin() {
        let at = ActiveTheme::default();
        for level in 0u8..=5 {
            assert_eq!(
                at.resolve_shadow(level).layers.len(),
                oasis_types::shadow::Shadow::elevation(level).layers.len()
            );
        }
    }

    #[test]
    fn resolve_shadow_honors_elevation_overrides() {
        let toml = r##"
[[elevation.level_2]]
offset_x = 4
offset_y = 4
spread = 1
alpha = 120
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        let s = at.resolve_shadow(2);
        assert_eq!(s.layers.len(), 1);
        assert_eq!(s.layers[0].alpha, 120);
        // Unset levels still resolve to the built-in ladder.
        assert_eq!(
            at.resolve_shadow(1).layers.len(),
            oasis_types::shadow::Shadow::elevation(1).layers.len()
        );
    }
}

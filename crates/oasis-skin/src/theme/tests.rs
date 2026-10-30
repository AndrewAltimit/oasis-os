//! Tests for `SkinTheme` parsing, base colors and derived `ui::Theme`.

use super::*;

#[test]
fn default_theme_parses() {
    let theme = SkinTheme::default();
    assert_ne!(theme.background_color(), Color::WHITE);
    assert_eq!(theme.prompt_color(), Color::rgb(0, 255, 0));
}

#[test]
fn parse_hex_colors() {
    assert_eq!(parse_hex_color("#FF0000"), Some(Color::rgb(255, 0, 0)));
    assert_eq!(
        parse_hex_color("#00FF0080"),
        Some(Color::rgba(0, 255, 0, 128))
    );
    assert_eq!(parse_hex_color("invalid"), None);
    assert_eq!(parse_hex_color("#GG0000"), None);
}

#[test]
fn deserialize_from_toml() {
    let toml = r##"
background = "#000000"
primary = "#00FF00"
text = "#00FF00"
prompt = "#00FF00"
output = "#00CC00"
error = "#FF0000"
"##;
    let theme: SkinTheme = toml::from_str(toml).unwrap();
    assert_eq!(theme.background_color(), Color::rgb(0, 0, 0));
    assert_eq!(theme.text_color(), Color::rgb(0, 255, 0));
}

#[test]
fn wm_theme_overrides() {
    let toml = r##"
[wm_theme]
titlebar_height = 32
titlebar_active = "#0000FF"
button_size = 20
"##;
    let theme: SkinTheme = toml::from_str(toml).unwrap();
    let wm = theme.build_wm_theme();
    assert_eq!(wm.titlebar_height, 32);
    assert_eq!(wm.titlebar_active_color, Color::rgb(0, 0, 255));
    assert_eq!(wm.button_size, 20);
    // Non-overridden values remain default.
    assert_eq!(wm.border_width, 1);
}

#[test]
fn no_wm_overrides_returns_default() {
    let theme = SkinTheme::default();
    let wm = theme.build_wm_theme();
    assert_eq!(wm.titlebar_height, 24);
}

#[test]
fn wm_theme_nine_patch_overrides() {
    let toml = r##"
[wm_theme]
titlebar_nine_patch = { image = "assets/tb.png", insets = [4, 4, 4, 4] }
frame_nine_patch = { image = "assets/frame.png", insets = [6, 6, 6, 6] }
"##;
    let theme: SkinTheme = toml::from_str(toml).unwrap();
    let wm = theme.build_wm_theme();
    assert_eq!(
        wm.titlebar_nine_patch,
        Some(("assets/tb.png".to_string(), [4, 4, 4, 4]))
    );
    assert_eq!(
        wm.frame_nine_patch,
        Some(("assets/frame.png".to_string(), [6, 6, 6, 6]))
    );
    // Runtime patches are resolved by the shell, never at parse time.
    assert!(wm.titlebar_patch.is_none());
    assert!(wm.frame_patch.is_none());
}

#[test]
fn to_ui_theme_derives_from_base_colors() {
    let skin = SkinTheme::default();
    let ui = skin.to_ui_theme();
    // Background should match.
    assert_eq!(ui.background, skin.background_color());
    // Accent should match primary.
    assert_eq!(ui.accent, skin.primary_color());
    // Error should match.
    assert_eq!(ui.error, skin.error_color());
    // Text primary should match text.
    assert_eq!(ui.text_primary, skin.text_color());
    // Border radii should be reasonable.
    assert!(ui.border_radius_md > 0);
}

#[test]
fn to_ui_theme_respects_extended_fields() {
    let toml = r##"
background = "#000000"
primary = "#FF0000"
surface = "#111111"
accent_hover = "#FF5555"
border_radius = 8
shadow_intensity = 2
gradient_enabled = true
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let ui = skin.to_ui_theme();
    assert_eq!(ui.surface, Color::rgb(0x11, 0x11, 0x11));
    assert_eq!(ui.accent_hover, Color::rgb(0xFF, 0x55, 0x55));
    assert_eq!(ui.border_radius_md, 8);
}

#[test]
fn to_ui_theme_roundtrip() {
    let skin = SkinTheme::default();
    let ui = skin.to_ui_theme();
    assert_eq!(ui.accent, skin.primary_color());
}

#[test]
fn to_ui_theme_accent_override() {
    let toml = r##"
primary = "#FF71CE"
accent = "#01CDFE"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let ui = skin.to_ui_theme();
    // Accent family derives from the explicit accent, not primary.
    assert_eq!(ui.accent, Color::rgb(0x01, 0xCD, 0xFE));
    assert_ne!(ui.accent, skin.primary_color());
    assert_eq!(ui.accent_subtle.r, 0x01);
    assert_eq!(ui.info, ui.accent);
}

#[test]
fn build_wm_theme_titlebar_text_active_inactive() {
    let toml = r##"
[wm_theme]
titlebar_text_active = "#FFFFFF"
titlebar_text_inactive = "#888888"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let wm = skin.build_wm_theme();
    assert_eq!(wm.titlebar_text_color, Color::rgb(0xFF, 0xFF, 0xFF));
    assert_eq!(
        wm.titlebar_text_inactive_color,
        Color::rgb(0x88, 0x88, 0x88)
    );
}

#[test]
fn build_wm_theme_inactive_text_defaults_to_active() {
    let toml = r##"
[wm_theme]
titlebar_text = "#FF00FF"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let wm = skin.build_wm_theme();
    assert_eq!(wm.titlebar_text_color, Color::rgb(0xFF, 0x00, 0xFF));
    assert_eq!(wm.titlebar_text_inactive_color, wm.titlebar_text_color);
}

// -- resolve_easing tests --

#[test]
fn resolve_easing_known_names() {
    let linear = resolve_easing("linear");
    assert!((linear(0.5) - 0.5).abs() < f32::EPSILON);

    let ease_out = resolve_easing("ease_out_quad");
    assert!(ease_out(0.5) > 0.5);

    let ease_in = resolve_easing("ease_in_quad");
    assert!(ease_in(0.5) < 0.5);
}

#[test]
fn resolve_easing_unknown_returns_linear() {
    let f = resolve_easing("unknown_easing");
    assert!((f(0.5) - 0.5).abs() < f32::EPSILON);
}

// -- GradientPreset / AnimationPreset deserialization tests --

#[test]
fn gradient_preset_deserialize() {
    let toml = r##"
[gradients.primary]
from = "#0066FF"
to = "#0044AA"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let grads = skin.gradients.unwrap();
    let g = &grads["primary"];
    assert_eq!(g.from, "#0066FF");
    assert_eq!(g.to, "#0044AA");
}

#[test]
fn animation_preset_deserialize() {
    let toml = r##"
[animations.button_press]
duration_ms = 100
easing = "ease_out_quad"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let anims = skin.animations.unwrap();
    let a = &anims["button_press"];
    assert_eq!(a.duration_ms, 100);
    assert_eq!(a.easing, "ease_out_quad");
}

#[test]
fn animation_preset_default_easing() {
    let toml = r##"
[animations.fast]
duration_ms = 50
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let anims = skin.animations.unwrap();
    assert_eq!(anims["fast"].easing, "linear");
}

// -- widget_states deserialization tests --

#[test]
fn widget_states_deserialize() {
    let toml = r##"
[widget_states.button]
normal_bg = "#505050"
hover_bg = "#656565"
pressed_bg = "#353535"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let states = skin.widget_states.unwrap();
    let button = &states["button"];
    assert_eq!(button["hover_bg"], "#656565");
}

#[test]
fn widget_states_override_ui_theme() {
    let toml = r##"
[widget_states.button]
normal_bg = "#505050"
hover_bg = "#656565"

[widget_states.toggle]
track_on = "#FF8C1E"
thumb = "#101010"

[widget_states.input]
focus_border = "#00FFAA"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let theme = skin.to_ui_theme();
    assert_eq!(theme.button_bg, Color::rgb(0x50, 0x50, 0x50));
    assert_eq!(theme.button_bg_hover, Color::rgb(0x65, 0x65, 0x65));
    assert_eq!(theme.toggle_track_on, Color::rgb(0xFF, 0x8C, 0x1E));
    assert_eq!(theme.toggle_thumb, Color::rgb(0x10, 0x10, 0x10));
    assert_eq!(theme.input_border_focus, Color::rgb(0x00, 0xFF, 0xAA));
    // Slots not overridden keep their derived values.
    let plain = SkinTheme::default().to_ui_theme();
    assert_eq!(theme.button_bg_pressed, plain.button_bg_pressed);
    assert_eq!(theme.toggle_track_off, plain.toggle_track_off);
}

#[test]
fn typography_overrides_ui_theme_scale() {
    let toml = r##"
[typography]
font_size_md = 10
font_size_lg = 18
spacing_md = 10
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let theme = skin.to_ui_theme();
    let plain = SkinTheme::default().to_ui_theme();

    assert_eq!(theme.font_size_md, 10);
    assert_eq!(theme.font_size_lg, 18);
    assert_eq!(theme.spacing_md, 10);
    // Unset steps keep the historical ladder.
    assert_eq!(theme.font_size_xs, plain.font_size_xs);
    assert_eq!(theme.font_size_xxl, plain.font_size_xxl);
    assert_eq!(theme.spacing_xl, plain.spacing_xl);
}

#[test]
fn no_typography_table_is_identical_to_before() {
    // Every shipped skin predates `[typography]`: the derived scale must be
    // exactly the values that were hardcoded in the derivation.
    let theme = SkinTheme::default().to_ui_theme();
    assert_eq!(
        (
            theme.font_size_xs,
            theme.font_size_sm,
            theme.font_size_md,
            theme.font_size_lg,
            theme.font_size_xl,
            theme.font_size_xxl
        ),
        (8, 8, 8, 16, 16, 24)
    );
    assert_eq!(
        (
            theme.spacing_xs,
            theme.spacing_sm,
            theme.spacing_md,
            theme.spacing_lg,
            theme.spacing_xl
        ),
        (2, 4, 8, 12, 16)
    );
}

#[test]
fn ui_theme_toggle_defaults_derive_from_accent() {
    let skin = SkinTheme::default();
    let theme = skin.to_ui_theme();
    // Without widget_states, the toggle derives from the accent family
    // exactly as the widget hardcoded before.
    assert_eq!(theme.toggle_track_off, Color::rgba(255, 255, 255, 10));
    assert_eq!(theme.toggle_track_on, theme.accent);
    assert_eq!(theme.toggle_thumb, theme.text_primary);
}

#[test]
fn ui_theme_slider_defaults_derive_from_legacy_slots() {
    // Without widget_states, the slider slots equal the fields the
    // widget used to borrow (input_bg / accent / surface).
    let theme = SkinTheme::default().to_ui_theme();
    assert_eq!(theme.slider_track, theme.input_bg);
    assert_eq!(theme.slider_fill, theme.accent);
    assert_eq!(theme.slider_thumb, theme.surface);
}

#[test]
fn ui_theme_menu_defaults_are_win95_grays() {
    // Without widget_states, the menu bar keeps the Win95 grays that
    // `MenuStyle::default()` always hardcoded.
    let theme = SkinTheme::default().to_ui_theme();
    assert_eq!(theme.menu_bg, Color::rgb(240, 240, 240));
    assert_eq!(theme.menu_border, Color::rgb(180, 180, 180));
    assert_eq!(theme.menu_text, Color::rgb(30, 30, 30));
    assert_eq!(theme.menu_hover_bg, Color::rgb(49, 106, 197));
    assert_eq!(theme.menu_hover_text, Color::rgb(255, 255, 255));
    assert_eq!(theme.menu_dropdown_bg, Color::rgb(236, 236, 236));
    assert_eq!(theme.menu_dropdown_border_light, Color::rgb(255, 255, 255));
    assert_eq!(theme.menu_dropdown_border_dark, Color::rgb(105, 105, 105));
    assert_eq!(theme.menu_item_text, Color::rgb(20, 20, 20));
    assert_eq!(theme.menu_disabled_text, Color::rgb(150, 150, 150));
    assert_eq!(theme.menu_separator, Color::rgb(170, 170, 170));
}

#[test]
fn widget_states_slider_overrides_ui_theme() {
    let toml = r##"
[widget_states.slider]
track = "#101018"
fill = "#FF8C1E"
thumb = "#F0F0E8"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let theme = skin.to_ui_theme();
    assert_eq!(theme.slider_track, Color::rgb(0x10, 0x10, 0x18));
    assert_eq!(theme.slider_fill, Color::rgb(0xFF, 0x8C, 0x1E));
    assert_eq!(theme.slider_thumb, Color::rgb(0xF0, 0xF0, 0xE8));
    // Unrelated slots keep their derived values.
    let plain = SkinTheme::default().to_ui_theme();
    assert_eq!(theme.accent, plain.accent);
    assert_eq!(theme.input_bg, plain.input_bg);
}

#[test]
fn widget_states_menu_overrides_ui_theme() {
    let toml = r##"
[widget_states.menu]
bg = "#2A2A30"
text = "#E0E0E8"
hover_bg = "#F5820F"
hover_text = "#101010"
dropdown_bg = "#33333A"
separator = "#55555C"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let theme = skin.to_ui_theme();
    assert_eq!(theme.menu_bg, Color::rgb(0x2A, 0x2A, 0x30));
    assert_eq!(theme.menu_text, Color::rgb(0xE0, 0xE0, 0xE8));
    assert_eq!(theme.menu_hover_bg, Color::rgb(0xF5, 0x82, 0x0F));
    assert_eq!(theme.menu_hover_text, Color::rgb(0x10, 0x10, 0x10));
    assert_eq!(theme.menu_dropdown_bg, Color::rgb(0x33, 0x33, 0x3A));
    assert_eq!(theme.menu_separator, Color::rgb(0x55, 0x55, 0x5C));
    // Slots not overridden keep the Win95 defaults.
    assert_eq!(theme.menu_border, Color::rgb(180, 180, 180));
    assert_eq!(theme.menu_item_text, Color::rgb(20, 20, 20));
}

#[test]
fn success_warning_override_toast_and_ui() {
    let toml = r##"
success = "#00CC66"
warning = "#FFAA00"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let theme = skin.to_ui_theme();
    assert_eq!(theme.success, Color::rgb(0x00, 0xCC, 0x66));
    assert_eq!(theme.warning, Color::rgb(0xFF, 0xAA, 0x00));
    // Defaults are the historical literals.
    let plain = SkinTheme::default().to_ui_theme();
    assert_eq!(plain.success, Color::rgb(80, 200, 120));
    assert_eq!(plain.warning, Color::rgb(255, 180, 50));
}

// -- Phase 12: expanded SkinTheme tests --

#[test]
fn default_theme_background_is_dark() {
    let skin = SkinTheme::default();
    let bg = skin.background_color();
    assert!(bg.r < 50 && bg.g < 50 && bg.b < 50);
}

#[test]
fn default_theme_text_is_green() {
    let skin = SkinTheme::default();
    let text = skin.text_color();
    assert_eq!(text.g, 255);
}

#[test]
fn parse_hex_color_three_byte() {
    assert_eq!(
        parse_hex_color("#AABBCC"),
        Some(Color::rgb(0xAA, 0xBB, 0xCC))
    );
}

#[test]
fn parse_hex_color_four_byte() {
    assert_eq!(
        parse_hex_color("#AABBCC80"),
        Some(Color::rgba(0xAA, 0xBB, 0xCC, 0x80))
    );
}

#[test]
fn parse_hex_color_lowercase() {
    assert_eq!(
        parse_hex_color("#aabbcc"),
        Some(Color::rgb(0xAA, 0xBB, 0xCC))
    );
}

#[test]
fn parse_hex_color_missing_hash() {
    assert_eq!(parse_hex_color("AABBCC"), None);
}

#[test]
fn to_ui_theme_surface_derived_from_background() {
    let skin = SkinTheme::default();
    let ui = skin.to_ui_theme();
    // Surface should be lighter than background (derived via lighten).
    let bg = skin.background_color();
    assert!(ui.surface.r >= bg.r || ui.surface.g >= bg.g || ui.surface.b >= bg.b);
}

#[test]
fn to_ui_theme_border_derived() {
    let skin = SkinTheme::default();
    let ui = skin.to_ui_theme();
    // Border strong should match primary.
    assert_eq!(ui.border_strong, skin.primary_color());
}

#[test]
fn to_ui_theme_font_sizes_are_reasonable() {
    let skin = SkinTheme::default();
    let ui = skin.to_ui_theme();
    assert!(ui.font_size_sm > 0);
    assert!(ui.font_size_md > 0);
    assert!(ui.font_size_lg > ui.font_size_sm);
}

#[test]
fn contrast_ratio_black_white() {
    let ratio = contrast_ratio(Color::BLACK, Color::WHITE);
    assert!((ratio - 21.0).abs() < 0.1);
}

#[test]
fn contrast_ratio_same_color() {
    let ratio = contrast_ratio(Color::rgb(128, 128, 128), Color::rgb(128, 128, 128));
    assert!((ratio - 1.0).abs() < 0.01);
}

#[test]
fn default_theme_passes_contrast() {
    let skin = SkinTheme::default();
    let warnings = skin.validate_contrast();
    // Default theme (light text on dark bg) should pass.
    assert!(
        warnings.is_empty(),
        "default theme has contrast warnings: {warnings:?}"
    );
}

#[test]
fn low_contrast_theme_warns() {
    let toml = r##"
background = "#808080"
text = "#909090"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let warnings = skin.validate_contrast();
    assert!(!warnings.is_empty(), "should warn on low contrast");
    assert!(warnings.iter().any(|w| w.pair.contains("text")));
}

#[test]
fn contrast_ratio_known_gray_pair() {
    // #767676 on white is the canonical ~4.54:1 AA boundary pair.
    let ratio = contrast_ratio(Color::rgb(0x76, 0x76, 0x76), Color::WHITE);
    assert!((ratio - 4.54).abs() < 0.02, "got {ratio}");
}

#[test]
fn composite_over_opaque_passthrough() {
    let fg = Color::rgb(10, 20, 30);
    let out = composite_over(fg, Color::WHITE);
    assert_eq!((out.r, out.g, out.b), (10, 20, 30));
    assert_eq!(out.a, 255);
}

#[test]
fn composite_over_blends_alpha() {
    // 50% black over white -> mid gray.
    let out = composite_over(Color::rgba(0, 0, 0, 128), Color::WHITE);
    assert!((out.r as i32 - 127).abs() <= 1, "got {}", out.r);
    // Fully transparent -> backdrop.
    let out = composite_over(Color::rgba(255, 0, 0, 0), Color::rgb(1, 2, 3));
    assert_eq!((out.r, out.g, out.b), (1, 2, 3));
}

#[test]
fn derived_button_text_reads_on_the_button_fill() {
    // The base text color nearly matches the button fill (derived from
    // `secondary`): the derived `button_text` is pushed until it reaches
    // 4.5:1, so there is no button warning.
    let toml = r##"
background = "#101010"
text = "#5A5A5A"
secondary = "#4A4A4A"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let ui = skin.to_ui_theme();
    assert_eq!(ui.text_primary, Color::rgb(0x5A, 0x5A, 0x5A));
    assert!(contrast_ratio(ui.button_text, ui.button_bg) >= 4.5);
    let warnings = skin.validate_contrast();
    assert!(
        !warnings.iter().any(|w| w.pair.contains("button")),
        "{warnings:?}"
    );
    // A readable base text color is kept as is.
    let ok: SkinTheme = toml::from_str("text = \"#F0F0F0\"\nsecondary = \"#202020\"").unwrap();
    assert_eq!(ok.to_ui_theme().button_text, Color::rgb(0xF0, 0xF0, 0xF0));
}

#[test]
fn explicit_button_text_is_used_as_given_and_linted() {
    // `[widget_states.button] text` wins over the derivation, even when
    // it is unreadable; the contrast report then flags it.
    let toml = r##"
background = "#101010"
text = "#F0F0F0"
secondary = "#4A4A4A"

[widget_states.button]
text = "#505050"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    assert_eq!(skin.to_ui_theme().button_text, Color::rgb(0x50, 0x50, 0x50));
    let warnings = skin.validate_contrast();
    assert!(
        warnings
            .iter()
            .any(|w| w.pair.contains("button") && w.required == 4.5),
        "missing button contrast warning: {warnings:?}"
    );
    // The derivation follows an overridden fill too.
    let light: SkinTheme =
        toml::from_str("text = \"#F0F0F0\"\n[widget_states.button]\nnormal_bg = \"#F8F8F8\"")
            .unwrap();
    let ui = light.to_ui_theme();
    assert!(contrast_ratio(ui.button_text, ui.button_bg) >= 4.5);
}

#[test]
fn contrast_warns_on_low_bar_pair() {
    // Clock text matches the status bar backdrop -> unreadable.
    let toml = r##"
background = "#202020"
status_bar = "#303030"

[bar_overrides]
statusbar_bg = "#303030"
clock_color = "#3A3A3A"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let warnings = skin.validate_contrast();
    assert!(
        warnings
            .iter()
            .any(|w| w.pair.contains("status bar") && w.required == 3.0),
        "missing status bar contrast warning: {warnings:?}"
    );
}

#[test]
fn contrast_ratio_included_in_warning() {
    let toml = r##"
background = "#808080"
text = "#909090"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let warnings = skin.validate_contrast();
    let text_warning = warnings
        .iter()
        .find(|w| w.pair == "text on background")
        .expect("text pair should warn");
    assert!(text_warning.ratio >= 1.0 && text_warning.ratio < 4.5);
    assert_eq!(text_warning.required, 4.5);
}

#[test]
fn bar_overrides_deserialize() {
    let toml = r##"
[bar_overrides]
statusbar_bg = "#112233"
clock_color = "#FFFFFF"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let bars = skin.bar_overrides.unwrap();
    assert_eq!(bars.statusbar_bg.as_deref(), Some("#112233"));
    assert_eq!(bars.clock_color.as_deref(), Some("#FFFFFF"));
}

#[test]
fn icon_overrides_deserialize() {
    let toml = r##"
[icon_overrides]
body_color = "#FF0000"
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let icons = skin.icon_overrides.unwrap();
    assert_eq!(icons.body_color.as_deref(), Some("#FF0000"));
}

// -- text_scale (accessibility) tests --

#[test]
fn default_text_scale_is_one() {
    let skin = SkinTheme::default();
    assert_eq!(skin.text_scale, 1.0);
    assert_eq!(skin.text_scale_factor(), 1.0);
}

#[test]
fn text_scale_one_is_pixel_identical() {
    let skin = SkinTheme::default();
    assert_eq!(skin.scale_font(8), 8);
    assert_eq!(skin.scale_font(16), 16);
    assert_eq!(skin.scale_font(24), 24);
    let ui = skin.to_ui_theme();
    assert_eq!(ui.font_size_md, 8);
    assert_eq!(ui.font_size_lg, 16);
    assert_eq!(ui.font_size_xxl, 24);
}

#[test]
fn text_scale_multiplies_font_ladder() {
    let skin: SkinTheme = toml::from_str("text_scale = 2.0\n").unwrap();
    assert_eq!(skin.scale_font(8), 16);
    assert_eq!(skin.scale_font(16), 32);
    let ui = skin.to_ui_theme();
    assert_eq!(ui.font_size_md, 16);
    assert_eq!(ui.font_size_lg, 32);
    assert_eq!(ui.font_size_xxl, 48);
    // Spacing is untouched by text_scale.
    let plain = SkinTheme::default().to_ui_theme();
    assert_eq!(ui.spacing_md, plain.spacing_md);
}

#[test]
fn text_scale_composes_with_typography() {
    // text_scale multiplies the resolved (typography-overridden) ladder.
    let toml = "text_scale = 2.0\n[typography]\nfont_size_md = 10\n";
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    assert_eq!(skin.to_ui_theme().font_size_md, 20);
}

#[test]
fn text_scale_clamped_and_min_one_px() {
    let skin: SkinTheme = toml::from_str("text_scale = 0.01\n").unwrap();
    assert_eq!(skin.text_scale_factor(), 0.5);
    assert!(skin.scale_font(1) >= 1);
    let big: SkinTheme = toml::from_str("text_scale = 99.0\n").unwrap();
    assert_eq!(big.text_scale_factor(), 3.0);
}

// -- density tests --

#[test]
fn density_default_is_comfortable_identity() {
    let skin = SkinTheme::default();
    assert_eq!(skin.density_preset(), overrides::Density::Comfortable);
    let ui = skin.to_ui_theme();
    // Exact legacy spacing ladder preserved.
    assert_eq!(
        (
            ui.spacing_xs,
            ui.spacing_sm,
            ui.spacing_md,
            ui.spacing_lg,
            ui.spacing_xl
        ),
        (2, 4, 8, 12, 16)
    );
}

#[test]
fn density_compact_shrinks_spacing_only() {
    let skin: SkinTheme = toml::from_str("density = \"compact\"\n").unwrap();
    let ui = skin.to_ui_theme();
    let plain = SkinTheme::default().to_ui_theme();
    // Spacing shrinks (0.85x rounded).
    assert_eq!(ui.spacing_lg, 10); // 12 * 0.85 = 10.2 -> 10
    assert_eq!(ui.spacing_xl, 14); // 16 * 0.85 = 13.6 -> 14
    assert!(ui.spacing_xl < plain.spacing_xl);
    // Font sizes are unaffected by density.
    assert_eq!(ui.font_size_md, plain.font_size_md);
    assert_eq!(ui.font_size_xxl, plain.font_size_xxl);
}

#[test]
fn density_spacious_grows_spacing() {
    let skin: SkinTheme = toml::from_str("density = \"spacious\"\n").unwrap();
    let ui = skin.to_ui_theme();
    assert_eq!(ui.spacing_lg, 14); // 12 * 1.15 = 13.8 -> 14
    assert_eq!(ui.spacing_xl, 18); // 16 * 1.15 = 18.4 -> 18
}

#[test]
fn density_unknown_falls_back_to_comfortable() {
    let skin: SkinTheme = toml::from_str("density = \"cozy\"\n").unwrap();
    assert_eq!(skin.density_preset(), overrides::Density::Comfortable);
    assert_eq!(skin.to_ui_theme().spacing_xl, 16);
}

// -- elevation ladder tests --

#[test]
fn default_elevation_ladder_is_builtin() {
    let skin = SkinTheme::default();
    assert!(skin.elevation_ladder().is_default());
}

#[test]
fn elevation_table_builds_ladder() {
    let toml = r##"
[[elevation.level_1]]
offset_x = 5
offset_y = 5
alpha = 90
"##;
    let skin: SkinTheme = toml::from_str(toml).unwrap();
    let ladder = skin.elevation_ladder();
    assert!(!ladder.is_default());
    assert_eq!(ladder.resolve(1).layers[0].alpha, 90);
}

#[test]
fn default_no_overrides() {
    let skin = SkinTheme::default();
    assert!(skin.surface.is_none());
    assert!(skin.accent_hover.is_none());
    assert!(skin.border_radius.is_none());
    assert!(skin.shadow_intensity.is_none());
    assert!(skin.wm_theme.is_none());
    assert!(skin.bar_overrides.is_none());
    assert!(skin.icon_overrides.is_none());
}

//! Tests for skin loading, layout application, swapping and inheritance.

use super::*;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

const MANIFEST: &str = r#"
name = "classic"
version = "1.0"
author = "AndrewAltimit"
description = "PSP-style icon grid dashboard"
screen_width = 480
screen_height = 272
"#;

const LAYOUT: &str = r##"
[status_bar]
x = 0
y = 0
w = 480
h = 24
color = "#283C5A"

[content_bg]
x = 0
y = 24
w = 480
h = 248
color = "#1A1A2D"
"##;

const FEATURES: &str = r#"
dashboard = true
terminal = true
file_browser = true
browser = true
window_manager = false
dashboard_pages = 3
icons_per_page = 6
grid_cols = 3
grid_rows = 2
"#;

#[test]
fn load_skin_from_toml() {
    let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    assert_eq!(skin.manifest.name, "classic");
    assert_eq!(skin.manifest.screen_width, 480);
    assert_eq!(skin.layout.objects.len(), 2);
    assert!(skin.features.dashboard);
    assert!(skin.features.browser);
    assert!(!skin.features.window_manager);
    assert_eq!(skin.features.grid_cols, 3);
}

#[test]
fn apply_layout_creates_objects() {
    let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    assert!(sdi.contains("status_bar"));
    assert!(sdi.contains("content_bg"));
    let bar = sdi.get("status_bar").unwrap();
    assert_eq!(bar.w, 480);
    assert_eq!(bar.h, 24);
}

#[test]
fn apply_layout_updates_existing() {
    let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    {
        let obj = sdi.create("status_bar");
        obj.x = 999;
    }
    skin.apply_layout(&mut sdi);
    let bar = sdi.get("status_bar").unwrap();
    assert_eq!(bar.x, 0); // Updated by layout.
}

#[test]
fn default_features() {
    let f = SkinFeatures::default();
    assert!(f.dashboard);
    assert!(f.terminal);
    assert_eq!(f.dashboard_pages, 1);
    assert_eq!(f.icons_per_page, 15);
    assert_eq!(f.grid_cols, 5);
    assert_eq!(f.grid_rows, 3);
    assert!(f.clock_in_bottombar);
}

#[test]
fn manifest_defaults() {
    let toml = r#"name = "minimal""#;
    let m: SkinManifest = toml::from_str(toml).unwrap();
    assert_eq!(m.screen_width, 480);
    assert_eq!(m.screen_height, 272);
    assert_eq!(m.version, "1.0");
}

#[test]
fn from_toml_full_with_theme_and_strings() {
    let theme_toml = r##"
background = "#000000"
prompt = "#00FF00"
"##;
    let strings_toml = r#"
prompt_format = "hack> "
title = "HACKER TERM"
boot_text = ["Initializing..."]
"#;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, strings_toml).unwrap();
    assert_eq!(skin.strings.prompt_format, "hack> ");
    assert_eq!(skin.strings.title, "HACKER TERM");
    assert_eq!(
        skin.theme.background_color(),
        oasis_types::backend::Color::rgb(0, 0, 0)
    );
    assert!(skin.corrupted_modifiers.is_none());
}

#[test]
fn corrupted_feature_creates_modifiers() {
    let features = r#"
terminal = true
corrupted = true
"#;
    let skin = Skin::from_toml(MANIFEST, LAYOUT, features).unwrap();
    assert!(skin.corrupted_modifiers.is_some());
}

#[test]
fn swap_skin_replaces_sdi_objects() {
    let skin_a = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    skin_a.apply_layout(&mut sdi);
    assert!(sdi.contains("status_bar"));
    assert!(sdi.contains("content_bg"));

    let layout_b = r##"
[terminal_bg]
x = 0
y = 0
w = 480
h = 272
color = "#000000"
"##;
    let skin_b = Skin::from_toml(MANIFEST, layout_b, FEATURES).unwrap();
    let _new = Skin::swap(&skin_a, skin_b, &mut sdi);

    // Old objects removed.
    assert!(!sdi.contains("status_bar"));
    assert!(!sdi.contains("content_bg"));
    // New objects created.
    assert!(sdi.contains("terminal_bg"));
}

#[test]
fn swap_preserves_non_layout_objects() {
    let skin_a = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    skin_a.apply_layout(&mut sdi);
    // Create an SDI object NOT defined in the layout (e.g., from WM).
    sdi.create("wm_object");

    let layout_b = r##"
[terminal_bg]
x = 0
y = 0
w = 480
h = 272
color = "#000000"
"##;
    let skin_b = Skin::from_toml(MANIFEST, layout_b, FEATURES).unwrap();
    let _new = Skin::swap(&skin_a, skin_b, &mut sdi);

    // WM object survives.
    assert!(sdi.contains("wm_object"));
}

#[test]
fn from_toml_corrupted_custom_modifiers() {
    let corrupted_toml = r#"
position_jitter = 5
text_garble_chance = 0.3
intensity = 0.5
"#;
    let skin =
        Skin::from_toml_corrupted(MANIFEST, LAYOUT, FEATURES, "", "", corrupted_toml).unwrap();
    let mods = skin.corrupted_modifiers.unwrap();
    assert_eq!(mods.position_jitter, 5);
    assert!((mods.intensity - 0.5).abs() < f32::EPSILON);
}

// -- Malformed TOML tests --

#[test]
fn malformed_manifest_toml() {
    let bad = "this is [[[not valid";
    let result = Skin::from_toml(bad, LAYOUT, FEATURES);
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("skin.toml"));
}

#[test]
fn malformed_layout_toml() {
    let bad = "[unclosed";
    let result = Skin::from_toml(MANIFEST, bad, FEATURES);
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("layout.toml"));
}

#[test]
fn malformed_features_toml() {
    let bad = "dashboard = not_a_bool";
    let result = Skin::from_toml(MANIFEST, LAYOUT, bad);
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("features.toml"));
}

#[test]
fn malformed_theme_toml() {
    let bad = "color = [invalid";
    let result = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, bad, "");
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("theme.toml"));
}

#[test]
fn malformed_strings_toml() {
    let bad = "prompt_format = [oops";
    let result = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, "", bad);
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("strings.toml"));
}

#[test]
fn malformed_corrupted_toml() {
    let bad = "position_jitter = \"not a number\"";
    let result = Skin::from_toml_corrupted(MANIFEST, LAYOUT, FEATURES, "", "", bad);
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("corrupted.toml"));
}

// -- Partial / minimal TOML tests --

#[test]
fn minimal_manifest_only_name() {
    let toml = r#"name = "bare""#;
    let m: SkinManifest = toml::from_str(toml).unwrap();
    assert_eq!(m.name, "bare");
    assert_eq!(m.version, "1.0");
    assert_eq!(m.author, "");
    assert_eq!(m.description, "");
    assert_eq!(m.screen_width, 480);
    assert_eq!(m.screen_height, 272);
}

#[test]
fn empty_layout_produces_no_objects() {
    let skin = Skin::from_toml(MANIFEST, "", FEATURES).unwrap();
    assert!(skin.layout.objects.is_empty());
}

#[test]
fn empty_features_uses_defaults() {
    let skin = Skin::from_toml(MANIFEST, LAYOUT, "").unwrap();
    assert!(skin.features.dashboard);
    assert!(skin.features.terminal);
    assert_eq!(skin.features.grid_cols, 3);
    assert_eq!(skin.features.grid_rows, 3);
}

#[test]
fn partial_features_fills_defaults() {
    let features = r#"
dashboard = false
window_manager = true
"#;
    let skin = Skin::from_toml(MANIFEST, LAYOUT, features).unwrap();
    assert!(!skin.features.dashboard);
    assert!(skin.features.window_manager);
    // Defaults for unspecified fields:
    assert!(skin.features.terminal);
    assert!(skin.features.browser);
    assert_eq!(skin.features.dashboard_pages, 1);
}

// -- Layout object partial fields --

#[test]
fn layout_object_partial_fields() {
    let layout = r##"
[partial_obj]
x = 10
color = "#FF0000"
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let obj = &skin.layout.objects["partial_obj"];
    assert_eq!(obj.x, Some(10));
    assert!(obj.y.is_none());
    assert!(obj.w.is_none());
    assert!(obj.h.is_none());
    assert_eq!(obj.color, Some("#FF0000".to_string()));
}

#[test]
fn layout_object_extended_visual_properties() {
    let layout = r##"
[fancy]
x = 0
y = 0
w = 100
h = 50
border_radius = 8
gradient_top = "#FF0000"
gradient_bottom = "#0000FF"
shadow_level = 3
stroke_width = 2
stroke_color = "#00FF00"
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let obj = &skin.layout.objects["fancy"];
    assert_eq!(obj.border_radius, Some(8));
    assert_eq!(obj.gradient_top, Some("#FF0000".to_string()));
    assert_eq!(obj.gradient_bottom, Some("#0000FF".to_string()));
    assert_eq!(obj.shadow_level, Some(3));
    assert_eq!(obj.stroke_width, Some(2));
    assert_eq!(obj.stroke_color, Some("#00FF00".to_string()));
}

#[test]
fn layout_object_text_shadow_properties() {
    let layout = r##"
[shadowed]
x = 0
y = 0
w = 100
h = 50
text_shadow_dx = 2
text_shadow_dy = 3
text_shadow_color = "#00000080"
shadow_color = "#FF000040"
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let obj = &skin.layout.objects["shadowed"];
    assert_eq!(obj.text_shadow_dx, Some(2));
    assert_eq!(obj.text_shadow_dy, Some(3));
    assert_eq!(obj.text_shadow_color, Some("#00000080".to_string()));
    assert_eq!(obj.shadow_color, Some("#FF000040".to_string()));
}

#[test]
fn apply_layout_text_shadow_properties() {
    let layout = r##"
[shadow_obj]
x = 0
y = 0
w = 100
h = 50
text_shadow_dx = 1
text_shadow_dy = 2
text_shadow_color = "#00000080"
shadow_color = "#FF000040"
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    let obj = sdi.get("shadow_obj").unwrap();
    assert_eq!(obj.text_shadow_offset, Some((1, 2)));
    assert_eq!(
        obj.text_shadow_color,
        Some(oasis_types::backend::Color::rgba(0, 0, 0, 128))
    );
    assert_eq!(
        obj.shadow_color,
        Some(oasis_types::backend::Color::rgba(255, 0, 0, 64))
    );
}

#[test]
fn apply_layout_text_shadow_defaults_dy() {
    let layout = r##"
[shadow_dx_only]
x = 0
y = 0
text_shadow_dx = 3
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    let obj = sdi.get("shadow_dx_only").unwrap();
    assert_eq!(obj.text_shadow_offset, Some((3, 1)));
}

#[test]
fn apply_layout_extended_properties() {
    let layout = r##"
[styled]
x = 5
y = 10
w = 100
h = 50
border_radius = 4
shadow_level = 2
stroke_width = 1
stroke_color = "#FFFFFF"
gradient_top = "#FF0000"
gradient_bottom = "#0000FF"
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    let obj = sdi.get("styled").unwrap();
    assert_eq!(obj.border_radius, Some(4));
    assert_eq!(obj.shadow_level, Some(2));
    assert_eq!(obj.stroke_width, Some(1));
}

// -- Invalid color strings --

#[test]
fn apply_layout_invalid_color_ignored() {
    let layout = r##"
[bad_color]
x = 0
y = 0
color = "not-a-color"
text_color = "also-bad"
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    // Object created but colors remain default (parse_hex_color returns None)
    assert!(sdi.contains("bad_color"));
}

// -- Skin swap --

#[test]
fn swap_returns_new_skin() {
    let skin_a = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    skin_a.apply_layout(&mut sdi);

    let manifest_b = r#"name = "new_skin""#;
    let layout_b = r##"
[new_obj]
x = 0
y = 0
w = 480
h = 272
"##;
    let skin_b = Skin::from_toml(manifest_b, layout_b, FEATURES).unwrap();
    let result = Skin::swap(&skin_a, skin_b, &mut sdi);
    assert_eq!(result.manifest.name, "new_skin");
}

// -- Empty corrupted TOML skips modifiers --

#[test]
fn from_toml_corrupted_empty_string() {
    let skin = Skin::from_toml_corrupted(MANIFEST, LAYOUT, FEATURES, "", "", "").unwrap();
    // Empty corrupted_toml means no override, but features.corrupted is false
    // so corrupted_modifiers is None.
    assert!(skin.corrupted_modifiers.is_none());
}

// -- Discover skins with nonexistent directory --

#[test]
fn discover_skins_nonexistent_dir() {
    let skins = Skin::discover_skins(Path::new("/nonexistent/path/to/skins"));
    assert!(skins.is_empty());
}

// -- from_directory with nonexistent dir --

#[test]
fn from_directory_missing_files() {
    let result = Skin::from_directory(Path::new("/nonexistent/skin/dir"));
    assert!(result.is_err());
}

// -- Robustness / edge cases ----------------------------------------

#[test]
fn completely_invalid_manifest_toml() {
    let result = Skin::from_toml("{{{{", LAYOUT, FEATURES);
    assert!(result.is_err());
}

#[test]
fn completely_invalid_layout_toml() {
    let result = Skin::from_toml(MANIFEST, "{{{{", FEATURES);
    assert!(result.is_err());
}

#[test]
fn completely_invalid_features_toml() {
    let result = Skin::from_toml(MANIFEST, LAYOUT, "{{{{");
    assert!(result.is_err());
}

#[test]
fn empty_manifest_toml() {
    let result = Skin::from_toml("", LAYOUT, FEATURES);
    // Empty manifest lacks required `name` -- should fail or use defaults.
    let _ = result;
}

#[test]
fn empty_layout_toml() {
    let skin = Skin::from_toml(MANIFEST, "", FEATURES).unwrap();
    assert!(skin.layout.objects.is_empty());
}

#[test]
fn empty_features_toml() {
    let skin = Skin::from_toml(MANIFEST, LAYOUT, "").unwrap();
    // Empty features should use defaults.
    let _ = skin.features;
}

#[test]
fn manifest_with_extra_fields() {
    let manifest = r#"
name = "test"
version = "1.0"
future_field = "should be ignored"
another_unknown = 42
"#;
    let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
    assert_eq!(skin.manifest.name, "test");
    // Unknown keys parse fine but are recorded as schema warnings.
    assert!(
        skin.schema_warnings
            .iter()
            .any(|w| w.contains("skin.toml") && w.contains("future_field")),
        "missing schema warning: {:?}",
        skin.schema_warnings
    );
}

#[test]
fn unknown_theme_keys_are_recorded() {
    let theme = r##"
background = "#000000"
not_a_real_field = true

[wm_theme]
titlebar_height = 20
close_button_colour = "#FF0000"

[transition]
bogus_ms = 300
"##;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
    let has = |s: &str| skin.schema_warnings.iter().any(|w| w.contains(s));
    assert!(has("not_a_real_field"), "{:?}", skin.schema_warnings);
    assert!(
        has("wm_theme.close_button_colour"),
        "{:?}",
        skin.schema_warnings
    );
    assert!(has("transition.bogus_ms"), "{:?}", skin.schema_warnings);
    // Warnings also flow through validate().
    assert!(skin.validate().iter().any(|w| w.contains("bogus_ms")));
}

#[test]
fn transition_ms_converts_to_frames() {
    let theme = r#"
[transition]
fade_ms = 300
slide_ms = 400
"#;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
    assert_eq!(skin.features.transition_fade_frames, Some(18));
    assert_eq!(skin.features.transition_slide_frames, Some(24));
}

#[test]
fn explicit_frames_beat_transition_ms() {
    let features = r#"
transition_fade_frames = 5
"#;
    let theme = r#"
[transition]
fade_ms = 1000
"#;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, features, theme, "").unwrap();
    assert_eq!(skin.features.transition_fade_frames, Some(5));
}

#[test]
fn bar_text_color_fallback() {
    let theme = r##"
[bar_overrides]
text_color = "#123456"
clock_color = "#654321"
"##;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
    let at = crate::ActiveTheme::from_skin(&skin.theme);
    // Specific color wins; unset elements fall back to text_color.
    assert_eq!(
        at.bar.clock_color,
        oasis_types::backend::Color::rgb(0x65, 0x43, 0x21)
    );
    assert_eq!(
        at.bar.version_color,
        oasis_types::backend::Color::rgb(0x12, 0x34, 0x56)
    );
    assert_eq!(
        at.bar.url_color,
        oasis_types::backend::Color::rgb(0x12, 0x34, 0x56)
    );
}

#[test]
fn all_shipped_skins_lint_clean() {
    let mut report = String::new();
    for name in crate::builtin::builtin_names() {
        let skin = crate::builtin::load_builtin(name).unwrap();
        for w in &skin.schema_warnings {
            report.push_str(&format!("builtin '{name}': {w}\n"));
        }
    }
    for name in crate::builtin::generated::GENERATED_SKIN_NAMES {
        let skin = crate::builtin::generated::load_generated_skin(name)
            .unwrap()
            .unwrap();
        for w in &skin.schema_warnings {
            report.push_str(&format!("generated '{name}': {w}\n"));
        }
    }
    assert!(
        report.is_empty(),
        "shipped skins have schema warnings:\n{report}"
    );
}

#[test]
fn layout_object_invalid_color() {
    let layout = r##"
[obj]
x = 0
y = 0
w = 100
h = 50
color = "not_a_color"
"##;
    // Invalid color should not crash -- may use fallback or ignore.
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    assert_eq!(skin.layout.objects.len(), 1);
}

#[test]
fn layout_object_empty_color() {
    let layout = r##"
[obj]
x = 0
y = 0
w = 100
h = 50
color = ""
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    assert_eq!(skin.layout.objects.len(), 1);
}

#[test]
fn layout_object_zero_dimensions() {
    let layout = r##"
[zero_obj]
x = 0
y = 0
w = 0
h = 0
color = "#FF0000"
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    assert_eq!(skin.layout.objects.len(), 1);
}

#[test]
fn layout_object_negative_position() {
    let layout = r##"
[neg_obj]
x = -100
y = -50
w = 200
h = 100
color = "#00FF00"
"##;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    assert_eq!(skin.layout.objects.len(), 1);
}

#[test]
fn layout_many_objects() {
    let mut layout = String::new();
    for i in 0..100 {
        layout.push_str(&format!(
            r##"
[obj_{i}]
x = {x}
y = {y}
w = 10
h = 10
color = "#AABBCC"
"##,
            x = (i % 48) * 10,
            y = (i / 48) * 10,
        ));
    }
    let skin = Skin::from_toml(MANIFEST, &layout, FEATURES).unwrap();
    assert_eq!(skin.layout.objects.len(), 100);
}

#[test]
fn features_boolean_fields() {
    let features = r#"
dashboard = false
terminal = false
file_browser = false
browser = false
window_manager = true
"#;
    let skin = Skin::from_toml(MANIFEST, LAYOUT, features).unwrap();
    assert!(!skin.features.dashboard);
    assert!(skin.features.window_manager);
}

#[test]
fn manifest_unicode_name() {
    let manifest = "name = \"\u{30b9}\u{30ad}\u{30f3}\"\nversion = \"1.0\"\nauthor = \"\u{30c6}\u{30b9}\u{30c8}\"\n";
    let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
    assert_eq!(skin.manifest.name, "\u{30b9}\u{30ad}\u{30f3}");
}

#[test]
fn manifest_very_long_name() {
    let name = "x".repeat(1000);
    let manifest = format!("name = \"{name}\"");
    let skin = Skin::from_toml(&manifest, LAYOUT, FEATURES).unwrap();
    assert_eq!(skin.manifest.name.len(), 1000);
}

// -- Skin inheritance tests --

#[test]
fn manifest_inherits_field() {
    let manifest = r#"
name = "child"
inherits = "classic"
"#;
    let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
    assert_eq!(skin.manifest.inherits.as_deref(), Some("classic"));
}

#[test]
fn manifest_inherits_default_none() {
    let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    assert!(skin.manifest.inherits.is_none());
}

#[test]
fn merge_theme_fills_missing_fields() {
    let parent_theme = r##"
border_radius = 8
shadow_intensity = 2

[gradients.accent]
from = "#FF0000"
to = "#880000"
"##;
    let mut parent = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, parent_theme, "").unwrap();
    // Give parent a unique name.
    parent.manifest.name = "parent".to_string();

    // Child has no border_radius or gradients.
    let mut child = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    assert!(child.theme.border_radius.is_none());
    assert!(child.theme.gradients.is_none());

    child.merge_theme_from(&parent);
    assert_eq!(child.theme.border_radius, Some(8));
    assert_eq!(child.theme.shadow_intensity, Some(2));
    assert!(child.theme.gradients.is_some());
}

#[test]
fn merge_theme_child_overrides_win() {
    let parent_theme = r##"
border_radius = 8
shadow_intensity = 2
"##;
    let child_theme = r##"
border_radius = 4
"##;
    let parent = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, parent_theme, "").unwrap();
    let mut child = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, child_theme, "").unwrap();

    child.merge_theme_from(&parent);
    // Child's border_radius should win over parent's.
    assert_eq!(child.theme.border_radius, Some(4));
    // Parent's shadow_intensity fills in.
    assert_eq!(child.theme.shadow_intensity, Some(2));
}

// -- External skin loading tests --

#[test]
fn load_macos_skin() {
    let skin = Skin::from_toml_full(
        include_str!("../../../../skins/macos/skin.toml"),
        include_str!("../../../../skins/macos/layout.toml"),
        include_str!("../../../../skins/macos/features.toml"),
        include_str!("../../../../skins/macos/theme.toml"),
        "",
    )
    .unwrap();
    assert_eq!(skin.manifest.name, "macos");
    assert_eq!(skin.manifest.screen_width, 800);
    assert_eq!(skin.manifest.screen_height, 600);
    assert!(skin.features.window_manager);
    assert!(skin.features.start_menu);
    assert!(skin.theme.wm_theme.is_some());
    let wm = skin.theme.wm_theme.as_ref().unwrap();
    assert_eq!(wm.button_side.as_deref(), Some("left"));
}

#[test]
fn load_gnome_skin() {
    let skin = Skin::from_toml_full(
        include_str!("../../../../skins/gnome/skin.toml"),
        include_str!("../../../../skins/gnome/layout.toml"),
        include_str!("../../../../skins/gnome/features.toml"),
        include_str!("../../../../skins/gnome/theme.toml"),
        "",
    )
    .unwrap();
    assert_eq!(skin.manifest.name, "gnome");
    assert_eq!(skin.manifest.screen_width, 800);
    assert!(skin.features.window_manager);
    assert_eq!(skin.theme.border_radius, Some(12));
    assert_eq!(skin.theme.gradient_enabled, Some(false));
}

#[test]
fn load_retro_cga_skin() {
    let skin = Skin::from_toml_full(
        include_str!("../../../../skins/retro-cga/skin.toml"),
        include_str!("../../../../skins/retro-cga/layout.toml"),
        include_str!("../../../../skins/retro-cga/features.toml"),
        include_str!("../../../../skins/retro-cga/theme.toml"),
        "",
    )
    .unwrap();
    assert_eq!(skin.manifest.name, "retro-cga");
    assert_eq!(skin.manifest.screen_width, 480);
    assert!(!skin.features.window_manager);
    assert_eq!(skin.theme.border_radius, Some(0));
    assert_eq!(skin.theme.shadow_intensity, Some(0));
    assert_eq!(skin.theme.gradient_enabled, Some(false));
    assert_eq!(skin.theme.background, "#000000");
}

#[test]
fn load_balatro_skin() {
    let skin = Skin::from_toml_full(
        include_str!("../../../../skins/balatro/skin.toml"),
        include_str!("../../../../skins/balatro/layout.toml"),
        include_str!("../../../../skins/balatro/features.toml"),
        include_str!("../../../../skins/balatro/theme.toml"),
        "",
    )
    .unwrap();
    assert_eq!(skin.manifest.name, "balatro");
    assert_eq!(skin.manifest.screen_width, 800);
    assert!(skin.features.window_manager);
    assert_eq!(skin.theme.primary, "#00F0FF");
    assert_eq!(skin.theme.shadow_intensity, Some(2));
    assert!(skin.theme.wm_theme.is_some());
}

#[test]
fn load_paper_skin() {
    let skin = Skin::from_toml_full(
        include_str!("../../../../skins/paper/skin.toml"),
        include_str!("../../../../skins/paper/layout.toml"),
        include_str!("../../../../skins/paper/features.toml"),
        include_str!("../../../../skins/paper/theme.toml"),
        "",
    )
    .unwrap();
    assert_eq!(skin.manifest.name, "paper");
    assert_eq!(skin.manifest.screen_width, 480);
    assert!(!skin.features.window_manager);
    assert_eq!(skin.theme.border_radius, Some(0));
    assert_eq!(skin.theme.shadow_intensity, Some(0));
    assert_eq!(skin.theme.gradient_enabled, Some(false));
    assert_eq!(skin.theme.background, "#FAF8F0");
}

#[test]
fn load_win95_skin() {
    let skin = Skin::from_toml_full(
        include_str!("../../../../skins/win95/skin.toml"),
        include_str!("../../../../skins/win95/layout.toml"),
        include_str!("../../../../skins/win95/features.toml"),
        include_str!("../../../../skins/win95/theme.toml"),
        "",
    )
    .unwrap();
    assert_eq!(skin.manifest.name, "win95");
    assert_eq!(skin.manifest.screen_width, 640);
    assert_eq!(skin.manifest.screen_height, 480);
    assert!(skin.features.window_manager);
    assert_eq!(skin.theme.border_radius, Some(0));
    assert_eq!(skin.theme.gradient_enabled, Some(false));
    assert_eq!(skin.theme.background, "#008080");
}

#[test]
fn load_solarized_skin() {
    let skin = Skin::from_toml_full(
        include_str!("../../../../skins/solarized/skin.toml"),
        include_str!("../../../../skins/solarized/layout.toml"),
        include_str!("../../../../skins/solarized/features.toml"),
        include_str!("../../../../skins/solarized/theme.toml"),
        "",
    )
    .unwrap();
    assert_eq!(skin.manifest.name, "solarized");
    assert_eq!(skin.manifest.screen_width, 800);
    assert_eq!(skin.manifest.screen_height, 600);
    assert!(skin.features.window_manager);
    assert_eq!(skin.theme.background, "#002B36");
}

#[test]
fn load_vaporwave_skin() {
    let skin = Skin::from_toml_full(
        include_str!("../../../../skins/vaporwave/skin.toml"),
        include_str!("../../../../skins/vaporwave/layout.toml"),
        include_str!("../../../../skins/vaporwave/features.toml"),
        include_str!("../../../../skins/vaporwave/theme.toml"),
        "",
    )
    .unwrap();
    assert_eq!(skin.manifest.name, "vaporwave");
    assert_eq!(skin.manifest.screen_width, 480);
    assert!(!skin.features.window_manager);
    assert_eq!(skin.theme.gradient_enabled, Some(true));
    assert_eq!(skin.theme.background, "#1A0A2E");
}

#[test]
fn load_highcontrast_skin() {
    let skin = Skin::from_toml_full(
        include_str!("../../../../skins/highcontrast/skin.toml"),
        include_str!("../../../../skins/highcontrast/layout.toml"),
        include_str!("../../../../skins/highcontrast/features.toml"),
        include_str!("../../../../skins/highcontrast/theme.toml"),
        "",
    )
    .unwrap();
    assert_eq!(skin.manifest.name, "highcontrast");
    assert_eq!(skin.theme.border_radius, Some(0));
    assert_eq!(skin.theme.shadow_intensity, Some(0));
    assert_eq!(skin.theme.gradient_enabled, Some(false));
    assert_eq!(skin.theme.background, "#000000");
    assert_eq!(skin.theme.text, "#FFFFFF");
}

#[test]
fn merge_theme_layout_merges() {
    let parent_layout = r#"
[parent_object]
x = 10
y = 20
w = 100
h = 50
"#;
    let parent = Skin::from_toml(MANIFEST, parent_layout, FEATURES).unwrap();
    let mut child = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    let had_parent_obj = child.layout.objects.contains_key("parent_object");
    assert!(!had_parent_obj);

    child.merge_theme_from(&parent);
    assert!(child.layout.objects.contains_key("parent_object"));
}

// -- Validation tests --

#[test]
fn validate_valid_skin() {
    let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    let warnings = skin.validate();
    assert!(warnings.is_empty(), "expected no warnings: {warnings:?}");
}

#[test]
fn validate_low_contrast_theme_warns() {
    // Contrast failures surface as advisory warnings with the
    // computed ratio in the message.
    let theme_toml = "background = \"#808080\"\ntext = \"#909090\"\n";
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, "").unwrap();
    let warnings = skin.validate();
    let w = warnings
        .iter()
        .find(|w| w.starts_with("contrast: text on background"))
        .unwrap_or_else(|| panic!("missing contrast warning: {warnings:?}"));
    assert!(w.contains(":1"), "ratio missing from message: {w}");
    assert!(w.contains("4.5:1"), "threshold missing from message: {w}");
}

#[test]
fn validate_all_builtin_skins_contrast_lint_runs() {
    // The contrast lint must never panic; warnings are fine
    // (stylized skins like corrupted/vaporwave may fail on purpose).
    for name in crate::builtin::builtin_names() {
        let skin = crate::builtin::load_builtin(name)
            .unwrap_or_else(|e| panic!("builtin '{name}' failed to load: {e}"));
        let _warnings = skin.validate();
    }
}

#[test]
fn validate_empty_name() {
    let manifest = r#"name = """#;
    let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
    let warnings = skin.validate();
    assert!(
        warnings.iter().any(|w| w.contains("name is empty")),
        "missing empty name warning: {warnings:?}"
    );
}

#[test]
fn validate_invalid_theme_color() {
    let theme_toml = "background = \"not-a-color\"\nprimary = \"#FF0000\"\n";
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, "").unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("background") && w.contains("not-a-color")),
        "missing invalid color warning: {warnings:?}"
    );
}

#[test]
fn validate_invalid_layout_color() {
    let layout = "[bad_obj]\nx = 10\ny = 20\ncolor = \"xyz\"\n";
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("bad_obj") && w.contains("xyz")),
        "missing invalid layout color warning: {warnings:?}"
    );
}

#[test]
fn validate_extreme_coordinates() {
    let layout = "[far_away]\nx = 999999\ny = 0\n";
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("far_away") && w.contains("x=")),
        "missing out-of-bounds warning: {warnings:?}"
    );
}

#[test]
fn validate_icons_exceed_grid() {
    let features = "dashboard = true\nicons_per_page = 20\ngrid_cols = 3\ngrid_rows = 3\n";
    let skin = Skin::from_toml(MANIFEST, LAYOUT, features).unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("icons_per_page") && w.contains("grid capacity")),
        "missing grid capacity warning: {warnings:?}"
    );
}

#[test]
fn validate_chrome_layer_unsupported_kind() {
    let theme_toml = r#"
[[chrome_layers]]
kind = "image"
source = "assets/x.png"

[[chrome_layers]]
kind = "crosshair"
size = 12
"#;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, "").unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("chrome_layers[0]") && w.contains("image")),
        "missing chrome layer kind warning: {warnings:?}"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("chrome_layers[1]")),
        "vector kind should not warn: {warnings:?}"
    );
}

#[test]
fn validate_widget_states_accepts_known_slots() {
    let theme_toml = r##"
[widget_states.button]
hover_bg = "#656565"

[widget_states.slider]
track = "#101018"
fill = "#FF8C1E"
thumb = "#F0F0E8"

[widget_states.menu]
bg = "#2A2A30"
hover_bg = "#F5820F"
separator = "#55555C"
"##;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, "").unwrap();
    let warnings = skin.validate();
    assert!(
        !warnings.iter().any(|w| w.contains("widget_states")),
        "known widget_states slots should not warn: {warnings:?}"
    );
}

#[test]
fn validate_widget_states_flags_typos() {
    let theme_toml = r##"
[widget_states.slider]
trak = "#101018"

[widget_states.menus]
bg = "#2A2A30"
"##;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, "").unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("widget_states.slider.trak") && w.contains("unknown state")),
        "missing typo'd slot warning: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("widget_states.menus") && w.contains("unknown widget")),
        "missing unknown widget warning: {warnings:?}"
    );
}

#[test]
fn validate_zero_screen_dimensions() {
    let manifest = "name = \"zero\"\nscreen_width = 0\nscreen_height = 0\n";
    let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
    let warnings = skin.validate();
    assert!(
        warnings.iter().any(|w| w.contains("screen_width is 0")),
        "missing zero width warning: {warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| w.contains("screen_height is 0")),
        "missing zero height warning: {warnings:?}"
    );
}

// -- Asset tests --

fn png_bytes() -> Vec<u8> {
    crate::assets::tests::encode_png(8, 8, png::ColorType::Rgba)
}

#[test]
fn add_asset_png_registers_asset() {
    let mut skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    skin.add_asset_png("assets/logo.png", &png_bytes()).unwrap();
    let asset = &skin.assets["assets/logo.png"];
    assert_eq!(asset.width, 8);
    assert_eq!(asset.height, 8);
}

#[test]
fn add_asset_png_invalid_bytes_errors() {
    let mut skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    let err = skin.add_asset_png("assets/bad.png", b"nope").unwrap_err();
    assert!(format!("{err}").contains("assets/bad.png"));
}

#[test]
fn from_directory_loads_assets() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("skin.toml"), MANIFEST).unwrap();
    std::fs::write(dir.path().join("layout.toml"), LAYOUT).unwrap();
    std::fs::write(dir.path().join("features.toml"), FEATURES).unwrap();
    let assets = dir.path().join("assets");
    std::fs::create_dir(&assets).unwrap();
    std::fs::write(assets.join("logo.png"), png_bytes()).unwrap();
    std::fs::write(assets.join("broken.png"), b"not a png").unwrap();
    std::fs::write(assets.join("readme.txt"), b"ignored").unwrap();

    let skin = Skin::from_directory(dir.path()).unwrap();
    assert!(skin.assets.contains_key("assets/logo.png"));
    assert!(!skin.assets.contains_key("assets/readme.txt"));
    // Broken PNG is skipped with a warning, not a hard failure.
    assert!(!skin.assets.contains_key("assets/broken.png"));
    assert!(
        skin.schema_warnings
            .iter()
            .any(|w| w.contains("assets/broken.png")),
        "{:?}",
        skin.schema_warnings
    );
}

#[test]
fn layout_texture_field_parses() {
    let layout = r#"
[bar_top]
x = 0
y = 0
w = 480
h = 24
texture = "assets/bar_top.png"
"#;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    assert_eq!(
        skin.layout.objects["bar_top"].texture.as_deref(),
        Some("assets/bar_top.png")
    );
}

#[test]
fn upload_layout_textures_attaches_and_sizes() {
    let layout = r#"
[logo]
x = 10
y = 20
texture = "assets/logo.png"

[bar]
x = 0
y = 0
w = 480
h = 24
texture = "assets/logo.png"

[plain]
x = 0
y = 0
w = 10
h = 10
"#;
    let mut skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    skin.add_asset_png("assets/logo.png", &png_bytes()).unwrap();
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
    let ids = skin.upload_layout_textures(&mut sdi, &mut backend);
    assert_eq!(ids.len(), 2);
    // No explicit w/h -> asset native dims.
    let logo = sdi.get("logo").unwrap();
    assert!(logo.texture.is_some());
    assert_eq!((logo.w, logo.h), (8, 8));
    // Explicit w/h wins over asset dims.
    let bar = sdi.get("bar").unwrap();
    assert!(bar.texture.is_some());
    assert_eq!((bar.w, bar.h), (480, 24));
    assert!(sdi.get("plain").unwrap().texture.is_none());
}

#[test]
fn upload_layout_nine_patch_attaches_slices() {
    let layout = r#"
[panel]
x = 0
y = 0
w = 200
h = 60
nine_patch = { image = "assets/logo.png", insets = [2, 3, 2, 3] }
"#;
    let mut skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    skin.add_asset_png("assets/logo.png", &png_bytes()).unwrap();
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
    let ids = skin.upload_layout_textures(&mut sdi, &mut backend);
    assert_eq!(ids.len(), 1);
    let panel = sdi.get("panel").unwrap();
    assert!(panel.texture.is_some());
    let slices = panel.nine_patch.expect("nine_patch slices attached");
    assert_eq!((slices.tex_width, slices.tex_height), (8, 8));
    assert_eq!(
        (slices.left, slices.top, slices.right, slices.bottom),
        (2, 3, 2, 3)
    );
    assert_eq!((panel.w, panel.h), (200, 60));
}

#[test]
fn validate_nine_patch_missing_asset_and_bad_insets() {
    let layout = r#"
[panel]
x = 0
y = 0
w = 100
h = 40
nine_patch = { image = "assets/missing.png", insets = [4, 4, 4, 4] }

[chunky]
x = 0
y = 0
w = 100
h = 40
nine_patch = { image = "assets/logo.png", insets = [5, 5, 5, 5] }
"#;
    let mut skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    // logo.png is 8x8, so 5+5 insets cannot fit.
    skin.add_asset_png("assets/logo.png", &png_bytes()).unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("panel") && w.contains("missing asset")),
        "missing nine_patch asset warning: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("chunky") && w.contains("don't fit")),
        "missing insets warning: {warnings:?}"
    );
}

#[test]
fn upload_layout_textures_missing_asset_skipped() {
    let layout = r#"
[ghost]
x = 0
y = 0
texture = "assets/missing.png"
"#;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
    let ids = skin.upload_layout_textures(&mut sdi, &mut backend);
    assert!(ids.is_empty());
    assert!(sdi.get("ghost").unwrap().texture.is_none());
}

#[test]
fn validate_missing_texture_asset() {
    let layout = r#"
[ghost]
x = 0
y = 0
texture = "assets/missing.png"
"#;
    let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("ghost") && w.contains("assets/missing.png")),
        "{warnings:?}"
    );
}

#[test]
fn validate_image_wallpaper_sources() {
    let theme = r#"
[wallpaper]
style = "image"
fit = "sideways"
"#;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
    let warnings = skin.validate();
    assert!(
        warnings.iter().any(|w| w.contains("no source set")),
        "{warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| w.contains("unknown fit")),
        "{warnings:?}"
    );
}

#[test]
fn validate_image_layer_sources() {
    let theme = r#"
[[background_layers]]
kind = "image"
source = "assets/nope.png"

[[background_layers]]
kind = "image"
"#;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("background_layers[0]") && w.contains("assets/nope.png")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("background_layers[1]") && w.contains("no source set")),
        "{warnings:?}"
    );
}

#[test]
fn validate_non_pot_asset_is_fine() {
    // The PSP backend pads NPOT textures on upload, so odd sizes are
    // not a lint error (psix-hifi ships a 160x80 wordmark).
    let mut skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    let bytes = crate::assets::tests::encode_png(10, 8, png::ColorType::Rgba);
    skin.add_asset_png("assets/odd.png", &bytes).unwrap();
    let warnings = skin.validate();
    assert!(
        !warnings.iter().any(|w| w.contains("assets/odd.png")),
        "{warnings:?}"
    );
}

#[test]
fn image_layer_theme_derivation() {
    let theme = r#"
[[background_layers]]
kind = "image"
source = "assets/logo.png"
alpha = 128

[background_layers.position]
anchor = "bottom_right"
offset_x = -0.05

[background_layers.animation]
pulse_speed = 0.5

[[background_layers]]
kind = "grid"
"#;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
    let at = crate::ActiveTheme::from_skin(&skin.theme);
    // Image layers are split out; the grid stays a vector layer.
    assert_eq!(at.image_layers.len(), 1);
    assert_eq!(at.background_layers.len(), 1);
    let layer = &at.image_layers[0];
    assert_eq!(layer.source, "assets/logo.png");
    assert_eq!(layer.alpha, 128);
    assert!((layer.animation.pulse_speed - 0.5).abs() < f32::EPSILON);
    assert!(matches!(
        layer.position.anchor,
        oasis_vector::background::Anchor::BottomRight
    ));
}

#[test]
fn wallpaper_image_theme_derivation() {
    let theme = r#"
[wallpaper]
style = "image"
source = "assets/wall.png"
fit = "tile"
"#;
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
    let at = crate::ActiveTheme::from_skin(&skin.theme);
    assert_eq!(at.wallpaper.style, "image");
    assert_eq!(at.wallpaper.source.as_deref(), Some("assets/wall.png"));
    assert_eq!(at.wallpaper.fit, "tile");
}

#[test]
fn validate_optional_theme_color() {
    let theme_toml = "surface = \"bad\"\naccent_hover = \"#FF0000\"\n";
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, "").unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("surface") && w.contains("bad")),
        "missing invalid optional color warning: {warnings:?}"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("accent_hover")),
        "unexpected accent_hover warning: {warnings:?}"
    );
}

// -- Typography font asset tests --

const FONT_THEME: &str = r#"
[typography]
font = "assets/skin.ttf"
"#;

/// The oasis-browser web-font test fixture: a tiny but valid TTF.
const TEST_TTF: &[u8] = include_bytes!("../../../oasis-browser/test_data/minimal.ttf");

#[test]
fn typography_font_key_parses_without_warning() {
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, FONT_THEME, "").unwrap();
    assert_eq!(
        skin.theme
            .typography
            .as_ref()
            .and_then(|t| t.font.as_deref()),
        Some("assets/skin.ttf")
    );
    // The key is a real schema field, not an unknown-key warning.
    assert!(
        !skin.schema_warnings.iter().any(|w| w.contains("font")),
        "{:?}",
        skin.schema_warnings
    );
}

#[test]
fn active_font_bytes_resolves_registered_font() {
    let mut skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, FONT_THEME, "").unwrap();
    assert!(skin.active_font_bytes().is_none(), "font not loaded yet");
    skin.add_asset_font("assets/skin.ttf", TEST_TTF.to_vec());
    assert_eq!(skin.active_font_bytes(), Some(TEST_TTF));
}

#[test]
fn active_font_bytes_none_without_typography() {
    let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    assert!(skin.active_font_bytes().is_none());
}

#[test]
fn validate_missing_font_asset_warns() {
    let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, FONT_THEME, "").unwrap();
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("typography.font") && w.contains("missing")),
        "{warnings:?}"
    );
}

#[test]
fn validate_registered_font_ok() {
    let mut skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, FONT_THEME, "").unwrap();
    skin.add_asset_font("assets/skin.ttf", TEST_TTF.to_vec());
    let warnings = skin.validate();
    assert!(
        !warnings.iter().any(|w| w.contains("typography.font")),
        "{warnings:?}"
    );
}

#[test]
fn validate_font_over_budget_warns() {
    let mut skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, FONT_THEME, "").unwrap();
    // Valid header so the (feature-gated) parse check isn't what fires,
    // padded past the budget.
    let mut bytes = TEST_TTF.to_vec();
    bytes.resize(crate::assets::FONT_BUDGET_BYTES + 1, 0);
    skin.add_asset_font("assets/skin.ttf", bytes);
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("typography.font") && w.contains("budget")),
        "{warnings:?}"
    );
}

#[cfg(feature = "ttf")]
#[test]
fn validate_unparseable_font_warns() {
    let mut skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, FONT_THEME, "").unwrap();
    skin.add_asset_font("assets/skin.ttf", b"not a font at all".to_vec());
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("typography.font") && w.contains("failed to parse")),
        "{warnings:?}"
    );
}

#[test]
fn from_directory_loads_font_assets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    std::fs::write(path.join("skin.toml"), MANIFEST).expect("skin.toml");
    std::fs::write(path.join("layout.toml"), LAYOUT).expect("layout.toml");
    std::fs::write(path.join("features.toml"), FEATURES).expect("features.toml");
    std::fs::write(path.join("theme.toml"), FONT_THEME).expect("theme.toml");
    std::fs::create_dir(path.join("assets")).expect("assets dir");
    std::fs::write(path.join("assets/skin.ttf"), TEST_TTF).expect("font file");

    let skin = Skin::from_directory(path).expect("skin loads");
    assert_eq!(skin.active_font_bytes(), Some(TEST_TTF));
    assert!(
        !skin
            .validate()
            .iter()
            .any(|w| w.contains("typography.font")),
        "{:?}",
        skin.validate()
    );
}

#[test]
fn merge_theme_fills_font_assets() {
    let mut parent = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, FONT_THEME, "").unwrap();
    parent.add_asset_font("assets/skin.ttf", TEST_TTF.to_vec());

    let mut child = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
    child.merge_theme_from(&parent);
    // Child inherits both the typography table and the font bytes it
    // references.
    assert_eq!(child.active_font_bytes(), Some(TEST_TTF));
}

//! Tests for the skin validator (`Skin::validate`).

use crate::assets::tests::make_wav;
use crate::loader::Skin;

const MANIFEST: &str = r#"name = "sound-test""#;

fn skin_with_sounds(theme_toml: &str) -> Skin {
    Skin::from_toml_full(MANIFEST, "", "", theme_toml, "").unwrap()
}

#[test]
fn sounds_table_parses() {
    let skin = skin_with_sounds(
        r#"
[sounds]
click = "assets/click.wav"
open = "assets/open.wav"
close = "assets/close.wav"
error = "assets/error.wav"
toast = "assets/toast.wav"
nav = "assets/nav.wav"
volume = 0.5
"#,
    );
    let sounds = skin.theme.sounds.as_ref().unwrap();
    assert_eq!(sounds.click.as_deref(), Some("assets/click.wav"));
    assert_eq!(sounds.nav.as_deref(), Some("assets/nav.wav"));
    assert_eq!(sounds.path_for("toast"), Some("assets/toast.wav"));
    assert!((sounds.effective_volume() - 0.5).abs() < f32::EPSILON);
    assert!(
        skin.schema_warnings.is_empty(),
        "{:?}",
        skin.schema_warnings
    );
}

#[test]
fn sounds_unknown_key_warns() {
    let skin = skin_with_sounds(
        r#"
[sounds]
clik = "assets/click.wav"
"#,
    );
    assert!(
        skin.schema_warnings
            .iter()
            .any(|w| w.contains("sounds.clik")),
        "{:?}",
        skin.schema_warnings
    );
}

#[test]
fn no_sounds_table_is_silent_and_valid() {
    let skin = skin_with_sounds("");
    assert!(skin.theme.sounds.is_none());
    assert!(
        !skin.validate().iter().any(|w| w.contains("sounds")),
        "{:?}",
        skin.validate()
    );
}

#[test]
fn missing_sound_asset_warns() {
    let skin = skin_with_sounds(
        r#"
[sounds]
click = "assets/click.wav"
"#,
    );
    let warnings = skin.validate();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("click") && w.contains("missing asset")),
        "{warnings:?}"
    );
}

#[test]
fn valid_sound_asset_passes() {
    let mut skin = skin_with_sounds(
        r#"
[sounds]
click = "assets/click.wav"
"#,
    );
    let wav = make_wav(&[0i16; 480], 48_000, 1); // 10 ms
    skin.add_asset_wav("assets/click.wav", &wav).unwrap();
    let warnings = skin.validate();
    assert!(
        !warnings.iter().any(|w| w.contains("sounds")),
        "{warnings:?}"
    );
}

#[test]
fn long_sound_warns_duration() {
    let mut skin = skin_with_sounds(
        r#"
[sounds]
open = "assets/open.wav"
"#,
    );
    // 3 seconds at 8 kHz mono.
    let wav = make_wav(&[0i16; 24_000], 8_000, 1);
    skin.add_asset_wav("assets/open.wav", &wav).unwrap();
    let warnings = skin.validate();
    assert!(warnings.iter().any(|w| w.contains("3.0s")), "{warnings:?}");
}

#[test]
fn out_of_range_volume_warns() {
    let skin = skin_with_sounds(
        r#"
[sounds]
volume = 1.5
"#,
    );
    let warnings = skin.validate();
    assert!(
        warnings.iter().any(|w| w.contains("volume 1.5")),
        "{warnings:?}"
    );
    // Runtime clamps rather than failing.
    assert!((skin.theme.sounds.as_ref().unwrap().effective_volume() - 1.0).abs() < f32::EPSILON);
}

#[test]
fn add_asset_wav_rejects_garbage() {
    let mut skin = skin_with_sounds("");
    assert!(skin.add_asset_wav("assets/bad.wav", b"nope").is_err());
    assert!(skin.sound_assets.is_empty());
}

#[test]
fn sound_bytes_count_toward_budget() {
    let mut skin = skin_with_sounds(
        r#"
[sounds]
click = "assets/click.wav"
"#,
    );
    // A WAV larger than the whole per-skin budget on its own.
    let frames = crate::assets::ASSET_BUDGET_BYTES / 2 + 1024;
    let wav = make_wav(&vec![0i16; frames], 48_000, 1);
    skin.add_asset_wav("assets/click.wav", &wav).unwrap();
    let warnings = skin.validate();
    assert!(
        warnings.iter().any(|w| w.contains("per-skin budget")),
        "{warnings:?}"
    );
}

// -- Colorblind (deuteranopia/protanopia) lint --

fn colorblind_warnings(theme_toml: &str) -> Vec<String> {
    let skin = Skin::from_toml_full(MANIFEST, "", "", theme_toml, "").unwrap();
    skin.validate()
        .into_iter()
        .filter(|w| w.contains("red/green hue"))
        .collect()
}

#[test]
fn default_theme_has_no_colorblind_warnings() {
    // The default palette (bright green prompt vs red error) is separable
    // by luminance, so real skins inheriting it must not be spammed.
    let warnings = colorblind_warnings("");
    assert!(warnings.is_empty(), "default theme warned: {warnings:?}");
}

#[test]
fn red_green_low_luminance_pair_warns() {
    // Pure red prompt vs a mid green error of similar luminance: a
    // deuteranope/protanope cannot tell them apart.
    let warnings = colorblind_warnings("prompt = \"#FF0000\"\nerror = \"#009000\"\n");
    assert!(
        warnings.iter().any(|w| w.contains("'prompt' and 'error'")),
        "expected colorblind warning: {warnings:?}"
    );
}

#[test]
fn high_luminance_red_green_pair_does_not_warn() {
    // Bright green prompt vs dim red error: distinguishable by luminance
    // even without hue, so the heuristic must NOT fire.
    let warnings =
        colorblind_warnings("prompt = \"#00FF00\"\nerror = \"#440000\"\noutput = \"#00FF00\"\n");
    assert!(
        warnings.is_empty(),
        "high-separation pair should not warn: {warnings:?}"
    );
}

#[test]
fn inheritance_merges_sounds() {
    let mut parent = skin_with_sounds(
        r#"
[sounds]
click = "assets/click.wav"
"#,
    );
    let wav = make_wav(&[0i16; 48], 48_000, 1);
    parent.add_asset_wav("assets/click.wav", &wav).unwrap();

    let mut child = skin_with_sounds("");
    child.merge_theme_from(&parent);
    assert!(child.theme.sounds.is_some());
    assert!(child.sound_assets.contains_key("assets/click.wav"));
}

#[test]
fn unknown_shader_name_warns() {
    let skin = skin_with_sounds(
        r#"
[[background_layers]]
kind = "shader"
shader = "balatr0"

[[background_layers]]
kind = "shader"
shader = "calm_waves"
"#,
    );
    let warnings = skin.validate();
    let shader: Vec<_> = warnings.iter().filter(|w| w.contains("shader")).collect();
    assert_eq!(shader.len(), 1, "{warnings:?}");
    assert!(shader[0].contains("background_layers[0]"), "{}", shader[0]);
    assert!(shader[0].contains("balatr0"), "{}", shader[0]);
}

#[test]
fn builtin_skin_shader_names_are_known() {
    for name in crate::builtin::builtin_names() {
        let skin = crate::builtin::load_builtin(name).unwrap();
        let bad: Vec<_> = skin
            .validate()
            .into_iter()
            .filter(|w| w.contains("unknown shader"))
            .collect();
        assert!(bad.is_empty(), "{name}: {bad:?}");
    }
}

/// Write a minimal skin directory with the given `skin.toml` / `theme.toml`.
#[cfg(not(target_arch = "wasm32"))]
fn skin_dir(manifest: &str, theme: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("skin.toml"), manifest).unwrap();
    std::fs::write(dir.path().join("layout.toml"), "").unwrap();
    std::fs::write(dir.path().join("features.toml"), "").unwrap();
    std::fs::write(dir.path().join("theme.toml"), theme).unwrap();
    dir
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn parse_errors_name_directory_file_and_line() {
    let dir = skin_dir(MANIFEST, "background = \"#000000\"\nprimary = \n");
    let err = Skin::from_directory(dir.path()).unwrap_err().to_string();
    assert!(err.contains(&dir.path().display().to_string()), "{err}");
    assert!(err.contains("theme.toml"), "{err}");
    assert!(err.contains("line 2"), "{err}");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn unknown_parent_skin_warns() {
    let dir = skin_dir("name = \"child\"\ninherits = \"no-such-parent\"\n", "");
    let skin = Skin::from_directory(dir.path()).unwrap();
    assert!(
        skin.validate()
            .iter()
            .any(|w| w.contains("inherits = \"no-such-parent\"")),
        "{:?}",
        skin.validate()
    );
}

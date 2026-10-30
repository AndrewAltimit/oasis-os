//! Built-in skins.
//!
//! Every built-in skin is a directory under `skins/` at the repo root. The
//! `oasis-skin` build script (`build.rs`) embeds each directory's TOML files
//! (and `assets/`) with `include_str!` / `include_bytes!` and generates one
//! loader per skin plus the name list, so the TOML directories are the single
//! source of truth: `--skin xp` (built-in) and `--skin skins/xp` (directory)
//! load the same skin.

#[cfg(all(test, feature = "serialize"))]
mod snapshot_tests;
#[cfg(all(test, feature = "serialize"))]
pub(crate) use snapshot_tests::canonical_dump as snapshot_dump;

/// Build-time generated skin loaders from `skins/` TOML directories.
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/builtin_generated.rs"));
}

use oasis_types::error::{OasisError, Result};

use super::loader::Skin;

/// Load the Classic skin -- the default skin, and the fallback when a
/// requested skin cannot be resolved (see `resolve_skin_or_classic`).
pub fn classic_skin() -> Result<Skin> {
    load_builtin("classic")
}

/// Load a built-in skin by name with optional inheritance.
///
/// If the skin's manifest specifies `inherits = "parent_name"`, the parent
/// skin is loaded first and its theme/layout fields fill in any missing
/// values in the child. Max inheritance depth is 3.
pub fn load_builtin(name: &str) -> Result<Skin> {
    load_builtin_recursive(name, 0)
}

/// Whether `name` is the name of a built-in skin.
pub fn is_builtin(name: &str) -> bool {
    builtin_names().contains(&name)
}

fn load_builtin_raw(name: &str) -> Result<Skin> {
    generated::load_generated_skin(name).unwrap_or_else(|| {
        Err(OasisError::Config(
            format!("unknown built-in skin: {name}").into(),
        ))
    })
}

fn load_builtin_recursive(name: &str, depth: u32) -> Result<Skin> {
    const MAX_DEPTH: u32 = 3;
    if depth > MAX_DEPTH {
        return Err(OasisError::Config(
            format!("skin inheritance depth exceeds {MAX_DEPTH} for '{name}'").into(),
        ));
    }

    let mut skin = load_builtin_raw(name)?;

    if let Some(ref parent_name) = skin.manifest.inherits {
        let parent = load_builtin_recursive(parent_name, depth + 1)?;
        skin.merge_theme_from(&parent);
    }

    Ok(skin)
}

/// List available built-in skin names: `classic` first, then alphabetical.
pub fn builtin_names() -> &'static [&'static str] {
    generated::GENERATED_SKIN_NAMES
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_sdi::SdiRegistry;

    #[test]
    fn corrupted_skin_loads() {
        let skin = load_builtin("corrupted").unwrap();
        assert_eq!(skin.manifest.name, "corrupted");
        assert!(skin.features.corrupted);
        assert!(skin.corrupted_modifiers.is_some());
        let mods = skin.corrupted_modifiers.as_ref().unwrap();
        assert_eq!(mods.position_jitter, 2);
        assert!(mods.text_garble_chance > 0.0);
    }

    #[test]
    fn corrupted_skin_has_glitch_overlay() {
        let skin = load_builtin("corrupted").unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        assert!(sdi.contains("glitch_overlay"));
    }

    #[test]
    fn desktop_skin_loads() {
        let skin = load_builtin("desktop").unwrap();
        assert_eq!(skin.manifest.name, "desktop");
        assert!(skin.features.window_manager);
        assert!(!skin.features.dashboard);
        assert_eq!(skin.manifest.screen_width, 800);
        assert_eq!(skin.manifest.screen_height, 600);
    }

    #[test]
    fn desktop_skin_has_wm_theme() {
        let skin = load_builtin("desktop").unwrap();
        let wm = skin.theme.build_wm_theme();
        assert_eq!(wm.titlebar_height, 24);
        assert_eq!(wm.button_size, 16);
    }

    #[test]
    fn desktop_skin_has_taskbar() {
        let skin = load_builtin("desktop").unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        assert!(sdi.contains("taskbar_bg"));
    }

    #[test]
    fn load_builtin_all_names() {
        for name in builtin_names() {
            let skin = load_builtin(name).unwrap();
            assert_eq!(skin.manifest.name, *name);
        }
    }

    #[test]
    fn load_builtin_unknown_fails() {
        assert!(load_builtin("nonexistent").is_err());
    }

    #[test]
    fn swap_between_builtin_skins() {
        let modern = load_builtin("modern").unwrap();
        let desktop = load_builtin("desktop").unwrap();

        let mut sdi = SdiRegistry::new();
        modern.apply_layout(&mut sdi);
        assert!(sdi.contains("content_bg"));

        let _new = Skin::swap(&modern, desktop, &mut sdi);
        // Modern-only objects removed, desktop objects created.
        assert!(!sdi.contains("content_bg"));
        assert!(sdi.contains("desktop_bg"));
        assert!(sdi.contains("taskbar_bg"));
    }

    #[test]
    fn all_skins_have_boot_text() {
        for name in builtin_names() {
            let skin = load_builtin(name).unwrap();
            assert!(
                !skin.strings.boot_text.is_empty(),
                "{name} skin should have boot text"
            );
        }
    }

    #[test]
    fn all_skins_have_prompt() {
        for name in builtin_names() {
            let skin = load_builtin(name).unwrap();
            assert!(
                !skin.strings.prompt_format.is_empty(),
                "{name} skin should have prompt format"
            );
        }
    }

    #[test]
    fn modern_skin_loads() {
        let skin = load_builtin("modern").unwrap();
        assert_eq!(skin.manifest.name, "modern");
        assert!(skin.features.dashboard);
        assert!(skin.features.terminal);
        assert!(skin.features.window_manager);
        assert!(skin.features.browser);
        assert_eq!(skin.manifest.screen_width, 480);
        assert_eq!(skin.manifest.screen_height, 272);
    }

    #[test]
    fn modern_skin_has_wm_theme() {
        let skin = load_builtin("modern").unwrap();
        let wm = skin.theme.build_wm_theme();
        assert_eq!(wm.titlebar_radius, 6);
        assert!(wm.titlebar_gradient);
        assert_eq!(wm.frame_shadow_level, 2);
        assert_eq!(wm.frame_border_radius, 4);
        assert_eq!(wm.button_radius, 8);
    }

    #[test]
    fn modern_skin_has_extended_theme_fields() {
        let skin = load_builtin("modern").unwrap();
        assert_eq!(skin.theme.border_radius, Some(6));
        assert_eq!(skin.theme.shadow_intensity, Some(2));
        assert_eq!(skin.theme.gradient_enabled, Some(true));
        assert!(skin.theme.surface.is_some());
        assert!(skin.theme.accent_hover.is_some());
    }

    #[test]
    fn modern_skin_applies_layout() {
        let skin = load_builtin("modern").unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        assert!(sdi.contains("content_bg"));
    }

    #[test]
    fn modern_skin_content_bg_has_gradient() {
        let skin = load_builtin("modern").unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        let obj = sdi.get("content_bg").unwrap();
        assert!(obj.gradient_top.is_some());
        assert!(obj.gradient_bottom.is_some());
    }

    #[test]
    fn desktop_skin_has_extended_wm_fields() {
        let skin = load_builtin("desktop").unwrap();
        let wm = skin.theme.build_wm_theme();
        assert_eq!(wm.titlebar_radius, 4);
        assert_eq!(wm.frame_shadow_level, 1);
        assert_eq!(wm.frame_border_radius, 2);
        assert_eq!(wm.button_radius, 8);
    }

    #[test]
    fn desktop_skin_taskbar_has_gradient() {
        let skin = load_builtin("desktop").unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        let obj = sdi.get("taskbar_bg").unwrap();
        assert!(obj.gradient_top.is_some());
        assert!(obj.gradient_bottom.is_some());
    }

    // -- Skin inheritance tests --

    #[test]
    fn load_builtin_with_inheritance_all_names() {
        // Ensure all built-in skins load without error (including inheritance).
        for name in builtin_names() {
            let skin = load_builtin(name);
            assert!(skin.is_ok(), "failed to load builtin skin '{name}'");
        }
    }

    #[test]
    fn load_builtin_with_inheritance_unknown_fails() {
        assert!(load_builtin("nonexistent").is_err());
    }

    #[test]
    fn no_builtin_has_inherits_by_default() {
        // Legacy built-in skins don't use inheritance; the psix-* tributes
        // are the showcase for the `inherits` authoring pattern (on classic).
        for name in builtin_names() {
            let skin = load_builtin(name).unwrap();
            if *name == "psix-tribute" || *name == "psix-hifi" {
                assert_eq!(skin.manifest.inherits.as_deref(), Some("classic"));
            } else {
                assert!(
                    skin.manifest.inherits.is_none(),
                    "built-in skin '{name}' has unexpected inherits field"
                );
            }
        }
    }

    // -- Generated skin tests --

    #[test]
    fn all_generated_skins_load() {
        for name in super::generated::GENERATED_SKIN_NAMES {
            let result = super::generated::load_generated_skin(name);
            assert!(result.is_some(), "no generated loader for '{name}'");
            let skin = result.unwrap();
            assert!(
                skin.is_ok(),
                "generated skin '{name}' failed to load: {:?}",
                skin.err()
            );
            let skin = skin.unwrap();
            assert_eq!(skin.manifest.name, *name);
        }
    }

    #[test]
    fn generated_skin_dispatch_unknown_returns_none() {
        assert!(super::generated::load_generated_skin("nonexistent").is_none());
    }

    /// Hard floor for every built-in skin: no checked text/background
    /// pair may drop below 2:1 (effectively invisible). The WCAG AA
    /// 4.5 / 3.0 thresholds stay advisory (`validate_contrast`).
    #[test]
    fn all_builtin_skins_meet_minimum_contrast() {
        const FLOOR: f64 = 2.0;
        assert_eq!(builtin_names().len(), 17);
        let mut failures = Vec::new();
        for name in builtin_names() {
            let skin = load_builtin(name).expect("built-in skin loads");
            let placed = skin.features.clock_in_bottombar;
            for c in skin.theme.contrast_report(placed) {
                if c.ratio < FLOOR {
                    failures.push(format!("{name}: {} = {:.2}:1", c.pair, c.ratio));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "text/background pairs below {FLOOR}:1:\n{}",
            failures.join("\n")
        );
    }

    /// Advisory WCAG contrast misses the built-in skins keep on purpose:
    /// `(skin, pair, why)`. Everything else `skin lint` reports on a shipped
    /// skin is a bug (in `skins/` or in the validator) and fails
    /// [`all_builtin_skins_lint_clean`]. Entries must still fire, so a fixed
    /// pair has to be removed from this list.
    const ACCEPTED_CONTRAST: &[(&str, &str, &str)] = &[
        (
            "corrupted",
            "text on background",
            "glitch aesthetic: washed-out phosphor text",
        ),
        ("corrupted", "dim_text on background", "glitch aesthetic"),
        (
            "corrupted",
            "app text on window content",
            "glitch aesthetic",
        ),
        (
            "corrupted",
            "terminal output on app background",
            "glitch aesthetic",
        ),
        (
            "corrupted",
            "button text on button background",
            "glitch aesthetic",
        ),
        (
            "retro-cga",
            "button text on button background",
            "fixed 4-color CGA palette",
        ),
        (
            "solarized",
            "dim_text on background",
            "canonical base01 on base03",
        ),
        (
            "solarized",
            "button text on button background",
            "canonical base0 on base02",
        ),
        // win95 / xp: `background` is the desktop wallpaper; body text is
        // drawn on the gray / cream app surface (checked separately).
        (
            "win95",
            "text on background",
            "base text never sits on the teal desktop",
        ),
        (
            "win95",
            "dim_text on background",
            "base text never sits on the teal desktop",
        ),
        (
            "win95",
            "error on background",
            "errors render on the gray app surface",
        ),
        (
            "xp",
            "error on background",
            "errors render on the cream app surface",
        ),
    ];

    /// `skin lint` must report every built-in skin as clean, apart from the
    /// documented [`ACCEPTED_CONTRAST`] misses.
    #[test]
    fn all_builtin_skins_lint_clean() {
        let mut failures = Vec::new();
        let mut fired = vec![false; ACCEPTED_CONTRAST.len()];
        for name in builtin_names() {
            let skin = load_builtin(name).expect("built-in skin loads");
            for w in skin.validate() {
                let accepted = ACCEPTED_CONTRAST.iter().position(|(s, pair, _)| {
                    s == name && w.starts_with(&format!("contrast: {pair} is"))
                });
                match accepted {
                    Some(i) => fired[i] = true,
                    None => failures.push(format!("{name}: {w}")),
                }
            }
        }
        for (i, (skin, pair, _)) in ACCEPTED_CONTRAST.iter().enumerate() {
            if !fired[i] {
                failures.push(format!("stale ACCEPTED_CONTRAST entry: {skin} / {pair}"));
            }
        }
        assert!(
            failures.is_empty(),
            "skin lint warnings:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn contrast_checks_clock_on_its_actual_bar() {
        // macOS draws the clock in its dark dock-style bottom bar.
        let skin = load_builtin("macos").expect("macos loads");
        assert!(skin.features.clock_in_bottombar);
        let report = skin.theme.contrast_report(true);
        assert!(report.iter().any(|c| c.pair == "clock on bottom bar"));
        let top = skin.theme.contrast_report(false);
        assert!(top.iter().all(|c| c.pair != "clock on bottom bar"));
    }

    #[test]
    fn highcontrast_icon_colors_are_high_contrast() {
        let skin = load_builtin("highcontrast").expect("highcontrast loads");
        let at = crate::active_theme::ActiveTheme::from_skin(&skin.theme);
        let bg = skin.theme.background_color();
        assert!(!at.icon.fallback_colors.is_empty());
        for c in &at.icon.fallback_colors {
            let ratio = oasis_types::color::contrast_ratio(*c, bg);
            assert!(
                ratio >= 7.0,
                "icon color {c:?} is only {ratio:.2}:1 on {bg:?}"
            );
        }
        // The grey Win2K "pixel" chrome is not a high-contrast surface.
        assert_ne!(at.icon.vector_preset, "pixel");
    }
}

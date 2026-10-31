//! Asset lints: referenced images / fonts / sounds must resolve, parse and
//! stay inside the per-skin budgets.

use super::super::Skin;

impl Skin {
    /// Asset-related validation: wallpaper / layer / cursor / font / sound
    /// references must resolve, and the per-skin size budget is enforced.
    pub(super) fn validate_assets(&self, warnings: &mut Vec<String>) {
        // Wallpaper image source.
        if let Some(ref wp) = self.theme.wallpaper {
            let style_is_image = wp.style.as_deref() == Some("image");
            match (&wp.source, style_is_image) {
                (Some(src), _) if !self.assets.contains_key(src) => {
                    warnings.push(format!("wallpaper: missing asset \"{src}\""));
                },
                (None, true) => {
                    warnings.push("wallpaper: style is \"image\" but no source set".to_string());
                },
                _ => {},
            }
            if let Some(ref fit) = wp.fit
                && !matches!(fit.as_str(), "cover" | "contain" | "stretch" | "tile")
            {
                warnings.push(format!(
                    "wallpaper: unknown fit \"{fit}\" (expected cover|contain|stretch|tile)"
                ));
            }
        }

        // Bar tab pill textures.
        if let Some(ref bars) = self.theme.bar_overrides {
            for (key, src) in [
                ("tab_texture_active", &bars.tab_texture_active),
                ("tab_texture_inactive", &bars.tab_texture_inactive),
            ] {
                if let Some(src) = src
                    && !self.assets.contains_key(src)
                {
                    warnings.push(format!("bar_overrides.{key}: missing asset \"{src}\""));
                }
            }
        }

        // Software cursor texture.
        if let Some(ref cursor) = self.theme.cursor
            && let Some(ref src) = cursor.texture
            && !self.assets.contains_key(src)
        {
            warnings.push(format!("cursor: missing asset \"{src}\""));
        }

        // Image background layers.
        if let Some(ref layers) = self.theme.background_layers {
            for (i, layer) in layers.iter().enumerate() {
                if layer.kind != "image" {
                    continue;
                }
                match &layer.source {
                    Some(src) if !self.assets.contains_key(src) => {
                        warnings.push(format!("background_layers[{i}]: missing asset \"{src}\""));
                    },
                    None => {
                        warnings.push(format!(
                            "background_layers[{i}]: kind is \"image\" but no source set"
                        ));
                    },
                    _ => {},
                }
            }
        }

        // Shader background layers: the name must be a built-in shader. An
        // unknown name draws no wallpaper on the GPU path, so flag typos.
        if let Some(ref layers) = self.theme.background_layers {
            for (i, layer) in layers.iter().enumerate() {
                if layer.kind != "shader" {
                    continue;
                }
                if let Some(ref name) = layer.shader
                    && oasis_shader::BuiltinShader::from_name(name).is_none()
                {
                    let known: Vec<&str> = oasis_shader::BuiltinShader::ALL
                        .iter()
                        .map(|s| s.name())
                        .collect();
                    warnings.push(format!(
                        "background_layers[{i}]: unknown shader \"{name}\" (built-in: {})",
                        known.join(", ")
                    ));
                }
            }
        }

        // Chrome layers are vector-only: image/shader kinds are silently
        // dropped at render time, so flag them here for the author.
        if let Some(ref layers) = self.theme.chrome_layers {
            for (i, layer) in layers.iter().enumerate() {
                if matches!(layer.kind.as_str(), "image" | "shader") {
                    warnings.push(format!(
                        "chrome_layers[{i}]: kind \"{}\" is not supported in chrome layers \
                         (vector kinds only)",
                        layer.kind
                    ));
                }
            }
        }

        // Widget state overrides: warn on widget/slot names nothing reads.
        if let Some(ref states) = self.theme.widget_states {
            const KNOWN: &[(&str, &[&str])] = &[
                (
                    "button",
                    &[
                        "normal_bg",
                        "hover_bg",
                        "pressed_bg",
                        "disabled_bg",
                        "text",
                        "disabled_text",
                    ],
                ),
                ("input", &["bg", "border", "focus_border"]),
                ("toggle", &["track_off", "track_on", "thumb"]),
                ("scrollbar", &["track", "thumb", "thumb_hover"]),
                ("slider", &["track", "fill", "thumb"]),
                (
                    "menu",
                    &[
                        "bg",
                        "border",
                        "text",
                        "hover_bg",
                        "hover_text",
                        "dropdown_bg",
                        "dropdown_border_light",
                        "dropdown_border_dark",
                        "item_text",
                        "disabled_text",
                        "separator",
                    ],
                ),
            ];
            let mut widgets: Vec<&String> = states.keys().collect();
            widgets.sort();
            for widget in widgets {
                match KNOWN.iter().find(|(w, _)| w == widget) {
                    None => warnings.push(format!(
                        "widget_states.{widget}: unknown widget (expected one of: \
                         button, input, toggle, scrollbar, slider, menu)"
                    )),
                    Some((_, keys)) => {
                        let mut slots: Vec<&String> = states[widget].keys().collect();
                        slots.sort();
                        for slot in slots {
                            if !keys.contains(&slot.as_str()) {
                                warnings.push(format!(
                                    "widget_states.{widget}.{slot}: unknown state slot"
                                ));
                            }
                        }
                    },
                }
            }
        }

        // Typography font: the reference must resolve to loaded TTF/OTF
        // bytes, stay inside the size budget, and (when the `ttf` feature is
        // on) actually parse as a font.
        if let Some(ref typo) = self.theme.typography
            && let Some(ref font) = typo.font
        {
            match self.font_assets.get(font) {
                None => {
                    warnings.push(format!(
                        "typography.font: missing font asset \"{font}\" \
                         (expected a .ttf/.otf under assets/)"
                    ));
                },
                Some(bytes) => {
                    if bytes.len() > crate::assets::FONT_BUDGET_BYTES {
                        warnings.push(format!(
                            "typography.font: \"{font}\" is {} KB, over the {} KB budget \
                             (subset the font)",
                            bytes.len() / 1024,
                            crate::assets::FONT_BUDGET_BYTES / 1024
                        ));
                    }
                    #[cfg(feature = "ttf")]
                    {
                        let settings = fontdue::FontSettings {
                            collection_index: 0,
                            scale: 40.0,
                            load_substitutions: true,
                        };
                        if let Err(e) = fontdue::Font::from_bytes(bytes.as_slice(), settings) {
                            warnings
                                .push(format!("typography.font: \"{font}\" failed to parse: {e}"));
                        }
                    }
                },
            }
        }

        // Typography: a zero font size renders nothing, and the bitmap font is
        // unreadable past a few dozen pixels — both are almost always typos.
        if let Some(ref typo) = self.theme.typography {
            for (name, value) in [
                ("font_size_xs", typo.font_size_xs),
                ("font_size_sm", typo.font_size_sm),
                ("font_size_md", typo.font_size_md),
                ("font_size_lg", typo.font_size_lg),
                ("font_size_xl", typo.font_size_xl),
                ("font_size_xxl", typo.font_size_xxl),
            ] {
                match value {
                    Some(0) => warnings.push(format!(
                        "typography.{name}: 0 renders no text (omit the key to keep the default)"
                    )),
                    Some(v) if v > 64 => warnings.push(format!(
                        "typography.{name}: {v}px is beyond the bitmap font's usable range (max 64)"
                    )),
                    _ => {},
                }
            }
            for (name, value) in [
                ("spacing_xs", typo.spacing_xs),
                ("spacing_sm", typo.spacing_sm),
                ("spacing_md", typo.spacing_md),
                ("spacing_lg", typo.spacing_lg),
                ("spacing_xl", typo.spacing_xl),
            ] {
                if let Some(v) = value
                    && v > 64
                {
                    warnings.push(format!(
                        "typography.{name}: {v}px will push widget content off small screens"
                    ));
                }
            }
        }

        // UI sound theme ([sounds] in theme.toml).
        self.validate_sounds(warnings);

        // Total budget. Sound bytes count toward the same per-skin budget as
        // decoded images. Non-power-of-two images are fine: the PSP backend
        // pads every texture up to the next power of two on upload
        // (`oasis-backend-psp/src/textures.rs`), so NPOT is not an error.
        let total_bytes = self.assets.values().map(|a| a.byte_size()).sum::<usize>()
            + self.sound_assets.values().map(Vec::len).sum::<usize>();
        if total_bytes > crate::assets::ASSET_BUDGET_BYTES {
            warnings.push(format!(
                "assets: {} KB decoded exceeds the {} KB per-skin budget",
                total_bytes / 1024,
                crate::assets::ASSET_BUDGET_BYTES / 1024
            ));
        }
    }

    /// `[sounds]` checks: referenced assets must exist and parse as PCM
    /// WAV, sounds should be short one-shots, volume must be sane.
    ///
    /// See the tests module at the bottom of this file for coverage.
    fn validate_sounds(&self, warnings: &mut Vec<String>) {
        /// UI sounds are one-shots; anything longer than this is probably
        /// a music loop pointed at the wrong table.
        const MAX_SOUND_SECS: f32 = 2.0;

        let Some(ref sounds) = self.theme.sounds else {
            return;
        };
        if let Some(v) = sounds.volume
            && !(0.0..=1.0).contains(&v)
        {
            warnings.push(format!(
                "sounds: volume {v} outside 0.0-1.0 (clamped at runtime)"
            ));
        }
        for (event, path) in sounds.entries() {
            let Some(path) = path else { continue };
            match self.sound_assets.get(path) {
                None => {
                    warnings.push(format!(
                        "sounds: {event} references missing asset \"{path}\""
                    ));
                },
                Some(bytes) => match crate::assets::probe_wav(bytes) {
                    None => {
                        warnings.push(format!(
                            "sounds: {event} asset \"{path}\" is not an uncompressed PCM WAV"
                        ));
                    },
                    Some(info) => {
                        let secs = info.duration_secs();
                        if secs > MAX_SOUND_SECS {
                            warnings.push(format!(
                                "sounds: {event} asset \"{path}\" is {secs:.1}s long \
                                 (keep UI sounds under {MAX_SOUND_SECS:.0}s)"
                            ));
                        }
                    },
                },
            }
        }
    }
}

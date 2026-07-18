//! Skin validation helpers.

use oasis_types::backend::Color;

use crate::theme::{contrast_ratio, parse_hex_color};

use super::Skin;

/// Signed position of a color on the red-vs-green axis.
///
/// Positive => red-dominant, negative => green-dominant, near-zero => neutral.
fn red_green_axis(c: Color) -> i32 {
    i32::from(c.r) - i32::from(c.g)
}

/// Whether two colors are distinguished *primarily* by red-vs-green hue: one is
/// clearly red-dominant and the other clearly green-dominant. Colors on opposite
/// ends of the red/green axis are exactly what deuteranopia/protanopia collapse.
fn red_green_opposed(a: Color, b: Color) -> bool {
    const MIN_AXIS: i32 = 40;
    // A color only sits on the red/green confusion axis when blue is not its
    // dominant channel. Without this guard a blue such as #4488CC (where the
    // green channel exceeds red purely by channel math) is misread as
    // "green-dominant" and falsely paired with a red -- deuteranopia does not
    // collapse blue against red.
    let on_axis = |c: Color| {
        red_green_axis(c).abs() >= MIN_AXIS && i32::from(c.b) <= i32::from(c.r).max(i32::from(c.g))
    };
    on_axis(a) && on_axis(b) && (red_green_axis(a).signum() != red_green_axis(b).signum())
}

impl Skin {
    /// Validate a loaded skin and return a list of warnings.
    ///
    /// Checks required fields, valid hex color values, layout coordinate
    /// bounds, and recognized feature flag values. Returns an empty `Vec`
    /// when the skin is fully valid.
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        // -- Schema checks (unknown keys recorded at parse time) --
        warnings.extend(self.schema_warnings.iter().cloned());

        // -- Manifest checks --
        if self.manifest.name.is_empty() {
            warnings.push("manifest: name is empty (set a non-empty name = \"...\")".to_string());
        }
        if self.manifest.screen_width == 0 {
            warnings.push(
                "manifest: screen_width is 0 (set a positive pixel width, e.g. 480)".to_string(),
            );
        }
        if self.manifest.screen_height == 0 {
            warnings.push(
                "manifest: screen_height is 0 (set a positive pixel height, e.g. 272)".to_string(),
            );
        }
        // Sanity bound: screens larger than 8K are suspicious.
        const MAX_SCREEN: u32 = 7680;
        if self.manifest.screen_width > MAX_SCREEN {
            warnings.push(format!(
                "manifest: screen_width {}px exceeds the {}px maximum (use a smaller resolution)",
                self.manifest.screen_width, MAX_SCREEN,
            ));
        }
        if self.manifest.screen_height > MAX_SCREEN {
            warnings.push(format!(
                "manifest: screen_height {}px exceeds the {}px maximum (use a smaller resolution)",
                self.manifest.screen_height, MAX_SCREEN,
            ));
        }

        // -- Theme color checks --
        let theme_colors: &[(&str, &str)] = &[
            ("background", &self.theme.background),
            ("primary", &self.theme.primary),
            ("secondary", &self.theme.secondary),
            ("text", &self.theme.text),
            ("dim_text", &self.theme.dim_text),
            ("status_bar", &self.theme.status_bar),
            ("prompt", &self.theme.prompt),
            ("output", &self.theme.output),
            ("error", &self.theme.error),
        ];
        for (name, value) in theme_colors {
            if parse_hex_color(value).is_none() {
                warnings.push(format!(
                    "theme: invalid color for '{name}': \"{value}\" \
                     (expected #RRGGBB or #RRGGBBAA)"
                ));
            }
        }
        // Optional theme color fields.
        let opt_colors: &[(&str, &Option<String>)] = &[
            ("surface", &self.theme.surface),
            ("accent", &self.theme.accent),
            ("accent_hover", &self.theme.accent_hover),
        ];
        for (name, value) in opt_colors {
            if let Some(v) = value
                && parse_hex_color(v).is_none()
            {
                warnings.push(format!(
                    "theme: invalid color for '{name}': \"{v}\" \
                     (expected #RRGGBB or #RRGGBBAA)"
                ));
            }
        }

        // -- [palette] ANSI slot checks --
        if let Some(ref palette) = self.theme.palette {
            for (idx, name) in crate::active_theme::AnsiPalette::SLOT_NAMES
                .iter()
                .enumerate()
            {
                if let Some(v) = palette.slot(idx)
                    && parse_hex_color(v).is_none()
                {
                    warnings.push(format!("palette: invalid color for '{name}': \"{v}\""));
                }
            }
        }

        // -- [cursor] checks --
        if let Some(ref cursor) = self.theme.cursor {
            let cursor_colors: &[(&str, &Option<String>)] =
                &[("fill", &cursor.fill), ("outline", &cursor.outline)];
            for (name, value) in cursor_colors {
                if let Some(v) = value
                    && parse_hex_color(v).is_none()
                {
                    warnings.push(format!("cursor: invalid color for '{name}': \"{v}\""));
                }
            }
        }

        // -- [boot] checks --
        if let Some(ref boot) = self.theme.boot {
            let boot_colors: &[(&str, &Option<String>)] = &[
                ("banner_bg", &boot.banner_bg),
                ("banner_border", &boot.banner_border),
                ("chrome", &boot.chrome),
                ("text", &boot.text),
                ("bios_text", &boot.bios_text),
            ];
            for (name, value) in boot_colors {
                if let Some(v) = value
                    && parse_hex_color(v).is_none()
                {
                    warnings.push(format!("boot: invalid color for '{name}': \"{v}\""));
                }
            }
            let boot_stop_lists: &[(&str, &Option<Vec<String>>, usize)] = &[
                ("sky_stops", &boot.sky_stops, 6),
                ("ground_stops", &boot.ground_stops, 4),
            ];
            for (name, stops, expected) in boot_stop_lists {
                if let Some(list) = stops {
                    if list.len() != *expected {
                        warnings.push(format!(
                            "boot: {name} has {} entries, expected {expected}",
                            list.len()
                        ));
                    }
                    for v in list {
                        if parse_hex_color(v).is_none() {
                            warnings.push(format!("boot: invalid {name} color: \"{v}\""));
                        }
                    }
                }
            }
        }

        // -- Layout coordinate checks --
        let sw = self.manifest.screen_width as i32;
        let sh = self.manifest.screen_height as i32;
        // Allow some overshoot for scrollable/off-screen elements (4x).
        let max_coord = (sw.max(sh) * 4).max(4096);
        for (obj_name, def) in &self.layout.objects {
            if let Some(x) = def.x
                && (x < -max_coord || x > max_coord)
            {
                warnings.push(format!(
                    "layout: '{obj_name}' x={x} outside bounds [-{max_coord}, {max_coord}]"
                ));
            }
            if let Some(y) = def.y
                && (y < -max_coord || y > max_coord)
            {
                warnings.push(format!(
                    "layout: '{obj_name}' y={y} outside bounds [-{max_coord}, {max_coord}]"
                ));
            }
            if let Some(w) = def.w
                && w > max_coord as u32
            {
                warnings.push(format!("layout: '{obj_name}' w={w} exceeds {max_coord}"));
            }
            if let Some(h) = def.h
                && h > max_coord as u32
            {
                warnings.push(format!("layout: '{obj_name}' h={h} exceeds {max_coord}"));
            }
            // Validate color strings in layout objects.
            let obj_colors: &[(&str, &Option<String>)] = &[
                ("color", &def.color),
                ("text_color", &def.text_color),
                ("gradient_top", &def.gradient_top),
                ("gradient_bottom", &def.gradient_bottom),
                ("stroke_color", &def.stroke_color),
                ("text_shadow_color", &def.text_shadow_color),
                ("shadow_color", &def.shadow_color),
            ];
            for (field, value) in obj_colors {
                if let Some(v) = value
                    && !v.is_empty()
                    && parse_hex_color(v).is_none()
                {
                    warnings.push(format!("layout: '{obj_name}' invalid {field}: \"{v}\""));
                }
            }
            // Texture references must resolve to a loaded asset.
            if let Some(ref tex) = def.texture
                && !self.assets.contains_key(tex)
            {
                warnings.push(format!(
                    "layout: '{obj_name}' references missing asset \"{tex}\""
                ));
            }
            // Nine-patch: asset must resolve and insets must fit inside it.
            if let Some(ref np) = def.nine_patch {
                match self.assets.get(&np.image) {
                    None => {
                        warnings.push(format!(
                            "layout: '{obj_name}' nine_patch references missing asset \"{}\"",
                            np.image
                        ));
                    },
                    Some(asset) => {
                        let [left, top, right, bottom] = np.insets;
                        if u32::from(left) + u32::from(right) >= asset.width
                            || u32::from(top) + u32::from(bottom) >= asset.height
                        {
                            warnings.push(format!(
                                "layout: '{obj_name}' nine_patch insets \
                                 [{left}, {top}, {right}, {bottom}] don't fit inside \
                                 {}x{} \"{}\"",
                                asset.width, asset.height, np.image
                            ));
                        }
                    },
                }
                if def.texture.is_some() {
                    warnings.push(format!(
                        "layout: '{obj_name}' sets both texture and nine_patch \
                         (nine_patch wins)"
                    ));
                }
            }
        }

        // -- Asset checks --
        self.validate_assets(&mut warnings);

        // -- Accessibility: WCAG AA contrast (advisory) --
        // Stylized skins may fail these on purpose; the lint informs
        // authors and never blocks loading.
        for c in self.theme.validate_contrast() {
            warnings.push(format!(
                "contrast: {} is {:.2}:1, below the WCAG AA recommendation of {}:1",
                c.pair, c.ratio, c.required
            ));
        }
        // Colorblind-unsafe semantic pairs (deuteranopia/protanopia), advisory.
        self.check_colorblind_pairs(&mut warnings);

        // -- Feature flag checks --
        if !matches!(
            self.features.icon_layout.as_str(),
            "grid" | "free" | "column"
        ) {
            warnings.push(format!(
                "features: unknown icon_layout \"{}\" (expected grid|free|column)",
                self.features.icon_layout,
            ));
        }
        if self.features.free_icon_cols == Some(0) {
            warnings.push(
                "features: free_icon_cols is 0 (set at least 1, or omit for auto)".to_string(),
            );
        }
        if !matches!(self.features.bottombar_style.as_str(), "" | "media_dock") {
            warnings.push(format!(
                "features: unknown bottombar_style \"{}\" (expected \"\"|media_dock)",
                self.features.bottombar_style,
            ));
        }
        if self.features.grid_cols == 0 {
            warnings.push(
                "features: grid_cols is 0 (set at least 1 so the icon grid has a column)"
                    .to_string(),
            );
        }
        if self.features.grid_rows == 0 {
            warnings.push(
                "features: grid_rows is 0 (set at least 1 so the icon grid has a row)".to_string(),
            );
        }
        if self.features.dashboard_pages == 0 && self.features.dashboard {
            warnings.push("features: dashboard is enabled but dashboard_pages is 0".to_string());
        }
        if self.features.icons_per_page == 0 && self.features.dashboard {
            warnings.push("features: dashboard is enabled but icons_per_page is 0".to_string());
        }
        if self.features.icons_per_page > self.features.grid_cols * self.features.grid_rows {
            warnings.push(format!(
                "features: icons_per_page ({}) exceeds grid capacity ({}x{}={})",
                self.features.icons_per_page,
                self.features.grid_cols,
                self.features.grid_rows,
                self.features.grid_cols * self.features.grid_rows,
            ));
        }

        warnings
    }

    /// Flag semantic color pairs that a user must tell apart but which are
    /// distinguished *only* by red-vs-green hue with little luminance
    /// separation -- the classic deuteranopia/protanopia failure mode. Warn
    /// only when the two colors are both red/green-opposed AND close in
    /// luminance (inter-color contrast below 1.5:1), so a normal high-contrast
    /// green/red pair (e.g. a bright prompt vs a dim error) does not warn.
    ///
    /// Advisory only: never blocks loading.
    fn check_colorblind_pairs(&self, warnings: &mut Vec<String>) {
        let error = self.theme.error_color();
        // (label_a, color_a, label_b, color_b) -- pairs that carry meaning.
        let pairs: &[(&str, Color, &str, Color)] = &[
            ("prompt", self.theme.prompt_color(), "error", error),
            ("output", self.theme.output_color(), "error", error),
            ("primary", self.theme.primary_color(), "error", error),
        ];

        const MIN_LUMA_RATIO: f64 = 1.5;
        for &(la, ca, lb, cb) in pairs {
            if red_green_opposed(ca, cb) && contrast_ratio(ca, cb) < MIN_LUMA_RATIO {
                warnings.push(format!(
                    "accessibility: '{la}' and '{lb}' differ mainly by red/green hue \
                     with low luminance separation ({:.2}:1); risky for \
                     deuteranopia/protanopia",
                    contrast_ratio(ca, cb)
                ));
            }
        }
    }

    /// Asset-related validation: wallpaper/background sources must resolve,
    /// PSP-hostile dimensions and oversized budgets are flagged.
    fn validate_assets(&self, warnings: &mut Vec<String>) {
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

        // Per-asset dimension checks + total budget. Sound bytes count
        // toward the same per-skin budget as decoded images.
        let mut total_bytes = 0usize;
        let mut names: Vec<&String> = self.assets.keys().collect();
        names.sort();
        for name in names {
            let asset = &self.assets[name];
            total_bytes += asset.byte_size();
            if !asset.is_power_of_two() {
                warnings.push(format!(
                    "asset \"{name}\": {}x{} is not power-of-two (required on PSP)",
                    asset.width, asset.height
                ));
            }
        }
        for bytes in self.sound_assets.values() {
            total_bytes += bytes.len();
        }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

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
        assert!(
            (skin.theme.sounds.as_ref().unwrap().effective_volume() - 1.0).abs() < f32::EPSILON
        );
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
        let warnings = colorblind_warnings(
            "prompt = \"#00FF00\"\nerror = \"#440000\"\noutput = \"#00FF00\"\n",
        );
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
}

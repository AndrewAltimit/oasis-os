//! Skin validation helpers.

use crate::theme::parse_hex_color;

use super::Skin;

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
            warnings.push("manifest: name is empty".to_string());
        }
        if self.manifest.screen_width == 0 {
            warnings.push("manifest: screen_width is 0".to_string());
        }
        if self.manifest.screen_height == 0 {
            warnings.push("manifest: screen_height is 0".to_string());
        }
        // Sanity bound: screens larger than 8K are suspicious.
        const MAX_SCREEN: u32 = 7680;
        if self.manifest.screen_width > MAX_SCREEN {
            warnings.push(format!(
                "manifest: screen_width {} exceeds {}",
                self.manifest.screen_width, MAX_SCREEN,
            ));
        }
        if self.manifest.screen_height > MAX_SCREEN {
            warnings.push(format!(
                "manifest: screen_height {} exceeds {}",
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
                warnings.push(format!("theme: invalid color for '{name}': \"{value}\""));
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
                warnings.push(format!("theme: invalid color for '{name}': \"{v}\""));
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

        // -- Feature flag checks --
        if !matches!(self.features.icon_layout.as_str(), "grid" | "free") {
            warnings.push(format!(
                "features: unknown icon_layout \"{}\" (expected grid|free)",
                self.features.icon_layout,
            ));
        }
        if self.features.grid_cols == 0 {
            warnings.push("features: grid_cols is 0".to_string());
        }
        if self.features.grid_rows == 0 {
            warnings.push("features: grid_rows is 0".to_string());
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
            ];
            let mut widgets: Vec<&String> = states.keys().collect();
            widgets.sort();
            for widget in widgets {
                match KNOWN.iter().find(|(w, _)| w == widget) {
                    None => warnings.push(format!(
                        "widget_states.{widget}: unknown widget (expected one of: \
                         button, input, toggle, scrollbar)"
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

        // Per-asset dimension checks + total budget.
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
        if total_bytes > crate::assets::ASSET_BUDGET_BYTES {
            warnings.push(format!(
                "assets: {} KB decoded exceeds the {} KB per-skin budget",
                total_bytes / 1024,
                crate::assets::ASSET_BUDGET_BYTES / 1024
            ));
        }
    }
}

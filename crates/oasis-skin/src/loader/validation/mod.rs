//! Skin validation (`skin lint`): [`Skin::validate`] and its check groups.
//!
//! - `mod.rs` -- manifest, base colors, layout objects, feature flags, and
//!   the advisory WCAG contrast report
//! - `assets` -- image / font / sound references, budgets, widget-state slots
//! - `vocabulary` -- enumerated string values (layer kinds, anchors, easing
//!   names, wallpaper styles, ...) that would otherwise fall back silently
//! - `accessibility` -- colorblind-unsafe semantic color pairs

mod accessibility;
mod assets;
mod vocabulary;

pub(crate) use vocabulary::lint_color_strings;

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
        // Desktop render size: the shell only honours the pair, so a lone
        // width or height is silently ignored.
        match (self.manifest.desktop_width, self.manifest.desktop_height) {
            (Some(_), None) | (None, Some(_)) => warnings.push(
                "manifest: desktop_width and desktop_height must be set together \
                 (one alone is ignored)"
                    .to_string(),
            ),
            (Some(w), Some(h)) if w == 0 || h == 0 || w > MAX_SCREEN || h > MAX_SCREEN => {
                warnings.push(format!(
                    "manifest: desktop size {w}x{h} is outside 1..={MAX_SCREEN} pixels"
                ));
            },
            _ => {},
        }

        // -- Theme scalar checks --
        let ts = self.theme.text_scale;
        if !ts.is_finite() || !(0.5..=3.0).contains(&ts) {
            warnings.push(format!(
                "theme: text_scale {ts} outside 0.5-3.0 (clamped at runtime)"
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
            ("success", &self.theme.success),
            ("warning", &self.theme.warning),
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
        // Allow some overshoot for scrollable/off-screen elements (4x). In
        // i64: a bogus screen size must not overflow the lint itself.
        let screen = i64::from(self.manifest.screen_width.max(self.manifest.screen_height));
        let max_coord = (screen * 4).max(4096);
        // Sorted, so the report is stable across runs.
        let mut objects: Vec<_> = self.layout.objects.iter().collect();
        objects.sort_by_key(|(name, _)| *name);
        for (obj_name, def) in objects {
            if let Some(x) = def.x
                && !(-max_coord..=max_coord).contains(&i64::from(x))
            {
                warnings.push(format!(
                    "layout: '{obj_name}' x={x} outside bounds [-{max_coord}, {max_coord}]"
                ));
            }
            if let Some(y) = def.y
                && !(-max_coord..=max_coord).contains(&i64::from(y))
            {
                warnings.push(format!(
                    "layout: '{obj_name}' y={y} outside bounds [-{max_coord}, {max_coord}]"
                ));
            }
            if let Some(w) = def.w
                && i64::from(w) > max_coord
            {
                warnings.push(format!("layout: '{obj_name}' w={w} exceeds {max_coord}"));
            }
            if let Some(h) = def.h
                && i64::from(h) > max_coord
            {
                warnings.push(format!("layout: '{obj_name}' h={h} exceeds {max_coord}"));
            }
            if def.font_size == Some(0) && def.text.is_some() {
                warnings.push(format!(
                    "layout: '{obj_name}' font_size = 0 renders no text (omit it for the default)"
                ));
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

        // -- Enumerated names (layer kinds, anchors, easing, styles) --
        self.validate_vocabulary(&mut warnings);

        // -- Accessibility: WCAG AA contrast (advisory) --
        // Stylized skins may fail these on purpose; the lint informs
        // authors and never blocks loading.
        for c in self
            .theme
            .validate_contrast_placed(self.features.clock_in_bottombar)
        {
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
        let capacity = u64::from(self.features.grid_cols) * u64::from(self.features.grid_rows);
        if u64::from(self.features.icons_per_page) > capacity {
            warnings.push(format!(
                "features: icons_per_page ({}) exceeds grid capacity ({}x{}={capacity})",
                self.features.icons_per_page, self.features.grid_cols, self.features.grid_rows,
            ));
        }

        warnings
    }
}

#[cfg(test)]
mod tests;

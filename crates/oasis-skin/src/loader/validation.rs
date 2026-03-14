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
        }

        // -- Feature flag checks --
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
}
